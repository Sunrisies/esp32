#![no_std]
#![no_main]
// #![deny(
//     clippy::mem_forget,
//     reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
//     holding buffers for the duration of a data transfer."
// )]
// #![deny(clippy::large_stack_frames)]
use core::net::Ipv4Addr;
use defmt::info;
use embassy_executor::Spawner;
use embassy_futures::select::Either;
use embassy_net::{
    IpListenEndpoint, Ipv4Cidr, Runner, StackResources, StaticConfigV4,
    dns::DnsSocket,
    tcp::{
        TcpSocket,
        client::{TcpClient, TcpClientState},
    },
};
use embassy_time::{Duration, Timer};
use embedded_io_async::Read;
use esp_alloc as _;
use esp_backtrace as _;
use esp_hal::{clock::CpuClock, rng::Rng};
use esp_println as _;
use esp_println::{print, println};
use esp_radio::wifi::{
    Config, ControllerConfig, Interface, WifiController, ap::AccessPointConfig, sta::StationConfig,
};
use esp_rtos as _;
use reqwless::{
    client::HttpClient,
    request::{Method, RequestBuilder},
};
// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

// Wi-Fi 凭证
const WIFI_SSID: &str = "HOVER-2.4G";
const WIFI_PASS: &str = "12345678";
macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

// 网络栈资源池大小
// const STACK_RESOURCES: StackResources<2> = StackResources::new();
// #[allow(
//     clippy::large_stack_frames,
//     reason = "it's not unusual to allocate larger buffers etc. in main"
// )]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    // generator version: 1.2.0
    // 分配内存
    esp_alloc::heap_allocator!(size:72 * 1024);
    // 1. 初始化日志
    info!("Starting Wi-Fi + LED demo");
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let p = esp_hal::init(config);
    // ✅ 先启动 esp_rtos
    let timg0 = esp_hal::timer::timg::TimerGroup::new(p.TIMG0);
    let sw_int = esp_hal::interrupt::software::SoftwareInterruptControl::new(p.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);
    let access_point_station_config = Config::AccessPointStation(
        StationConfig::default()
            .with_ssid(WIFI_SSID)
            .with_password(WIFI_PASS.to_ascii_lowercase()),
        AccessPointConfig::default().with_ssid("esp-radio-apsta"),
    );

    println!("Starting wifi");
    let (controller, interfaces) = esp_radio::wifi::new(
        p.WIFI,
        ControllerConfig::default().with_initial_config(access_point_station_config),
    )
    .unwrap();
    println!("Wifi started!");
    let wifi_ap_device = interfaces.access_point;
    let wifi_sta_device = interfaces.station;

    let ap_config = embassy_net::Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(Ipv4Addr::new(192, 168, 2, 1), 24),
        gateway: Some(Ipv4Addr::new(192, 168, 2, 1)),
        dns_servers: Default::default(),
    });
    let sta_config = embassy_net::Config::dhcpv4(Default::default());

    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    // Init network stacks
    let (ap_stack, ap_runner) = embassy_net::new(
        wifi_ap_device,
        ap_config,
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        seed,
    );
    let (sta_stack, sta_runner) = embassy_net::new(
        wifi_sta_device,
        sta_config,
        mk_static!(StackResources<4>, StackResources::<4>::new()),
        seed,
    );
    spawner.spawn(connection(controller).unwrap());
    spawner.spawn(net_task(ap_runner).unwrap());
    spawner.spawn(net_task(sta_runner).unwrap());
    let sta_address = loop {
        if let Some(config) = sta_stack.config_v4() {
            let address = config.address.address();
            println!("Got IP: {}", address);
            break address;
        }
        println!("Waiting for IP...");
        Timer::after(Duration::from_millis(500)).await;
    };
    ap_stack.wait_config_up().await;

    println!(
        "Connect to the AP `esp-radio-apsta` and point your browser to http://192.168.2.1:8080/"
    );
    println!("Use a static IP in the range 192.168.2.2 .. 192.168.2.255, use gateway 192.168.2.1");
    println!(
        "Or connect to the ap `{WIFI_SSID}` and point your browser to http://{sta_address}:8080/"
    );

    // Init HTTP client
    let tcp_client = TcpClient::new(
        sta_stack,
        mk_static!(
            TcpClientState<1, 1500, 1500>,
            TcpClientState::<1, 1500, 1500>::new()
        ),
    );
    let dns_client = DnsSocket::new(sta_stack);

    let mut ap_server_rx_buffer = [0; 1536];
    let mut ap_server_tx_buffer = [0; 1536];
    let mut sta_server_rx_buffer = [0; 1536];
    let mut sta_server_tx_buffer = [0; 1536];

    let mut ap_server_socket =
        TcpSocket::new(ap_stack, &mut ap_server_rx_buffer, &mut ap_server_tx_buffer);
    ap_server_socket.set_timeout(Some(embassy_time::Duration::from_secs(10)));

    let mut sta_server_socket = TcpSocket::new(
        sta_stack,
        &mut sta_server_rx_buffer,
        &mut sta_server_tx_buffer,
    );
    sta_server_socket.set_timeout(Some(embassy_time::Duration::from_secs(10)));

    loop {
        println!("Wait for connection...");
        // FIXME: If connections are attempted on both sockets at the same time, we
        // might end up dropping one of them. Might be better to spawn both
        // accept() calls, or use fused futures? Note that we only attempt to
        // serve one connection at a time, so we don't run out of ram.
        let either_socket = embassy_futures::select::select(
            ap_server_socket.accept(IpListenEndpoint {
                addr: None,
                port: 8080,
            }),
            sta_server_socket.accept(IpListenEndpoint {
                addr: None,
                port: 8080,
            }),
        )
        .await;
        let (r, server_socket) = match either_socket {
            Either::First(r) => (r, &mut ap_server_socket),
            Either::Second(r) => (r, &mut sta_server_socket),
        };
        println!("Connected...");

        if let Err(e) = r {
            println!("connect error: {:?}", e);
            continue;
        }

        use embedded_io_async::Write;

        let mut buffer = [0u8; 1024];
        let mut pos = 0;
        loop {
            match server_socket.read(&mut buffer).await {
                Ok(0) => {
                    println!("AP read EOF");
                    break;
                }
                Ok(len) => {
                    let to_print =
                        unsafe { core::str::from_utf8_unchecked(&buffer[..(pos + len)]) };

                    if to_print.contains("\r\n\r\n") {
                        print!("{}", to_print);
                        println!();
                        break;
                    }

                    pos += len;
                }
                Err(e) => {
                    println!("AP read error: {:?}", e);
                    break;
                }
            };
        }
        if sta_stack.is_link_up() {
            println!("connecting via HttpClient...");
            let mut client = HttpClient::new(&tcp_client, &dns_client);
            let mut rx_buf = [0u8; 4096];

            let builder_result = client
                .request(Method::GET, "http://httpbin.org/get?hello=Hello+esp-hal")
                .await;

            match builder_result {
                Ok(req_builder) => {
                    let headers = [("Host", "httpbin.org"), ("Connection", "close")];
                    let mut req_builder = req_builder.headers(&headers);
                    let response_result = req_builder.send(&mut rx_buf).await;

                    match response_result {
                        Ok(response) => {
                            println!("HTTP request successful, streaming body...");

                            let _ = server_socket.write_all(b"HTTP/1.0 200 OK\r\n").await;
                            let _ = server_socket
                                .write_all(b"Content-Type: application/json\r\n")
                                .await;
                            let _ = server_socket.write_all(b"Connection: close\r\n\r\n").await;

                            let mut body_reader = response.body().reader();
                            let mut chunk_buf = [0u8; 1024];

                            loop {
                                match body_reader.read(&mut chunk_buf).await {
                                    Ok(0) => break,
                                    Ok(n) => {
                                        if let Err(e) =
                                            server_socket.write_all(&chunk_buf[..n]).await
                                        {
                                            println!("AP write error: {:?}", e);
                                            break;
                                        }
                                    }
                                    Err(e) => {
                                        println!("Body read error: {:?}", e);
                                        break;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            println!("Station request error: {:?}", e);
                            let _ = server_socket
                                .write_all(
                                    b"HTTP/1.0 500 Internal Server Error\r\n\r\nRequest failed",
                                )
                                .await;
                        }
                    }
                }
                Err(e) => {
                    println!("DNS/Connect error: {:?}", e);
                    let _ = server_socket
                        .write_all(b"HTTP/1.0 500 Internal Server Error\r\n\r\nDNS Error")
                        .await;
                }
            }
        } else {
            let r = server_socket
                .write_all(
                    b"HTTP/1.0 200 OK\r\n\r\n\
                    <html>\
                        <body>\
                            <h1>Hello Rust! Hello esp-radio! Station is not connected.</h1>\
                        </body>\
                    </html>\r\n\
                    ",
                )
                .await;
            if let Err(e) = r {
                println!("AP write error: {:?}", e);
            }
        }
        let r = server_socket.flush().await;
        if let Err(e) = r {
            println!("AP flush error: {:?}", e);
        }
        Timer::after(Duration::from_millis(1000)).await;
        server_socket.close();
        Timer::after(Duration::from_millis(1000)).await;
        server_socket.abort();
    }
    // for inspiration have a look at the examples at https://github.com/esp-rs/esp-hal/tree/esp-hal-v1.0.0/examples
}

#[embassy_executor::task]
async fn connection(mut controller: WifiController<'static>) {
    println!("start connection task");

    loop {
        match controller.connect_async().await {
            Ok(_) => {
                // wait until we're no longer connected
                loop {
                    let info = embassy_futures::select::select(
                        controller.wait_for_disconnect_async(),
                        controller.wait_for_access_point_connected_event_async(),
                    )
                    .await;

                    match info {
                        Either::First(station_disconnected) => {
                            if let Ok(station_disconnected) = station_disconnected {
                                println!("Station disconnected: {:?}", station_disconnected);
                                break;
                            }
                        }
                        Either::Second(event) => {
                            if let Ok(event) = event {
                                match event {
                                    esp_radio::wifi::AccessPointStationEventInfo::Connected(
                                        access_point_station_connected_info,
                                    ) => {
                                        println!(
                                            "Station connected: {:?}",
                                            access_point_station_connected_info
                                        );
                                    }
                                    esp_radio::wifi::AccessPointStationEventInfo::Disconnected(
                                        access_point_station_disconnected_info,
                                    ) => {
                                        println!(
                                            "Station disconnected: {:?}",
                                            access_point_station_disconnected_info
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                println!("Failed to connect to wifi: {e:?}");
                Timer::after(Duration::from_millis(5000)).await
            }
        }
    }
}

#[embassy_executor::task(pool_size = 2)]
async fn net_task(mut runner: Runner<'static, Interface<'static>>) {
    runner.run().await
}
