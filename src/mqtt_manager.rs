// // src/mqtt_manager.rs
// #![allow(dead_code)]
// use crate::myrtio_mqtt::{
//     self, LastWill, MqttEvent, MqttOptions, QoS, TcpTransport,
//     client::MqttClient,
//     packet::Publish,
//     runtime::{MqttModule, MqttRuntime, PublishOutbox, PublishRequestChannel, TopicCollector},
// };
// use core::str;

// use defmt::info;
// use heapless::{String, Vec}; // 添加这行

// use core::net::{Ipv4Addr, SocketAddrV4};
// use embassy_net::{IpEndpoint, Stack, tcp::TcpSocket};
// use embassy_sync::{
//     blocking_mutex::raw::{CriticalSectionRawMutex, NoopRawMutex},
//     pubsub::{PubSubChannel, Publisher, Subscriber},
// };
// use embassy_time::{Duration, Timer};
// use esp_println::println;
// // 配置常量
// const MQTT_BROKER_IP: Ipv4Addr = Ipv4Addr::new(175, 27, 135, 250); // 示例 IP，请替换为实际 Broker
// const MQTT_PORT: u16 = 1883;
// const MQTT_CLIENT_ID: &str = "esp32c6-client";
// const KEEP_ALIVE_SECS: u64 = 120;

// // 定义 MQTT 消息类型
// #[derive(Clone, Debug)]
// pub struct MqttMessage {
//     pub topic: String<64>,     // 假设主题长度不超过 64 字节
//     pub payload: Vec<u8, 128>, // 假设 payload 长度不超过 128 字节
// }
// // 正确顺序：<互斥锁类型, 消息类型, 容量, 最大订阅者数, 最大发布者数>
// type MqttPubSub = PubSubChannel<CriticalSectionRawMutex, MqttMessage, 4, 2, 2>;
// // 创建全局 PubSub 通道，用于接收来自 MQTT 的订阅消息
// pub static MQTT_INCOMING: MqttPubSub = PubSubChannel::new();
// /// 启动 MQTT 管理任务
// #[embassy_executor::task]
// pub async fn mqtt_manager_task(stack: Stack<'static>) -> ! {
//     let mut reconnect_delay = Duration::from_secs(5);
//     let mut publisher: Publisher<'_, CriticalSectionRawMutex, MqttMessage, 4, 2, 2> =
//         MQTT_INCOMING.publisher().unwrap();
//     println!("[MQTT] Creating TCP socket...");
//     let rx_buffer = mk_static!([u8; 1024], [0; 1024]);
//     let tx_buffer = mk_static!([u8; 1024], [0; 1024]);
//     // loop {
//     let mut socket = TcpSocket::new(stack, rx_buffer, tx_buffer);
//     socket.set_timeout(Some(Duration::from_secs(15)));
//     let broker_addr = SocketAddrV4::new(MQTT_BROKER_IP, MQTT_PORT);
//     let endpoint = IpEndpoint::from(broker_addr);

//     match socket.connect(endpoint).await {
//         Ok(()) => println!("[MQTT] TCP connected"),
//         Err(e) => {
//             println!(
//                 "[MQTT] TCP connect error: {:?}, retrying in {:?}",
//                 e, reconnect_delay
//             );
//             Timer::after(reconnect_delay).await;
//             // continue;
//         }
//     }

//     let transport = TcpTransport::new(socket, Duration::from_secs(15));
//     // 设置遗言
//     let will = LastWill {
//         topic: OFFLINE_TOPIC,
//         payload: b"{\"status\":\"offline\",\"last_seen\":\"2024-01-01T12:00:00Z\"}",
//         qos: QoS::AtLeastOnce,
//         retain: true,
//     };
//     // KEEP_ALIVE_SECS
//     let options = MqttOptions::new(MQTT_CLIENT_ID)
//         .with_keep_alive(Duration::from_secs(10))
//         .with_last_will(will); // ← 设置遗言

//     let mut client = MqttClient::<_, 8, 1024>::new(transport, options);

//     if let Err(e) = client.connect().await {
//         println!("[MQTT] MQTT connect error: {:?}", e);
//         Timer::after(reconnect_delay).await;
//         // continue;
//     }
//     println!("[MQTT] MQTT connected!");

//     // 订阅你需要的所有主题（可在此处动态添加）
//     // let topics = ["device/cmd", "device/ota", "device/config"];
//     let topics = ["device/cmd"];

//     for topic in topics {
//         if let Err(e) = client.subscribe(topic, QoS::AtMostOnce).await {
//             println!("[MQTT] Subscribe to '{}' error: {:?}", topic, e);
//         } else {
//             println!("[MQTT] Subscribed to '{}'", topic);
//         }
//     }
//     const OFFLINE_TOPIC: &str = "device/status/offline";
//     loop {
//         match client.poll().await {
//             Ok(Some(MqttEvent::Publish(publish))) => {
//                 // 直接访问字段 topic
//                 let topic_str = publish.topic;
//                 info!("topic_str:{}", topic_str);
//                 info!("payload:{:?}", publish.payload);
//                 if let Ok(payload_str) = str::from_utf8(&publish.payload) {
//                     println!("[MQTT] Received payload: {}", payload_str);
//                 } else {
//                     println!("[MQTT] Received non-UTF8 payload: {:?}", publish.payload);
//                 }
//                 // 创建新的消息结构体并复制数据
//                 let mut msg = MqttMessage {
//                     topic: String::new(),
//                     payload: Vec::new(),
//                 };

//                 if let Ok(_) = msg.payload.extend_from_slice(publish.payload) {
//                     let msg_clone = msg.clone();
//                     // 只有复制成功才发布
//                     // publisher.publish(msg).await;
//                     if let Ok(payload_str) = str::from_utf8(&msg_clone.payload) {
//                         println!("[MQTT] Received payload: {}", payload_str);
//                     } else {
//                         println!("[MQTT] Received non-UTF8 payload: {:?}", msg_clone.payload);
//                     }
//                     let _ = client
//                         .publish("device/state", msg_clone.payload.as_ref(), QoS::AtMostOnce)
//                         .await;
//                 } else {
//                     println!("[MQTT] Received payload: {:?}", msg.payload);
//                 }

//                 // let _ = publish.ack().await;
//             }
//             // Ok(Some(MqttEvent::Disconnected)) => {
//             //     println!("[MQTT] Disconnected, will reconnect...");
//             //     break;
//             // }
//             Ok(None) => {
//                 // 没有事件，继续循环
//             }
//             Err(e) => {
//                 println!("[MQTT] Poll error: {:?}", e);
//                 // drop(client); // 显式丢弃 client，释放 socket
//                 // Timer::after_millis(100).await; // 等待底层资源回收
//                 // break;
//             }
//             _ => {} // 忽略其他不关心的事件
//         }

//         // 定时发布逻辑...
//         // if last_publish.elapsed() >= Duration::from_secs(30) {
//         // let _ = client
//         //     .publish("device/state", b"{\"temp\": 22.5}", QoS::AtMostOnce)
//         //     .await;
//         // last_publish = embassy_time::Instant::now();
//     }

//     // Timer::after_millis(50).await;
//     // }
//     // }
// }

// // 静态内存分配宏（与 main.rs 中保持一致）
// macro_rules! mk_static {
//     ($t:ty,$val:expr) => {{
//         static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
//         #[deny(unused_attributes)]
//         let x = STATIC_CELL.uninit().write(($val));
//         x
//     }};
// }
// use mk_static;

// src/mqtt_manager.rs
#![allow(dead_code)]
use crate::myrtio_mqtt::{
    self, LastWill, MqttEvent, MqttOptions, QoS, TcpTransport, client::MqttClient, packet::Publish,
};
use core::str;

use defmt::info;
use heapless::{String, Vec};

use core::net::{Ipv4Addr, SocketAddrV4};
use embassy_net::{IpEndpoint, Stack, tcp::TcpSocket};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::pubsub::PubSubChannel;
use embassy_time::{Duration, Timer};
use esp_println::println;

// 配置常量
const MQTT_BROKER_IP: Ipv4Addr = Ipv4Addr::new(175, 27, 135, 250);
const MQTT_PORT: u16 = 1883;
const MQTT_CLIENT_ID: &str = "esp32c6-client";

// ===== 用静态常量定义所有主题 =====
const TOPIC_CMD: &str = "device/cmd";
const TOPIC_CONFIG: &str = "device/config";
const TOPIC_OTA: &str = "device/ota";
const TOPIC_STATE: &str = "device/state";
const TOPIC_OFFLINE: &str = "device/status/offline";

// 订阅的主题列表
const SUB_TOPICS: &[&str] = &[TOPIC_CMD, TOPIC_CONFIG, TOPIC_OTA];

// MQTT 消息类型
#[derive(Clone, Debug)]
pub struct MqttMessage {
    pub topic: String<64>,
    pub payload: Vec<u8, 128>,
}

type MqttPubSub = PubSubChannel<CriticalSectionRawMutex, MqttMessage, 4, 2, 2>;
pub static MQTT_INCOMING: MqttPubSub = PubSubChannel::new();

// ===== 消息处理器 trait =====
trait MessageHandler {
    fn can_handle(&self, topic: &str) -> bool;
    fn handle(&self, topic: &str, payload: &[u8]) -> Option<&'static str>; // 返回要回复的状态
}

// ===== 具体处理器实现 =====
struct CmdHandler;
impl MessageHandler for CmdHandler {
    fn can_handle(&self, topic: &str) -> bool {
        topic == TOPIC_CMD
    }

    fn handle(&self, _topic: &str, payload: &[u8]) -> Option<&'static str> {
        if let Ok(cmd) = str::from_utf8(payload) {
            match cmd.trim() {
                "ON" => {
                    println!("[CMD] Turning device ON");
                    Some("{\"status\":\"on\"}")
                }
                "OFF" => {
                    println!("[CMD] Turning device OFF");
                    Some("{\"status\":\"off\"}")
                }
                "REBOOT" => {
                    println!("[CMD] Rebooting...");
                    Some("{\"status\":\"rebooting\"}")
                }
                _ => {
                    println!("[CMD] Unknown command: {}", cmd);
                    Some("{\"status\":\"unknown\"}")
                }
            }
        } else {
            None
        }
    }
}

struct ConfigHandler;
impl MessageHandler for ConfigHandler {
    fn can_handle(&self, topic: &str) -> bool {
        topic == TOPIC_CONFIG
    }

    fn handle(&self, topic: &str, payload: &[u8]) -> Option<&'static str> {
        println!("[CONFIG] Received config: {:?}", payload);
        // 解析 JSON 配置，更新设备参数...
        Some("{\"status\":\"configured\"}")
    }
}

struct OtaHandler;
impl MessageHandler for OtaHandler {
    fn can_handle(&self, topic: &str) -> bool {
        topic == TOPIC_OTA
    }

    fn handle(&self, topic: &str, payload: &[u8]) -> Option<&'static str> {
        println!("[OTA] Update request: {:?}", payload);
        // 启动 OTA 更新...
        Some("{\"status\":\"updating\"}")
    }
}

// ===== 处理器管理器 =====
struct HandlerManager {
    handlers: [&'static dyn MessageHandler; 3], // 静态分发，无运行时开销
}

impl HandlerManager {
    const fn new() -> Self {
        Self {
            handlers: [&CmdHandler, &ConfigHandler, &OtaHandler],
        }
    }

    fn process(&self, publish: &Publish<'_>) -> Option<&'static str> {
        for handler in self.handlers {
            if handler.can_handle(publish.topic) {
                return handler.handle(publish.topic, publish.payload);
            }
        }
        println!("[MQTT] Unknown topic: {}", publish.topic);
        None
    }
}

// ===== 主任务 =====
#[embassy_executor::task]
pub async fn mqtt_manager_task(stack: Stack<'static>) -> ! {
    let mut reconnect_delay = Duration::from_secs(5);
    let handler_manager = HandlerManager::new(); // 静态初始化
    // ===== 静态分配移到循环外，只初始化一次 =====
    static mut RX_BUFFER: [u8; 1024] = [0; 1024];
    static mut TX_BUFFER: [u8; 1024] = [0; 1024];
    // 外层循环：重连管理
    loop {
        println!("[MQTT] Connecting to broker...");
        // 使用 &raw mut 创建原始指针
        let rx_buffer = unsafe { &mut *(&raw mut RX_BUFFER) };
        let tx_buffer = unsafe { &mut *(&raw mut TX_BUFFER) };
        let mut socket = TcpSocket::new(stack, rx_buffer, tx_buffer);
        socket.set_timeout(Some(Duration::from_secs(20)));

        let endpoint = IpEndpoint::from(SocketAddrV4::new(MQTT_BROKER_IP, MQTT_PORT));

        // TCP 连接
        if let Err(e) = socket.connect(endpoint).await {
            println!(
                "[MQTT] TCP connect error: {:?}, retrying in {:?}",
                e, reconnect_delay
            );
            Timer::after(reconnect_delay).await;
            continue;
        }
        println!("[MQTT] TCP connected");

        // 创建 transport 和 client
        let transport = TcpTransport::new(socket, Duration::from_secs(10));

        let will = LastWill {
            topic: TOPIC_OFFLINE,
            payload: b"{\"status\":\"offline\"}",
            qos: QoS::AtLeastOnce,
            retain: true,
        };

        let options = MqttOptions::new(MQTT_CLIENT_ID)
            .with_keep_alive(Duration::from_secs(5))
            .with_last_will(will);

        let mut client = MqttClient::<_, 8, 1024>::new(transport, options);

        // MQTT 连接
        if let Err(e) = client.connect().await {
            println!("[MQTT] MQTT connect error: {:?}", e);
            Timer::after(reconnect_delay).await;
            continue;
        }
        println!("[MQTT] MQTT connected!");

        // 订阅所有主题
        for topic in SUB_TOPICS {
            if let Err(e) = client.subscribe(topic, QoS::AtMostOnce).await {
                println!("[MQTT] Subscribe to '{}' error: {:?}", topic, e);
            } else {
                println!("[MQTT] Subscribed to '{}'", topic);
            }
        }

        // 上线状态
        let _ = client
            .publish(TOPIC_STATE, b"{\"status\":\"online\"}", QoS::AtLeastOnce)
            .await;

        // 内层循环：运行逻辑
        loop {
            match client.poll().await {
                Ok(Some(MqttEvent::Publish(publish))) => {
                    info!(
                        "Received: topic={}, payload={:?}",
                        publish.topic, publish.payload
                    );

                    // 使用处理器管理器处理消息
                    if let Some(response) = handler_manager.process(&publish) {
                        // 回复状态
                        let _ = client
                            .publish(TOPIC_STATE, response.as_bytes(), QoS::AtLeastOnce)
                            .await;
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    println!("[MQTT] Poll error: {:?}, reconnecting...", e);
                    break; // 跳出内层，触发重连
                }
            }
        }
    }
}

// 静态内存分配宏
macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}
use mk_static;
