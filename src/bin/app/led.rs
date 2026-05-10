use defmt::info;
use embassy_futures::select::{Either, select};
use embassy_time::{Duration, Instant, Timer};
use esp_hal::{rmt::Rmt, time::Rate};
use esp32::{mqtt_manager::COLOR_CHANNEL, smart_led_buffer, ws2812::SmartLedsAdapter};
use smart_leds::{RGB8, SmartLedsWrite as _};

#[embassy_executor::task]
pub async fn control_task(
    rmt_peripheral: esp_hal::peripherals::RMT<'static>,
    pin: esp_hal::peripherals::GPIO8<'static>,
) {
    let rmt = Rmt::new(rmt_peripheral, Rate::from_mhz(40)).unwrap();
    let mut buffer = smart_led_buffer!(1);
    let mut led = SmartLedsAdapter::new(rmt.channel0, pin, &mut buffer);
    let mut current_color = RGB8::default();
    led.write([current_color].into_iter()).unwrap();

    let colors = [
        RGB8 { r: 50, g: 0, b: 0 },
        RGB8 { r: 0, g: 50, b: 0 },
        RGB8 { r: 0, g: 0, b: 50 },
    ];
    let mut auto_idx = 0;
    let mut command_until = Instant::now();
    let check_interval = Duration::from_millis(500);

    loop {
        let timer_fut = Timer::after(check_interval);
        let recv_fut = COLOR_CHANNEL.receive();

        match select(recv_fut, timer_fut).await {
            Either::First(cmd_str) => {
                if let Some(rgb) = parse_color_str(&cmd_str) {
                    current_color = rgb;
                    let _ = led.write([current_color].into_iter());
                    info!(
                        "LED color changed to R: {}, G: {}, B: {}",
                        current_color.r, current_color.g, current_color.b
                    );
                    command_until = Instant::now() + Duration::from_secs(3);
                }
            }
            Either::Second(()) => {
                if Instant::now() >= command_until {
                    current_color = colors[auto_idx % 3];
                    let _ = led.write([current_color].into_iter());
                    info!(
                        "LED color changed to R: {}, G: {}, B: {}",
                        current_color.r, current_color.g, current_color.b
                    );
                    auto_idx += 1;
                }
            }
        }
    }
}

fn parse_color_str(s: &str) -> Option<RGB8> {
    let parts: heapless::Vec<_, 3> = s.split(',').collect();
    if parts.len() == 3 {
        let r = parts[0].parse().ok()?;
        let g = parts[1].parse().ok()?;
        let b = parts[2].parse().ok()?;
        Some(RGB8 { r, g, b })
    } else {
        None
    }
}
