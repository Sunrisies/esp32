use embassy_net::{Ipv4Cidr, Runner, Stack, StaticConfigV4};
use embassy_time::{Duration, Timer};
use esp_hal::rng::Rng;
use esp_println::println;
use esp_radio::wifi::{
    AccessPointStationEventInfo, Config, ControllerConfig, Interface, WifiController,
    ap::AccessPointConfig, sta::StationConfig,
};
use esp32::config::{WIFI_AP_IP, WIFI_AP_SSID, WIFI_PASSWORD, WIFI_SSID};

pub fn start(
    wifi: esp_hal::peripherals::WIFI<'static>,
) -> (WifiController<'static>, Interface<'static>, Interface<'static>) {
    let access_point_station_config = Config::AccessPointStation(
        StationConfig::default()
            .with_ssid(WIFI_SSID)
            .with_password(WIFI_PASSWORD.to_ascii_lowercase()),
        AccessPointConfig::default().with_ssid(WIFI_AP_SSID),
    );

    println!("Starting wifi");
    let (controller, interfaces) = esp_radio::wifi::new(
        wifi,
        ControllerConfig::default().with_initial_config(access_point_station_config),
    )
    .unwrap();
    println!("Wifi started!");

    (controller, interfaces.access_point, interfaces.station)
}

pub fn network_configs() -> (embassy_net::Config, embassy_net::Config) {
    let ap_config = embassy_net::Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(WIFI_AP_IP, 24),
        gateway: Some(WIFI_AP_IP),
        dns_servers: Default::default(),
    });
    let sta_config = embassy_net::Config::dhcpv4(Default::default());

    (ap_config, sta_config)
}

pub fn random_seed() -> u64 {
    let rng = Rng::new();
    (rng.random() as u64) << 32 | rng.random() as u64
}

pub async fn wait_for_networks(ap_stack: Stack<'static>, sta_stack: Stack<'static>) {
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

    println!("Connect to the AP `{WIFI_AP_SSID}` and point your browser to http://{WIFI_AP_IP}:8080/");
    println!(
        "Use a static IP in the range 192.168.2.2 .. 192.168.2.255, use gateway {WIFI_AP_IP}"
    );
    println!("Or connect to the ap `{WIFI_SSID}` and point your browser to http://{sta_address}:8080/");
}

#[embassy_executor::task]
pub async fn connection(mut controller: WifiController<'static>) {
    println!("start connection task");

    loop {
        match controller.connect_async().await {
            Ok(_) => {
                println!("WiFi已连接，启动RSSI监视器.");
                let mut ticker = embassy_time::Ticker::every(Duration::from_secs(10));

                loop {
                    let either = embassy_futures::select::select3(
                        controller.wait_for_disconnect_async(),
                        controller.wait_for_access_point_connected_event_async(),
                        ticker.next(),
                    )
                    .await;

                    match either {
                        embassy_futures::select::Either3::Third(_) => match controller.rssi() {
                            Ok(rssi) => println!("Current WiFi RSSI: {} dBm", rssi),
                            Err(e) => println!("Failed to get RSSI: {:?}", e),
                        },
                        embassy_futures::select::Either3::First(station_disconnected) => {
                            if let Ok(info) = station_disconnected {
                                println!("Station disconnected: {:?}", info);
                                break;
                            }
                        }
                        embassy_futures::select::Either3::Second(event) => {
                            if let Ok(event) = event {
                                match event {
                                    AccessPointStationEventInfo::Connected(info) => {
                                        println!("Station connected: {:?}", info);
                                    }
                                    AccessPointStationEventInfo::Disconnected(info) => {
                                        println!("Station disconnected: {:?}", info);
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
pub async fn net_task(mut runner: Runner<'static, Interface<'static>>) {
    runner.run().await
}
