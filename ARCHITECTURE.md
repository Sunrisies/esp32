# myrtio-mqtt 架构

## 概述
`myrtio-mqtt` 是一个专为嵌入式系统设计的、`no_std` 兼容的异步 MQTT 客户端，构建在 Embassy 异步生态系统之上。

## 核心组件

### 1. MqttClient（直接使用）
- 简单直接的 MQTT 客户端，适用于基本应用
- 处理连接、订阅、发布和事件处理
- 位于 `src/myrtio_mqtt/client.rs`

### 2. MqttRuntime（高级使用）
- 用于具有多个关注点的复杂应用的运行时系统
- 使用基于 `MqttModule` 特性的模块化架构
- 位于 `src/myrtio_mqtt/runtime/`

#### 运行时子组件：
- `runtime.rs`: 主运行时协调器
- `module.rs`: MqttModule 特性定义
- `publisher.rs`: 处理消息发布
- `registry.rs`: 主题注册管理
- `event_loop.rs`: 主事件循环处理
- `traits.rs`: 共享特性和类型定义

### 3. 传输层
- 通过 `MqttTransport` 特性实现传输不可知设计
- 支持 TCP、UART、SPI 或任何可靠的基于流的通道
- 位于 `src/myrtio_mqtt/transport.rs`

### 4. 数据包处理
- MQTT 数据包的编码/解码，支持 v3.1.1 和 v5
- 位于 `src/myrtio_mqtt/packet.rs`

### 5. 实用工具
- 辅助函数和共享工具
- 位于 `src/myrtio_mqtt/util.rs`

### 6. 错误处理
- 集中化的错误类型和处理
- 位于 `src/myrtio_mqtt/error.rs`

## 数据流

1. **传输层** 提供原始字节流
2. **数据包层** 编码/解码 MQTT 数据包
3. **客户端/运行时** 管理 MQTT 状态机：
   - 连接处理
   - 订阅管理
   - 发布/QoS 处理
   - 事件生成
4. **应用逻辑** 通过：
   - 直接客户端使用（简单情况）
   - 运行时模块（具有多个关注点的复杂情况）

## 关键设计原则

- **no_std & no_alloc**: 使用 heapless 进行缓冲区管理
- **完全异步**: 基于 async/await 构建，与 Embassy 兼容
- **传输不可知**: 可在任何可靠的流上运行
- **模块化**: 运行时支持可插拔模块
- **特性标志**: 通过 Cargo 特性支持 MQTT v3.1.1/v5

## 使用模式

### 简单应用
```rust
let mut client = MqttClient::<_, 5, 256>::new(transport, options);
client.connect().await?;
client.subscribe("topic", QoS::AtMostOnce).await?;
client.publish("topic", b"payload", QoS::AtMostOnce).await?;
```

### 复杂应用
```rust
use myrtio_mqtt::runtime::{MqttModule, MqttRuntime, TopicRegistry, Context};

struct MyModule;

impl<'a, T, const MAX_TOPICS: usize, const BUF_SIZE: usize>
    MqttModule<'a, T, MAX_TOPICS, BUF_SIZE> for MyModule
where
    T: MqttTransport,
{
    fn register<'reg>(&'reg self, registry: &mut TopicRegistry<'reg, MAX_TOPICS>) {
        let _ = registry.add("device/cmd");
    }

    fn on_message<'m>(&mut self, msg: &Publish<'m>) {
        // 处理传入消息
    }
}
```

## 依赖项
- Embassy（异步生态系统）
- heapless（固定容量数据结构）
- tinyvec（可选，用于某些实用工具）