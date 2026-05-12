#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {{
        #[cfg(feature = "esp32-log")]
        {
            esp_println::println!($($arg)*);
        }
        #[cfg(all(not(feature = "esp32-log"), feature = "defmt"))]
        {
            defmt::info!("{}", defmt::Display2Format(&core::format_args!($($arg)*)));
        }
    }};
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {{
        #[cfg(feature = "esp32-log")]
        {
            esp_println::println!($($arg)*);
        }
        #[cfg(all(not(feature = "esp32-log"), feature = "defmt"))]
        {
            defmt::warn!("{}", defmt::Display2Format(&core::format_args!($($arg)*)));
        }
    }};
}

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {{
        #[cfg(feature = "esp32-log")]
        {
            esp_println::println!($($arg)*);
        }
        #[cfg(all(not(feature = "esp32-log"), feature = "defmt"))]
        {
            defmt::error!("{}", defmt::Display2Format(&core::format_args!($($arg)*)));
        }
    }};
}

#[macro_export]
macro_rules! protocol_log {
    ($($arg:tt)*) => {{
        #[cfg(feature = "protocol-log")]
        {
            $crate::log_info!($($arg)*);
        }
    }};
}

#[macro_export]
macro_rules! mqtt_protocol_log {
    ($($arg:tt)*) => {{
        #[cfg(feature = "protocol-log")]
        {
            $crate::log_info!($($arg)*);
        }
    }};
}
