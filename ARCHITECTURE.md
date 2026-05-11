# myrtio-mqtt 架构

## 概述

`myrtio-mqtt` 是一个面向嵌入式系统的 `no_std` 异步 MQTT 客户端，构建在 Embassy 生态之上。当前仓库中的实现分成三层：协议与传输、运行时编排、应用集成。

## 目录分层

### 配置层

- `build.rs`
  - 从构建环境变量、`.env.local` 或 `.env` 读取设备配置
  - 生成 `OUT_DIR/app_config.rs`
- `src/config.rs`
  - 通过 `include!` 暴露生成后的配置常量
  - 当前包含 Wi-Fi SSID、Wi-Fi 密码、AP 配置、MQTT broker 地址、端口和 client id
- `.env.example`
  - 可提交的本地配置模板，不包含真实凭据

### 协议与传输层

- `src/myrtio_mqtt/client.rs`
  - MQTT 连接、发布、订阅、轮询
- `src/myrtio_mqtt/packet.rs`
  - MQTT 数据包编码与解码
- `src/myrtio_mqtt/transport.rs`
  - `MqttTransport` 抽象和 `TcpTransport`
- `src/myrtio_mqtt/error.rs`
  - 协议错误、连接错误、传输错误
- `src/myrtio_mqtt/util.rs`
  - 变量长度整数、UTF-8 字符串、v5 属性辅助函数

### 运行时层

- `src/myrtio_mqtt/runtime/traits.rs`
  - `MqttModule`
  - `TopicCollector`
  - `PublishOutbox`
- `src/myrtio_mqtt/runtime/registry.rs`
  - `TopicRegistry`
- `src/myrtio_mqtt/runtime/publisher.rs`
  - `PublishRequest`
  - `PublisherHandle`
  - `BufferedOutbox`
- `src/myrtio_mqtt/runtime/event_loop.rs`
  - 当前导出的 `MqttRuntime`

### 参考实现

- `src/myrtio_mqtt/runtime/runtime.rs`
- `src/myrtio_mqtt/runtime/module.rs`

这两个文件保留为参考实现，不是当前 `runtime` 模块对外导出的主路径。

### 应用层

- `src/mqtt_manager.rs`
  - MQTT 业务消息处理与 LED 信道联动
- `src/ws2812.rs`
  - WS2812 / RMT 适配器
- `src/bin/main.rs`
  - 启动顺序、静态资源分配和任务编排
- `src/bin/app/wifi.rs`
  - Wi-Fi AP/STA 初始化、网络栈 runner、连接监控
- `src/bin/app/http.rs`
  - HTTP 监听、请求读取、上游 HTTP 客户端转发
- `src/bin/app/mqtt.rs`
  - MQTT manager 任务启动封装
- `src/bin/app/led.rs`
  - LED 控制任务和颜色命令解析

## 数据流

1. 传输层提供字节流。
2. `packet.rs` 将字节流编码或解码为 MQTT 包。
3. `client.rs` 维护连接状态、订阅、发布和 keep-alive。
4. `runtime/` 负责把 MQTT 消息分发给 `MqttModule`，并把模块产生的发布请求送回客户端。
5. `bin/main.rs` 负责启动编排，`src/bin/app/` 下的模块分别承载 Wi-Fi、HTTP、MQTT 和 LED 控制。
6. Wi-Fi 和 MQTT 参数由 `build.rs` 生成的 `esp32::config` 模块提供。

## 当前运行时 API

当前可用的运行时接口以 `runtime/traits.rs` 为中心：

```rust
use myrtio_mqtt::runtime::{MqttModule, PublishOutbox, TopicCollector};

struct MyModule {
    pending: bool,
}

impl MqttModule for MyModule {
    fn register(&self, collector: &mut dyn TopicCollector) {
        let _ = collector.add("device/cmd");
    }

    fn on_message(&mut self, msg: &myrtio_mqtt::Publish<'_>) {
        if msg.topic == "device/cmd" {
            self.pending = true;
        }
    }

    fn on_tick(&mut self, outbox: &mut dyn PublishOutbox) -> embassy_time::Duration {
        if self.pending {
            outbox.publish("device/state", b"updated", myrtio_mqtt::QoS::AtLeastOnce);
            self.pending = false;
        }
        embassy_time::Duration::from_secs(30)
    }
}
```

## 可编译示例

仓库根目录下提供两个可编译示例：

- `examples/const_topics_module.rs`
  - 静态主题、保留消息、Last Will
- `examples/dynamic_topics_module.rs`
  - 基于设备 ID 动态拼接主题

编译命令：

```bash
cargo check --examples --target riscv32imac-unknown-none-elf
```

## 关键设计原则

- `no_std` 和 `no_alloc`
- 固定容量缓冲区，尽量避免运行时分配
- 传输不可知，客户端只依赖 `MqttTransport`
- 模块化运行时，业务逻辑通过 `MqttModule` 接入
- 构建期配置注入，避免在源码中保存设备凭据和 broker 地址
- 默认启用 `esp32-log` 和 MQTT v5 支持
