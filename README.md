# esp32

这是一个面向 ESP32C6 的 `no_std` 异步示例工程，包含 Wi-Fi、HTTP、MQTT 和 WS2812 LED 控制代码。

## 主要内容

- `src/bin/main.rs`：设备入口，只保留启动顺序、静态资源分配和任务编排
- `src/bin/app/wifi.rs`：Wi-Fi AP/STA 初始化、网络栈 runner、连接监控
- `src/bin/app/http.rs`：HTTP 监听、请求读取、到 httpbin 的转发演示
- `src/bin/app/mqtt.rs`：MQTT manager 任务启动封装
- `src/bin/app/led.rs`：WS2812 LED 控制任务
- `src/myrtio_mqtt/`：可复用的 MQTT 客户端与运行时
- `src/ws2812.rs`：RMT 驱动的智能灯带适配器
- `examples/`：可编译示例，演示 MQTT 模块化用法

## 构建

```bash
cargo check --target riscv32imac-unknown-none-elf
cargo check --examples --target riscv32imac-unknown-none-elf
```

## 示例

- `examples/const_topics_module.rs`
  - 演示固定主题的 MQTT 模块
  - 展示 `TopicCollector`、`PublishOutbox` 和 `MqttRuntime` 的基本接线
- `examples/dynamic_topics_module.rs`
  - 演示基于设备 ID 动态拼接主题
  - 展示 `heapless::String` 组织运行时主题的方式

## 说明

- 默认启用 `esp32-log` 和 `v5` 特性
- 工程面向裸机目标，依赖 `esp-hal`、`esp-radio`、`embassy-*` 生态
- 设备凭据和 broker 地址目前仍在源码中，建议后续迁移到配置层
