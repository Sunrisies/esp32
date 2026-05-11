# Rust 项目结构诊断报告

## 项目概览

- 项目名称: `esp32`
- 主要依赖:
  - `esp-hal`: ESP32C6 硬件抽象层
  - `esp-radio` 和 `esp-rtos`: Wi-Fi 驱动与 Embassy 调度集成
  - `embassy-net` 和 `embassy-executor`: 异步网络栈与任务运行
  - `heapless`: `no_std` 下的固定容量字符串和集合
  - `reqwless`: 嵌入式 HTTP 客户端
- 二进制目标个数: 1
- 库目标个数: 1

## 模块与目录结构分析

### 核心模块列表

#### 入口模块

- `src/bin/main.rs`
- `src/bin/app/wifi.rs`
- `src/bin/app/http.rs`
- `src/bin/app/mqtt.rs`
- `src/bin/app/led.rs`
- `build.rs`
- `src/config.rs`

#### 业务逻辑模块

- `src/mqtt_manager.rs`
- `src/myrtio_mqtt/client.rs`
- `src/myrtio_mqtt/packet.rs`
- `src/myrtio_mqtt/runtime/traits.rs`
- `src/myrtio_mqtt/runtime/publisher.rs`
- `src/myrtio_mqtt/runtime/registry.rs`

#### 基础设施模块

- `src/myrtio_mqtt/transport.rs`
- `src/ws2812.rs`
- `.cargo/config.toml`

### 结构问题点

1. `src/myrtio_mqtt/runtime/event_loop.rs` 被导出为 `MqttRuntime`，但 `run()` 主体基本被注释掉并直接返回 `Ok(())`，运行时功能实际不可用。
2. `src/myrtio_mqtt/runtime/runtime.rs` 和 `src/myrtio_mqtt/runtime/module.rs` 是未挂载的旧实现，且与 `traits.rs` / `event_loop.rs` 存在重复设计。
3. `src/mqtt_mini.rs` 为空且未挂载，`MQTT_INCOMING`、`MqttMessage` 等符号未实际使用。
4. Wi-Fi 凭据和 MQTT broker 地址已迁出源码，由 `build.rs` 从环境变量、`.env.local` 或 `.env` 生成到 `OUT_DIR/app_config.rs`。

## Cargo.toml 诊断

### 依赖健康度

`cargo tree -d` 发现多个重复传递依赖版本，主要包括:

- `defmt` 0.3.100 和 1.0.1
- `embassy-sync` 0.6.2、0.7.2 和 0.8.0
- `embedded-io` 0.6.1 和 0.7.1
- `embedded-io-async` 0.6.1 和 0.7.0
- `heapless` 0.8.0 和 0.9.2
- `base64` 0.13.1 和 0.21.7
- `rand_core` 0.6.4、0.9.5 和 0.10.1

### 特性(features)使用情况

- `default` 仅启用 `esp32-log`。
- 已补充 `defmt` feature，避免 `cfg(feature = "defmt")` 触发 `unexpected cfg` 警告。
- MQTT 协议路径固定为 v3.1.1；未完成的 MQTT v5 properties/reason-code 实现已移除，不再默认启用。

### 最小化建议

- `log` 和 `critical-section` 未发现直接代码引用，已从直接依赖中移除；当前仍可能作为传递依赖存在于 `Cargo.lock`。
- `embassy-sync` 建议与依赖树主版本对齐。
- 如果不需要生成 `ws2812` 文档特性清单，可评估移除 `document-features`。
- 后续如确需 MQTT v5，应作为单独 feature 重新设计并补齐 properties、reason-code 与测试。

### 版本核对来源

- https://docs.rs/crate/esp-hal/latest
- https://docs.rs/crate/esp-radio/latest
- https://docs.rs/crate/esp-rtos/latest
- https://docs.rs/crate/embassy-net/latest
- https://docs.rs/crate/reqwless/latest

## 代码质量与规范

### 存在的编译警告

1. 已处理：`Cargo.toml` 已声明 `defmt` feature，`unexpected cfg condition value: defmt` 不应再出现。
2. `src/myrtio_mqtt/runtime/event_loop.rs`: 多个 import 未使用，`client` 和 `publisher_rx` 字段未读，`drain_outbox` 未使用。
3. `src/myrtio_mqtt/mod.rs` 和 `src/ws2812.rs`: `no_std` 等 crate-level 属性放在非 crate root 模块中。
4. `src/ws2812.rs`: `rgb::ComponentSlice` 和 `as_slice` 已废弃。
5. `src/mqtt_manager.rs`: `topic` 参数未使用，并存在不必要的 `mut`。

### Clippy 建议

1. `src/mqtt_manager.rs`: `static mut` 缓冲区立即解引用，建议改为 `StaticCell` 或明确的单例缓冲管理。
2. 已处理：`src/myrtio_mqtt/util.rs` 中未完成的 MQTT v5 properties 辅助函数已移除。
3. `src/myrtio_mqtt/runtime/event_loop.rs`: 大量字段、常量和 import 未使用，根因是 `MqttRuntime::run` 仍未实现。

### 潜在的内存/性能风险

1. `src/mqtt_manager.rs` 使用 `static mut RX_BUFFER/TX_BUFFER` 加 `unsafe` 可变借用，在异步重连和未来任务扩展下风险较高。
2. `src/bin/app/http.rs`、`src/myrtio_mqtt/client.rs`、`src/mqtt_manager.rs` 存在多个 1024 至 4096 字节栈缓冲，嵌入式环境下需核算任务栈。
3. `src/myrtio_mqtt/client.rs` 在等待 `PUBACK` / `SUBACK` 时跳过 `Publish` 包，可能丢失并发到达的业务消息。
4. `src/myrtio_mqtt/runtime/publisher.rs` 的 `BufferedOutbox` 在容量不足或 payload 过大时静默丢弃发布请求。

### 文档覆盖率

- `src/myrtio_mqtt` 和 `src/ws2812.rs` 文档较多。
- `src/lib.rs` 缺少 crate-level 文档。
- `src/mqtt_manager.rs` 的 public 类型和静态通道缺少说明。
- `README.md` 和 `ARCHITECTURE.md` 已同步到当前入口拆分结构。
- `examples/` 已补充可编译示例，但测试目录仍缺失。

## 行动清单

| 优先级 | 行动内容 | 建议负责人/时间 |
| --- | --- | --- |
| 高 | 修复 `MqttRuntime::run` 的真实实现，统一保留 `event_loop.rs` 或 `runtime.rs` 其中一套，并删除未挂载旧文件 | 建议本周完成 |
| 高 | 移除 `src/mqtt_manager.rs` 中的 `static mut`，改用 `StaticCell` 或受控缓冲 | 建议本周完成 |
| 高 | 已完成：将 Wi-Fi SSID、密码和 MQTT broker 地址从源码迁出，由 `build.rs` 从环境变量或 `.env` 生成配置 | 2026-05-12 已完成 |
| 中 | 已完成：清理 Cargo features，补充 `defmt` feature，移除默认 `v5` 和未完成的 MQTT v5 代码，并删除未使用的 `log` / `critical-section` 直接依赖 | 2026-05-12 已完成 |
| 中 | 为 MQTT packet 编解码、`PUBACK` / `SUBACK`、buffer 边界和 `TopicRegistry` 增加最小测试集 | 建议下个迭代 |
| 中 | 已完成：拆分 `src/bin/main.rs`，将 Wi-Fi、HTTP、MQTT、LED 控制拆成 `src/bin/app/` 独立模块 | 2026-05-12 已完成 |
