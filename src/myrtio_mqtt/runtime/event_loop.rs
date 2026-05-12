//! MQTT Runtime - drives modules and handles the event loop.

use embassy_futures::select::{Either3, select3};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Receiver;
use embassy_time::{Duration, Instant, Timer};

use super::publisher::{BufferedOutbox, PublishRequest};
use super::registry::TopicRegistry;
use super::traits::MqttModule;
use crate::myrtio_mqtt::client::MqttClient;
use crate::myrtio_mqtt::error::MqttError;
use crate::myrtio_mqtt::transport::{MqttTransport, TransportError};
use crate::myrtio_mqtt::{MqttEvent, QoS};

/// The MQTT runtime that drives modules and handles the event loop.
///
/// The runtime owns the `MqttClient` and multiplexes between:
/// - Incoming MQTT messages (dispatched to modules)
/// - Outgoing publish requests from controllers (via channel)
/// - Periodic ticks for module housekeeping
///
/// # Object-Safe Module Support
///
/// The runtime is generic over the module type `M`, which must implement
/// `MqttModule`. Because `MqttModule` is object-safe, you can use:
///
/// - Concrete module types for maximum performance
/// - `&mut dyn MqttModule` for trait objects (no `Box` needed)
///
/// # Topic Registration
///
/// During startup, the runtime calls `module.register()` with a `TopicCollector`
/// to collect all topics the module wants to subscribe to. Topics are copied
/// into the registry, so they only need to live for the duration of the call.
///
/// # Publishing Pattern
///
/// Modules use a `BufferedOutbox` to queue publish requests during `on_tick`
/// and `on_start`. The runtime then drains the outbox and performs the actual
/// async publishing.
pub struct MqttRuntime<
    'a,
    T,
    M,
    const MAX_TOPICS: usize,
    const BUF_SIZE: usize,
    const OUTBOX_DEPTH: usize,
> where
    T: MqttTransport,
    M: MqttModule,
{
    client: MqttClient<'a, T, MAX_TOPICS, BUF_SIZE>,
    module: M,
    publisher_rx: Receiver<'a, CriticalSectionRawMutex, PublishRequest<'a>, OUTBOX_DEPTH>,
}

/// Constants for the internal publish outbox used during module callbacks.
const OUTBOX_CAPACITY: usize = 8;
const OUTBOX_TOPIC_SIZE: usize = 128;
const OUTBOX_PAYLOAD_SIZE: usize = 1024;

impl<'a, T, M, const MAX_TOPICS: usize, const BUF_SIZE: usize, const OUTBOX_DEPTH: usize>
    MqttRuntime<'a, T, M, MAX_TOPICS, BUF_SIZE, OUTBOX_DEPTH>
where
    T: MqttTransport,
    T::Error: TransportError,
    M: MqttModule,
{
    /// Create a new MQTT runtime.
    ///
    /// # Arguments
    ///
    /// - `client`: The MQTT client (not yet connected)
    /// - `module`: The module (or composed modules) to drive
    /// - `publisher_rx`: Receiver end of the publish request channel
    pub fn new(
        client: MqttClient<'a, T, MAX_TOPICS, BUF_SIZE>,
        module: M,
        publisher_rx: Receiver<'a, CriticalSectionRawMutex, PublishRequest<'a>, OUTBOX_DEPTH>,
    ) -> Self {
        Self {
            client,
            module,
            publisher_rx,
        }
    }

    /// Run the MQTT runtime event loop.
    ///
    /// This method:
    /// 1. Connects to the MQTT broker
    /// 2. Subscribes to all topics registered by the module
    /// 3. Calls `on_start` for initial setup
    /// 4. Enters the main loop handling messages, publishes, and ticks
    ///
    /// This method runs forever unless an error occurs.
    pub async fn run(&mut self) -> Result<(), MqttError<T::Error>> {
        if let Some(last_will) = self.module.last_will()
            && !self.client.set_last_will(last_will)
        {
            return Err(MqttError::BufferTooSmall);
        }

        self.client.connect().await?;

        let mut registry = TopicRegistry::<MAX_TOPICS>::new();
        self.module.register(&mut registry);
        for topic in registry.iter() {
            self.client.subscribe(topic, QoS::AtMostOnce).await?;
        }

        let mut outbox: BufferedOutbox<OUTBOX_CAPACITY, OUTBOX_TOPIC_SIZE, OUTBOX_PAYLOAD_SIZE> =
            BufferedOutbox::new();

        self.module.on_start(&mut outbox);
        self.drain_outbox(&mut outbox).await?;

        let tick_interval = self.module.on_tick(&mut outbox);
        self.drain_outbox(&mut outbox).await?;
        let mut tick_deadline = Instant::now() + tick_interval;

        loop {
            while let Ok(req) = self.publisher_rx.try_receive() {
                self.publish_request(req).await?;
            }

            let remaining = Self::remaining_until(tick_deadline);
            let poll_fut = self.client.poll();
            let publish_fut = self.publisher_rx.receive();
            let timer_fut = Timer::after(remaining);

            match select3(poll_fut, publish_fut, timer_fut).await {
                Either3::First(result) => {
                    let needs_publish = match result? {
                        Some(MqttEvent::Publish(msg)) => {
                            self.module.on_message(&msg);
                            self.module.needs_immediate_publish()
                        }
                        None => false,
                    };

                    if needs_publish {
                        self.module.on_publish(&mut outbox);
                        self.drain_outbox(&mut outbox).await?;
                    }
                }
                Either3::Second(req) => {
                    self.publish_request(req).await?;
                }
                Either3::Third(()) => {
                    let interval = self.module.on_tick(&mut outbox);
                    self.drain_outbox(&mut outbox).await?;
                    tick_deadline = Instant::now() + interval;
                }
            }
        }
    }

    async fn publish_request(
        &mut self,
        req: PublishRequest<'_>,
    ) -> Result<(), MqttError<T::Error>> {
        self.client
            .publish_with_retain(req.topic, req.payload, req.qos, req.retain)
            .await
    }

    fn remaining_until(deadline: Instant) -> Duration {
        let now = Instant::now();
        if now >= deadline {
            Duration::from_millis(0)
        } else {
            deadline - now
        }
    }

    /// 清空发件箱并发布所有缓冲邮件。
    async fn drain_outbox(
        &mut self,
        outbox: &mut BufferedOutbox<OUTBOX_CAPACITY, OUTBOX_TOPIC_SIZE, OUTBOX_PAYLOAD_SIZE>,
    ) -> Result<(), MqttError<T::Error>> {
        for req in outbox.drain() {
            self.client
                .publish_with_retain(
                    req.topic.as_str(),
                    req.payload.as_slice(),
                    req.qos,
                    req.retain,
                )
                .await?;
        }
        outbox.clear();
        Ok(())
    }

    /// Get a reference to the underlying module.
    pub fn module(&self) -> &M {
        &self.module
    }

    /// Get a mutable reference to the underlying module.
    pub fn module_mut(&mut self) -> &mut M {
        &mut self.module
    }
}
