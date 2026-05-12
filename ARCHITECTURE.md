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
  - 变量长度整数、UTF-8 字符串辅助函数

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
  - 本地 HTTP 配网页面，提供 AP/STA 两个入口的 Wi-Fi 凭据修改接口
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

    fn needs_immediate_publish(&self) -> bool {
        self.pending
    }

    fn on_publish(&mut self, outbox: &mut dyn PublishOutbox) {
        if self.pending {
            outbox.publish("device/state", b"updated", myrtio_mqtt::QoS::AtLeastOnce);
            self.pending = false;
        }
    }
}
```

`MqttRuntime::run` 是当前唯一保留的运行时入口，位于 `src/myrtio_mqtt/runtime/event_loop.rs`。
它会在启动时连接 broker、订阅模块注册的 topic、执行 `on_start`，之后在同一个循环里处理 MQTT 收包、外部 `PublishRequestChannel` 发布请求和周期 `on_tick`。

多业务通道建议优先使用多个明确 topic，例如 `device/ch1/cmd`、`device/ch2/cmd`。
如果 broker 侧只能共用一个 topic，则在 payload 中放 `channel` 或 `type` 字段，并在 `on_message` 中根据内容分发到不同业务逻辑。
运行时不需要为每个频道复制一套。

## 当前架构边界

这个仓库当前更接近 ESP32C6 Rust 学习和 bring-up 工程，不是已经收敛的生产固件。后续架构应先收敛以下边界：

- MQTT 应统一到 `MqttRuntime` / `MqttModule` 模型。当前 `src/mqtt_manager.rs` 仍然手写连接、订阅、分发和重连逻辑，和 runtime 抽象并存。
- MQTT over TCP 需要先做帧边界层。TCP read 不等于 MQTT packet，不能让 packet decoder 直接处理任意一次 read 的结果。
- 业务通道建议放在 topic 层，例如 `device/{device_id}/ch/{channel}/cmd`；如果必须共用一个 topic，则 payload 必须有稳定字段，例如 `channel`、`type`、`value`。
- 设备 availability 建议使用同一个 retained topic 表示 online/offline，避免 retained offline 状态残留。
- AP/HTTP 调试入口已从 STA 联网状态解耦。设备配网错误时，AP 侧仍能提供本地 Wi-Fi 配置页面。
- `.env` 只适合构建期注入，不是安全凭据存储。烧录后的固件中仍包含这些字符串。
- HTTP 提交的新 Wi-Fi 凭据当前只在运行时生效，后续需要接入 flash/NVS 才能跨重启保存。
- `src/ws2812.rs` 目前像从独立 crate 搬入的适配器，仍需要整理 crate-level attribute、RGBW 宏路径、RMT 时钟假设和废弃 rgb API。

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
- 默认启用 `esp32-log`，应用日志统一走本地日志门面；MQTT/TCP 帧级日志需额外启用 `mqtt-protocol-log`
- `protocol-log` 统一控制详细协议 dump，当前 HTTP 原始请求 dump 也挂在该 feature 下
- MQTT 协议路径固定为 v3.1.1

## 建议的演进顺序

1. 先修复 MQTT 帧边界、packet 解码边界检查和 retained availability 这类会直接影响联网结果的问题。
2. 再把 `src/mqtt_manager.rs` 迁移为一个或多个 `MqttModule`，让 topic 注册、消息分发和发布都走统一 runtime。
3. 然后整理 HTTP 和 LED：HTTP 配置页接入凭据持久化，LED 命令从纯字符串升级为稳定命令协议。
4. 最后清理依赖、日志、WS2812 适配器和 RAM 预算，让工程从学习示例收敛成可维护固件。
