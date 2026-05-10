#![no_std]
#![no_main]

use embassy_sync::channel::Channel;
use embassy_time::Duration;
use esp_backtrace as _;
use esp_hal::main;

use esp32::myrtio_mqtt::{
    client::LastWill,
    packet::Publish,
    runtime::{
        BufferedOutbox, MqttModule, MqttRuntime, PublishOutbox, PublishRequestChannel,
        TopicCollector, TopicRegistry,
    },
    transport::{MqttTransport, TransportError},
    MqttClient, MqttOptions, QoS,
};

const CLIENT_ID: &str = "const-topics-demo";
const CMD_TOPIC: &str = "device/demo/cmd";
const STATE_TOPIC: &str = "device/demo/state";
const OFFLINE_TOPIC: &str = "device/demo/status/offline";

static PUBLISH_CHANNEL: PublishRequestChannel<'static, 4> = Channel::new();

#[derive(Debug)]
struct DummyError;

impl TransportError for DummyError {}

struct DummyTransport;

impl MqttTransport for DummyTransport {
    type Error = DummyError;

    async fn send(&mut self, _buf: &[u8]) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn recv(&mut self, _buf: &mut [u8]) -> Result<usize, Self::Error> {
        Ok(0)
    }
}

struct ConstTopicsModule {
    pending_state_publish: bool,
}

impl ConstTopicsModule {
    fn new() -> Self {
        Self {
            pending_state_publish: false,
        }
    }
}

impl MqttModule for ConstTopicsModule {
    fn register(&self, collector: &mut dyn TopicCollector) {
        let _ = collector.add(CMD_TOPIC);
    }

    fn on_message(&mut self, msg: &Publish<'_>) {
        if msg.topic == CMD_TOPIC {
            self.pending_state_publish = true;
        }
    }

    fn on_tick(&mut self, outbox: &mut dyn PublishOutbox) -> Duration {
        if self.pending_state_publish {
            outbox.publish_with_retain(
                STATE_TOPIC,
                b"{\"status\":\"updated\"}",
                QoS::AtLeastOnce,
                true,
            );
            self.pending_state_publish = false;
        }
        Duration::from_secs(30)
    }

    fn on_start(&mut self, outbox: &mut dyn PublishOutbox) {
        outbox.publish_with_retain(
            STATE_TOPIC,
            b"{\"status\":\"online\"}",
            QoS::AtLeastOnce,
            true,
        );
    }

    fn last_will(&self) -> Option<LastWill<'_>> {
        Some(LastWill {
            topic: OFFLINE_TOPIC,
            payload: b"{\"status\":\"offline\"}",
            qos: QoS::AtLeastOnce,
            retain: true,
        })
    }

    fn needs_immediate_publish(&self) -> bool {
        self.pending_state_publish
    }
}

#[main]
fn main() -> ! {
    let mut module = ConstTopicsModule::new();

    let mut registry = TopicRegistry::<4>::new();
    module.register(&mut registry);
    let _ = registry.len();

    let mut outbox = BufferedOutbox::<4, 64, 128>::new();
    module.on_start(&mut outbox);
    let _ = outbox.len();

    let transport = DummyTransport;
    let options = MqttOptions::new(CLIENT_ID).with_keep_alive(Duration::from_secs(30));
    let client = MqttClient::<_, 4, 512>::new(transport, options);

    let _runtime = MqttRuntime::new(client, module, PUBLISH_CHANNEL.receiver());

    loop {}
}
