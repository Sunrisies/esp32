#![no_std]
#![no_main]

#[path = "app/http.rs"]
mod http;
#[path = "app/led.rs"]
mod led;
#[path = "app/mqtt.rs"]
mod mqtt;
#[path = "app/wifi.rs"]
mod wifi;

use embassy_executor::Spawner;
use embassy_net::StackResources;
use esp_alloc as _;
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_println as _;
use esp_rtos as _;

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

macro_rules! mk_static {
    ($t:ty, $val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write($val);
        x
    }};
}

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    esp_alloc::heap_allocator!(size: 128 * 1024);

    esp32::log_info!("Starting Wi-Fi + LED demo");

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let p = esp_hal::init(config);

    let rmt = p.RMT;
    let led_pin = p.GPIO8;

    let timg0 = esp_hal::timer::timg::TimerGroup::new(p.TIMG0);
    let sw_int = esp_hal::interrupt::software::SoftwareInterruptControl::new(p.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);

    let (controller, wifi_ap_device, wifi_sta_device) = wifi::start(p.WIFI);
    let (ap_config, sta_config) = wifi::network_configs();
    let seed = wifi::random_seed();

    let (ap_stack, ap_runner) = embassy_net::new(
        wifi_ap_device,
        ap_config,
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        seed,
    );
    let (sta_stack, sta_runner) = embassy_net::new(
        wifi_sta_device,
        sta_config,
        mk_static!(StackResources<8>, StackResources::<8>::new()),
        seed,
    );

    spawner.spawn(wifi::connection(controller).unwrap());
    spawner.spawn(wifi::net_task(ap_runner).unwrap());
    spawner.spawn(wifi::net_task(sta_runner).unwrap());
    spawner.spawn(wifi::dhcp_server_task(ap_stack).unwrap());
    spawner.spawn(led::control_task(rmt, led_pin).unwrap());
    mqtt::spawn_manager(&spawner, sta_stack);

    wifi::wait_for_access_point(ap_stack, sta_stack).await;
    http::serve(ap_stack, sta_stack).await
}
