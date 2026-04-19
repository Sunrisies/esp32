#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use esp_hal::{
    clock::CpuClock,
    main,
    rmt::Rmt,
    time::{Duration, Instant, Rate},
};

use defmt::info;
use esp_backtrace as _;
use esp_hal_smartled::{SmartLedsAdapter, smart_led_buffer};
use esp_println as _;
use smart_leds::{RGB8, SmartLedsWrite as _};

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[main]
fn main() -> ! {
    // generator version: 1.2.0

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let p = esp_hal::init(config);
    let rmt = Rmt::new(p.RMT, Rate::from_mhz(80)).unwrap();
    let mut buffer = smart_led_buffer!(1);
    let mut led = SmartLedsAdapter::new(rmt.channel0, p.GPIO8, &mut buffer);
    const LEVEL: u8 = 10;
    let mut color = RGB8::default();
    color.r = LEVEL;
    loop {
        info!("Hello world!");
        led.write([color].into_iter()).unwrap();
        let delay_start = Instant::now();
        while delay_start.elapsed() < Duration::from_millis(1000) {}
        let tmp = color.r;
        color.r = color.b;
        color.b = color.g;
        color.g = tmp;
    }

    // for inspiration have a look at the examples at https://github.com/esp-rs/esp-hal/tree/esp-hal-v1.0.0/examples
}
