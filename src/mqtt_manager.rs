// src/mqtt_manager.rs
#![allow(dead_code)]
use crate::myrtio_mqtt::{
    self, MqttEvent, MqttOptions, QoS, TcpTransport,
    client::MqttClient,
    packet::Publish,
    runtime::{MqttModule, MqttRuntime, PublishOutbox, PublishRequestChannel, TopicCollector},
};
use heapless::{String, Vec}; // 添加这行

use core::net::{Ipv4Addr, SocketAddrV4};
use embassy_net::{IpEndpoint, Stack, tcp::TcpSocket};
use embassy_sync::{
    blocking_mutex::raw::{CriticalSectionRawMutex, NoopRawMutex},
    pubsub::{PubSubChannel, Publisher, Subscriber},
};
use embassy_time::{Duration, Timer};
use esp_println::println;
// 配置常量
const MQTT_BROKER_IP: Ipv4Addr = Ipv4Addr::new(175, 27, 135, 250); // 示例 IP，请替换为实际 Broker
const MQTT_PORT: u16 = 1883;
const MQTT_CLIENT_ID: &str = "esp32c6-client";
const KEEP_ALIVE_SECS: u64 = 120;

// 定义 MQTT 消息类型
#[derive(Clone, Debug)]
pub struct MqttMessage {
    pub topic: String<64>,     // 假设主题长度不超过 64 字节
    pub payload: Vec<u8, 128>, // 假设 payload 长度不超过 128 字节
}
// 正确顺序：<互斥锁类型, 消息类型, 容量, 最大订阅者数, 最大发布者数>
type MqttPubSub = PubSubChannel<CriticalSectionRawMutex, MqttMessage, 4, 2, 2>;
// 创建全局 PubSub 通道，用于接收来自 MQTT 的订阅消息
pub static MQTT_INCOMING: MqttPubSub = PubSubChannel::new();
/// 启动 MQTT 管理任务
#[embassy_executor::task]
pub async fn mqtt_manager_task(stack: Stack<'static>) -> ! {
    let mut reconnect_delay = Duration::from_secs(5);
    let mut publisher: Publisher<'_, CriticalSectionRawMutex, MqttMessage, 4, 2, 2> =
        MQTT_INCOMING.publisher().unwrap();
    println!("[MQTT] Creating TCP socket...");
    let rx_buffer = mk_static!([u8; 1024], [0; 1024]);
    let tx_buffer = mk_static!([u8; 1024], [0; 1024]);
    loop {
        let mut socket = TcpSocket::new(stack, rx_buffer, tx_buffer);
        socket.set_timeout(Some(Duration::from_secs(15)));
        let broker_addr = SocketAddrV4::new(MQTT_BROKER_IP, MQTT_PORT);
        let endpoint = IpEndpoint::from(broker_addr);

        match socket.connect(endpoint).await {
            Ok(()) => println!("[MQTT] TCP connected"),
            Err(e) => {
                println!(
                    "[MQTT] TCP connect error: {:?}, retrying in {:?}",
                    e, reconnect_delay
                );
                Timer::after(reconnect_delay).await;
                continue;
            }
        }

        let transport = TcpTransport::new(socket, Duration::from_secs(5));
        let options =
            MqttOptions::new(MQTT_CLIENT_ID).with_keep_alive(Duration::from_secs(KEEP_ALIVE_SECS));

        let mut client = MqttClient::<_, 8, 1024>::new(transport, options);

        if let Err(e) = client.connect().await {
            println!("[MQTT] MQTT connect error: {:?}", e);
            Timer::after(reconnect_delay).await;
            continue;
        }
        println!("[MQTT] MQTT connected!");

        // 订阅你需要的所有主题（可在此处动态添加）
        let topics = ["device/cmd", "device/ota", "device/config"];
        for topic in topics {
            if let Err(e) = client.subscribe(topic, QoS::AtMostOnce).await {
                println!("[MQTT] Subscribe to '{}' error: {:?}", topic, e);
            } else {
                println!("[MQTT] Subscribed to '{}'", topic);
            }
        }

        // 用于定时发布的心跳计数器
        let mut last_publish = embassy_time::Instant::now();
        // loop {
        match client.poll().await {
            Ok(Some(MqttEvent::Publish(publish))) => {
                // 直接访问字段 topic
                let topic_str = publish.topic;

                // 创建新的消息结构体并复制数据
                let mut msg = MqttMessage {
                    topic: String::new(),
                    payload: Vec::new(),
                };

                if let Ok(_) = msg.payload.extend_from_slice(publish.payload) {
                    let msg_clone = msg.clone();
                    // 只有复制成功才发布
                    publisher.publish(msg).await;
                    println!("[MQTT] Received on {:?} ", msg_clone);
                }

                // let _ = publish.ack().await;
            }
            // Ok(Some(MqttEvent::Disconnected)) => {
            //     println!("[MQTT] Disconnected, will reconnect...");
            //     break;
            // }
            Ok(None) => {
                // 没有事件，继续循环
            }
            Err(e) => {
                println!("[MQTT] Poll error: {:?}", e);
                // drop(client); // 显式丢弃 client，释放 socket
                // Timer::after_millis(100).await; // 等待底层资源回收
                // break;
            }
            _ => {} // 忽略其他不关心的事件
        }

        // 定时发布逻辑...
        // if last_publish.elapsed() >= Duration::from_secs(30) {
        let _ = client
            .publish("device/state", b"{\"temp\": 22.5}", QoS::AtMostOnce)
            .await;
        // last_publish = embassy_time::Instant::now();
        // }

        Timer::after_millis(50).await;
        // }
    }
}

// 静态内存分配宏（与 main.rs 中保持一致）
macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}
use mk_static;
