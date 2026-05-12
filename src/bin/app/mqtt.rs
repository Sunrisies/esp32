use embassy_executor::Spawner;
use embassy_net::Stack;
use embassy_time::{Duration, Timer};

use esp32::mqtt_manager::run_mqtt_manager;

pub fn spawn_manager(spawner: &Spawner, sta_stack: Stack<'static>) {
    spawner.spawn(delayed_mqtt_manager_task(sta_stack).unwrap());
}

#[embassy_executor::task]
async fn delayed_mqtt_manager_task(sta_stack: Stack<'static>) -> ! {
    loop {
        if let Some(config) = sta_stack.config_v4() {
            esp32::log_info!(
                "STA network ready at {}; starting MQTT",
                config.address.address()
            );
            break;
        }

        Timer::after(Duration::from_millis(500)).await;
    }

    run_mqtt_manager(sta_stack).await
}
