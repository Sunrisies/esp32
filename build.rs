fn main() {
    linker_be_nice();
    generate_app_config();
    println!("cargo:rustc-link-arg=-Tdefmt.x");
    // make sure linkall.x is the last linker script (otherwise might cause problems with flip-link)
    println!("cargo:rustc-link-arg=-Tlinkall.x");
}

fn generate_app_config() {
    println!("cargo:rerun-if-changed=.env");
    println!("cargo:rerun-if-changed=.env.local");
    println!("cargo:rerun-if-env-changed=ESP32_WIFI_SSID");
    println!("cargo:rerun-if-env-changed=ESP32_WIFI_PASSWORD");
    println!("cargo:rerun-if-env-changed=ESP32_WIFI_AP_SSID");
    println!("cargo:rerun-if-env-changed=ESP32_WIFI_AP_IP");
    println!("cargo:rerun-if-env-changed=ESP32_MQTT_BROKER_IP");
    println!("cargo:rerun-if-env-changed=ESP32_MQTT_PORT");
    println!("cargo:rerun-if-env-changed=ESP32_MQTT_CLIENT_ID");

    let dot_env = DotEnv::load();

    let wifi_ssid = config_value("ESP32_WIFI_SSID", "CHANGE_ME_WIFI_SSID", &dot_env);
    let wifi_password = config_value("ESP32_WIFI_PASSWORD", "CHANGE_ME_WIFI_PASSWORD", &dot_env);
    let wifi_ap_ssid = config_value("ESP32_WIFI_AP_SSID", "esp-radio-apsta", &dot_env);
    let wifi_ap_ip = ap_ipv4_octets("ESP32_WIFI_AP_IP", "192.168.2.1", &dot_env);
    let mqtt_broker_ip = ipv4_octets("ESP32_MQTT_BROKER_IP", "0.0.0.0", &dot_env);
    let mqtt_port = port_or_default("ESP32_MQTT_PORT", 1883, &dot_env);
    let mqtt_client_id = config_value("ESP32_MQTT_CLIENT_ID", "esp32c6-client", &dot_env);

    let generated = format!(
        r#"pub const WIFI_SSID: &str = {wifi_ssid};
pub const WIFI_PASSWORD: &str = {wifi_password};
pub const WIFI_AP_SSID: &str = {wifi_ap_ssid};
pub const WIFI_AP_IP: core::net::Ipv4Addr = core::net::Ipv4Addr::new({ap0}, {ap1}, {ap2}, {ap3});
pub const MQTT_BROKER_IP: core::net::Ipv4Addr = core::net::Ipv4Addr::new({mqtt0}, {mqtt1}, {mqtt2}, {mqtt3});
pub const MQTT_PORT: u16 = {mqtt_port};
pub const MQTT_CLIENT_ID: &str = {mqtt_client_id};
"#,
        wifi_ssid = rust_string_literal(&wifi_ssid),
        wifi_password = rust_string_literal(&wifi_password),
        wifi_ap_ssid = rust_string_literal(&wifi_ap_ssid),
        ap0 = wifi_ap_ip[0],
        ap1 = wifi_ap_ip[1],
        ap2 = wifi_ap_ip[2],
        ap3 = wifi_ap_ip[3],
        mqtt0 = mqtt_broker_ip[0],
        mqtt1 = mqtt_broker_ip[1],
        mqtt2 = mqtt_broker_ip[2],
        mqtt3 = mqtt_broker_ip[3],
        mqtt_port = mqtt_port,
        mqtt_client_id = rust_string_literal(&mqtt_client_id),
    );

    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR is set"));
    std::fs::write(out_dir.join("app_config.rs"), generated).expect("write generated app config");
}

struct DotEnv {
    values: std::collections::BTreeMap<String, String>,
}

impl DotEnv {
    fn load() -> Self {
        let mut values = std::collections::BTreeMap::new();
        Self::load_file(".env", &mut values);
        Self::load_file(".env.local", &mut values);
        Self { values }
    }

    fn load_file(path: &str, values: &mut std::collections::BTreeMap<String, String>) {
        let Ok(contents) = std::fs::read_to_string(path) else {
            return;
        };

        for (line_no, raw_line) in contents.lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let line = line.strip_prefix("export ").unwrap_or(line);
            let Some((key, value)) = line.split_once('=') else {
                panic!("{path}:{} must be KEY=VALUE", line_no + 1);
            };

            let key = key.trim();
            if key.is_empty() {
                panic!("{path}:{} has an empty key", line_no + 1);
            }

            values.insert(key.to_owned(), unquote_env_value(value.trim()));
        }
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }
}

fn unquote_env_value(value: &str) -> String {
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        let quoted = (bytes[0] == b'"' && bytes[value.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[value.len() - 1] == b'\'');
        if quoted {
            return value[1..value.len() - 1].to_owned();
        }
    }

    value.to_owned()
}

fn config_value(name: &str, default: &str, dot_env: &DotEnv) -> String {
    std::env::var(name)
        .ok()
        .or_else(|| dot_env.get(name).map(str::to_owned))
        .unwrap_or_else(|| default.to_owned())
}

fn ipv4_octets(name: &str, default: &str, dot_env: &DotEnv) -> [u8; 4] {
    let value = config_value(name, default, dot_env);
    let address: std::net::Ipv4Addr = value
        .parse()
        .unwrap_or_else(|_| panic!("{name} must be a valid IPv4 address, got `{value}`"));
    address.octets()
}

fn ap_ipv4_octets(name: &str, default: &str, dot_env: &DotEnv) -> [u8; 4] {
    let value = config_value(name, default, dot_env);
    let address: std::net::Ipv4Addr = value
        .parse()
        .unwrap_or_else(|_| panic!("{name} must be a valid IPv4 address, got `{value}`"));
    let octets = address.octets();

    if address.is_unspecified()
        || address.is_broadcast()
        || address.is_loopback()
        || address.is_multicast()
        || octets[3] == 0
        || octets[3] == 255
    {
        panic!("{name} must be a usable /24 AP host address, got `{value}`");
    }

    octets
}

fn port_or_default(name: &str, default: u16, dot_env: &DotEnv) -> u16 {
    let default = default.to_string();
    let value = config_value(name, &default, dot_env);
    value
        .parse()
        .unwrap_or_else(|_| panic!("{name} must be a valid u16 port, got `{value}`"))
}

fn rust_string_literal(value: &str) -> String {
    format!("{value:?}")
}

fn linker_be_nice() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        let kind = &args[1];
        let what = &args[2];

        match kind.as_str() {
            "undefined-symbol" => match what.as_str() {
                what if what.starts_with("_defmt_") => {
                    eprintln!();
                    eprintln!(
                        "💡 `defmt` not found - make sure `defmt.x` is added as a linker script and you have included `use defmt_rtt as _;`"
                    );
                    eprintln!();
                }
                "_stack_start" => {
                    eprintln!();
                    eprintln!("💡 Is the linker script `linkall.x` missing?");
                    eprintln!();
                }
                what if what.starts_with("esp_rtos_") => {
                    eprintln!();
                    eprintln!(
                        "💡 `esp-radio` has no scheduler enabled. Make sure you have initialized `esp-rtos` or provided an external scheduler."
                    );
                    eprintln!();
                }
                "embedded_test_linker_file_not_added_to_rustflags" => {
                    eprintln!();
                    eprintln!(
                        "💡 `embedded-test` not found - make sure `embedded-test.x` is added as a linker script for tests"
                    );
                    eprintln!();
                }
                "free"
                | "malloc"
                | "calloc"
                | "get_free_internal_heap_size"
                | "malloc_internal"
                | "realloc_internal"
                | "calloc_internal"
                | "free_internal" => {
                    eprintln!();
                    eprintln!(
                        "💡 Did you forget the `esp-alloc` dependency or didn't enable the `compat` feature on it?"
                    );
                    eprintln!();
                }
                _ => (),
            },
            // we don't have anything helpful for "missing-lib" yet
            _ => {
                std::process::exit(1);
            }
        }

        std::process::exit(0);
    }

    println!(
        "cargo:rustc-link-arg=--error-handling-script={}",
        std::env::current_exe().unwrap().display()
    );
}
