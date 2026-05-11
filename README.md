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

构建时通过环境变量或 `.env` 注入设备配置。未设置时会使用可编译的占位默认值，实际烧录前应显式设置。

推荐本地开发使用 `.env`：

```text
ESP32_WIFI_SSID=your-wifi-ssid
ESP32_WIFI_PASSWORD=your-wifi-password
ESP32_MQTT_BROKER_IP=192.168.1.10
ESP32_MQTT_PORT=1883
ESP32_MQTT_CLIENT_ID=esp32c6-client
```

可从 `.env.example` 复制一份 `.env`。`.env` 和 `.env.local` 会被 Git 忽略，不要提交真实凭据。

配置优先级：

1. 构建环境变量
2. `.env.local`
3. `.env`
4. `build.rs` 中的占位默认值

PowerShell 示例：

```powershell
$env:ESP32_WIFI_SSID="your-wifi-ssid"
$env:ESP32_WIFI_PASSWORD="your-wifi-password"
$env:ESP32_MQTT_BROKER_IP="192.168.1.10"
$env:ESP32_MQTT_PORT="1883"
$env:ESP32_MQTT_CLIENT_ID="esp32c6-client"
```

支持的配置项：

- `ESP32_WIFI_SSID`: STA 模式连接的 Wi-Fi SSID
- `ESP32_WIFI_PASSWORD`: STA 模式连接的 Wi-Fi 密码
- `ESP32_WIFI_AP_SSID`: 设备 AP 模式 SSID，默认 `esp-radio-apsta`
- `ESP32_WIFI_AP_IP`: 设备 AP 静态 IP，默认 `192.168.2.1`
- `ESP32_MQTT_BROKER_IP`: MQTT broker IPv4 地址，默认 `0.0.0.0`，实际烧录前必须设置
- `ESP32_MQTT_PORT`: MQTT broker 端口，默认 `1883`
- `ESP32_MQTT_CLIENT_ID`: MQTT client id，默认 `esp32c6-client`

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

- 默认启用 `esp32-log`；MQTT 协议路径固定为 v3.1.1，未启用未完成的 v5 实现
- 工程面向裸机目标，依赖 `esp-hal`、`esp-radio`、`embassy-*` 生态
- Wi-Fi 凭据和 MQTT broker 地址由 `build.rs` 生成到 `OUT_DIR`，不再保存在源码中
