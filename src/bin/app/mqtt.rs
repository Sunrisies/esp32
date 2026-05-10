use embassy_executor::Spawner;
use embassy_net::Stack;

use esp32::mqtt_manager::mqtt_manager_task;

pub fn spawn_manager(spawner: &Spawner, sta_stack: Stack<'static>) {
    spawner.spawn(mqtt_manager_task(sta_stack).unwrap());
}
