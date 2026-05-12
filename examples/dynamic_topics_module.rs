#![no_std]
#![no_main]

use embassy_sync::channel::Channel;
use embassy_time::Duration;
use esp_backtrace as _;
use esp_hal::main;
use heapless::String;

use esp32::myrtio_mqtt::{
    MqttClient, MqttOptions, QoS,
    client::LastWill,
    packet::Publish,
    runtime::{
        BufferedOutbox, MqttModule, MqttRuntime, PublishOutbox, PublishRequestChannel,
        TopicCollector, TopicRegistry,
    },
    transport::{MqttTransport, TransportError},
};

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

struct DynamicTopicsModule {
    command_topic: String<64>,
    state_topic: String<64>,
    offline_topic: String<64>,
    pending_state_publish: bool,
}

impl DynamicTopicsModule {
    fn new(device_id: &str) -> Self {
        Self {
            command_topic: Self::build_topic("device/", device_id, "/cmd"),
            state_topic: Self::build_topic("device/", device_id, "/state"),
            offline_topic: Self::build_topic("device/", device_id, "/status/offline"),
            pending_state_publish: false,
        }
    }

    fn build_topic(prefix: &str, device_id: &str, suffix: &str) -> String<64> {
        let mut topic = String::new();
        let _ = topic.push_str(prefix);
        let _ = topic.push_str(device_id);
        let _ = topic.push_str(suffix);
        topic
    }

    fn publish_state(&mut self, outbox: &mut dyn PublishOutbox) {
        if self.pending_state_publish {
            outbox.publish_with_retain(
                self.state_topic.as_str(),
                b"{\"status\":\"updated\"}",
                QoS::AtLeastOnce,
                true,
            );
            self.pending_state_publish = false;
        }
    }
}

impl MqttModule for DynamicTopicsModule {
    fn register(&self, collector: &mut dyn TopicCollector) {
        let _ = collector.add(self.command_topic.as_str());
    }

    fn on_message(&mut self, msg: &Publish<'_>) {
        if msg.topic == self.command_topic.as_str() {
            self.pending_state_publish = true;
        }
    }

    fn on_tick(&mut self, outbox: &mut dyn PublishOutbox) -> Duration {
        self.publish_state(outbox);
        Duration::from_secs(15)
    }

    fn on_start(&mut self, outbox: &mut dyn PublishOutbox) {
        outbox.publish_with_retain(
            self.state_topic.as_str(),
            b"{\"status\":\"online\"}",
            QoS::AtLeastOnce,
            true,
        );
    }

    fn last_will(&self) -> Option<LastWill<'_>> {
        Some(LastWill {
            topic: self.offline_topic.as_str(),
            payload: b"{\"status\":\"offline\"}",
            qos: QoS::AtLeastOnce,
            retain: true,
        })
    }

    fn needs_immediate_publish(&self) -> bool {
        self.pending_state_publish
    }

    fn on_publish(&mut self, outbox: &mut dyn PublishOutbox) {
        self.publish_state(outbox);
    }
}

#[main]
fn main() -> ! {
    let mut module = DynamicTopicsModule::new("node-01");

    let mut registry = TopicRegistry::<4>::new();
    module.register(&mut registry);
    let _ = registry.len();

    let mut outbox = BufferedOutbox::<4, 64, 128>::new();
    module.on_start(&mut outbox);
    let _ = outbox.len();

    let transport = DummyTransport;
    let options = MqttOptions::new("dynamic-topics-demo").with_keep_alive(Duration::from_secs(30));
    let client = MqttClient::<_, 4, 512>::new(transport, options);

    let _runtime = MqttRuntime::new(client, module, PUBLISH_CHANNEL.receiver());

    loop {}
}
