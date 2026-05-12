use core::{fmt::Write as FmtWrite, str};

use embassy_futures::select::Either;
use embassy_net::{IpListenEndpoint, Stack, tcp::TcpSocket};
use embassy_time::{Duration, Timer};
use embedded_io_async::Write;
use heapless::{String, Vec};

use crate::wifi::{self, MAX_WIFI_PASSWORD_LEN, MAX_WIFI_SSID_LEN, WifiConfigError};

const HTTP_PORT: u16 = 8080;
const REQUEST_BUFFER_SIZE: usize = 1536;

struct HttpRequest<'a> {
    method: &'a str,
    path: &'a str,
    body: &'a str,
}

pub async fn serve(ap_stack: Stack<'static>, sta_stack: Stack<'static>) -> ! {
    let mut ap_server_rx_buffer = [0; 1536];
    let mut ap_server_tx_buffer = [0; 1536];
    let mut sta_server_rx_buffer = [0; 1536];
    let mut sta_server_tx_buffer = [0; 1536];

    let mut ap_server_socket =
        TcpSocket::new(ap_stack, &mut ap_server_rx_buffer, &mut ap_server_tx_buffer);
    ap_server_socket.set_timeout(Some(Duration::from_secs(10)));

    let mut sta_server_socket = TcpSocket::new(
        sta_stack,
        &mut sta_server_rx_buffer,
        &mut sta_server_tx_buffer,
    );
    sta_server_socket.set_timeout(Some(Duration::from_secs(10)));

    loop {
        esp32::log_info!("Waiting for local HTTP connection...");

        let sta_ready = sta_stack.is_link_up() && sta_stack.is_config_up();
        let server_socket = if sta_ready {
            let either_socket = embassy_futures::select::select(
                ap_server_socket.accept(IpListenEndpoint {
                    addr: None,
                    port: HTTP_PORT,
                }),
                sta_server_socket.accept(IpListenEndpoint {
                    addr: None,
                    port: HTTP_PORT,
                }),
            )
            .await;
            match either_socket {
                Either::First(result) => {
                    if let Err(e) = result {
                        esp32::log_warn!("HTTP accept error: {:?}", e);
                        continue;
                    }
                    &mut ap_server_socket
                }
                Either::Second(result) => {
                    if let Err(e) = result {
                        esp32::log_warn!("HTTP accept error: {:?}", e);
                        continue;
                    }
                    &mut sta_server_socket
                }
            }
        } else if let Err(e) = ap_server_socket
            .accept(IpListenEndpoint {
                addr: None,
                port: HTTP_PORT,
            })
            .await
        {
            esp32::log_warn!("HTTP accept error: {:?}", e);
            continue;
        } else {
            &mut ap_server_socket
        };

        handle_connection(server_socket, sta_stack).await;

        if let Err(e) = server_socket.flush().await {
            esp32::log_warn!("HTTP flush error: {:?}", e);
        }

        Timer::after(Duration::from_millis(100)).await;
        server_socket.close();
        Timer::after(Duration::from_millis(100)).await;
        server_socket.abort();
    }
}

async fn handle_connection(socket: &mut TcpSocket<'_>, sta_stack: Stack<'static>) {
    let mut buffer = [0u8; REQUEST_BUFFER_SIZE];
    let Some(len) = read_request(socket, &mut buffer).await else {
        write_text_response(socket, 400, "Bad Request", "bad request").await;
        return;
    };

    let request = match parse_request(&buffer[..len]) {
        Ok(request) => request,
        Err(()) => {
            write_text_response(socket, 400, "Bad Request", "bad request").await;
            return;
        }
    };

    match (request.method, request.path) {
        ("GET", "/") | ("GET", "/wifi") => write_config_page(socket, sta_stack, None).await,
        ("POST", "/wifi") => handle_wifi_post(socket, sta_stack, request.body).await,
        _ => write_text_response(socket, 404, "Not Found", "not found").await,
    }
}

async fn read_request(socket: &mut TcpSocket<'_>, buffer: &mut [u8]) -> Option<usize> {
    let mut pos = 0;

    loop {
        if pos >= buffer.len() {
            esp32::log_warn!("HTTP request too large");
            return None;
        }

        match socket.read(&mut buffer[pos..]).await {
            Ok(0) => return None,
            Ok(len) => {
                pos += len;
                if request_complete(&buffer[..pos]) {
                    esp32::protocol_log!(
                        "HTTP request:\n{}",
                        str::from_utf8(&buffer[..pos]).unwrap_or("<non-utf8>")
                    );
                    return Some(pos);
                }
            }
            Err(e) => {
                esp32::log_warn!("HTTP read error: {:?}", e);
                return None;
            }
        }
    }
}

fn request_complete(buffer: &[u8]) -> bool {
    let Some(header_end) = find_header_end(buffer) else {
        return false;
    };

    let Ok(header) = str::from_utf8(&buffer[..header_end]) else {
        return true;
    };

    let content_len = content_length(header).unwrap_or(0);
    buffer.len() >= header_end + 4 + content_len
}

fn parse_request(buffer: &[u8]) -> Result<HttpRequest<'_>, ()> {
    let text = str::from_utf8(buffer).map_err(|_| ())?;
    let header_end = text.find("\r\n\r\n").ok_or(())?;
    let header = &text[..header_end];
    let mut lines = header.lines();
    let request_line = lines.next().ok_or(())?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().ok_or(())?;
    let path = request_parts.next().ok_or(())?;

    let content_len = content_length(header).unwrap_or(0);
    let body_start = header_end + 4;
    let body_end = body_start.saturating_add(content_len).min(text.len());

    Ok(HttpRequest {
        method,
        path,
        body: &text[body_start..body_end],
    })
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn content_length(header: &str) -> Option<usize> {
    for line in header.lines() {
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                return value.trim().parse().ok();
            }
        }
    }
    None
}

async fn handle_wifi_post(socket: &mut TcpSocket<'_>, sta_stack: Stack<'static>, body: &str) {
    let mut ssid = String::<MAX_WIFI_SSID_LEN>::new();
    let mut password = String::<MAX_WIFI_PASSWORD_LEN>::new();

    if parse_wifi_form(body, &mut ssid, &mut password).is_err() {
        write_config_page(socket, sta_stack, Some("Wi-Fi form is invalid")).await;
        return;
    }

    match wifi::request_station_credentials(ssid.as_str(), password.as_str()) {
        Ok(()) => {
            esp32::log_info!(
                "Queued new STA Wi-Fi credentials for SSID '{}'",
                ssid.as_str()
            );
            write_config_page(
                socket,
                sta_stack,
                Some("Saved. The device is reconnecting with the new Wi-Fi settings."),
            )
            .await;
        }
        Err(WifiConfigError::EmptySsid) => {
            write_config_page(socket, sta_stack, Some("SSID cannot be empty")).await;
        }
        Err(WifiConfigError::SsidTooLong) => {
            write_config_page(socket, sta_stack, Some("SSID is too long")).await;
        }
        Err(WifiConfigError::PasswordTooLong) => {
            write_config_page(socket, sta_stack, Some("Password is too long")).await;
        }
    }
}

fn parse_wifi_form(
    body: &str,
    ssid: &mut String<MAX_WIFI_SSID_LEN>,
    password: &mut String<MAX_WIFI_PASSWORD_LEN>,
) -> Result<(), ()> {
    for pair in body.split('&') {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        match name {
            "ssid" => percent_decode(value, ssid)?,
            "password" => percent_decode(value, password)?,
            _ => {}
        }
    }

    if ssid.is_empty() { Err(()) } else { Ok(()) }
}

fn percent_decode<const N: usize>(input: &str, output: &mut String<N>) -> Result<(), ()> {
    output.clear();
    let bytes = input.as_bytes();
    let mut decoded = Vec::<u8, N>::new();
    let mut index = 0;

    while index < bytes.len() {
        let byte = match bytes[index] {
            b'+' => {
                index += 1;
                b' '
            }
            b'%' if index + 2 < bytes.len() => {
                let decoded = (hex_value(bytes[index + 1])? << 4) | hex_value(bytes[index + 2])?;
                index += 3;
                decoded
            }
            byte => {
                index += 1;
                byte
            }
        };

        decoded.push(byte).map_err(|_| ())?;
    }

    let decoded = str::from_utf8(decoded.as_slice()).map_err(|_| ())?;
    output.push_str(decoded).map_err(|_| ())?;
    Ok(())
}

fn hex_value(byte: u8) -> Result<u8, ()> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(()),
    }
}

async fn write_config_page(
    socket: &mut TcpSocket<'_>,
    sta_stack: Stack<'static>,
    message: Option<&str>,
) {
    write_all(
        socket,
        b"HTTP/1.0 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n",
    )
    .await;

    write_all(
        socket,
        br#"<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Wi-Fi Setup</title><style>body{font-family:system-ui,sans-serif;margin:0;background:#f4f6f8;color:#111827}main{max-width:520px;margin:0 auto;padding:32px 20px}form{display:grid;gap:14px}label{font-size:14px;font-weight:600}input{width:100%;box-sizing:border-box;padding:12px;border:1px solid #cbd5e1;border-radius:6px;font-size:16px}button{padding:12px 16px;border:0;border-radius:6px;background:#0f766e;color:white;font-size:16px;font-weight:700}.status{margin:16px 0;padding:12px;border-radius:6px;background:#e0f2fe}.meta{margin:0 0 20px;color:#4b5563;font-size:14px}</style></head><body><main><h1>Wi-Fi Setup</h1>"#,
    )
    .await;

    write_all(socket, b"<p class=\"meta\">AP address: ").await;
    write_display(socket, esp32::config::WIFI_AP_IP).await;
    write_all(socket, b":8080").await;

    if let Some(config) = sta_stack.config_v4() {
        write_all(socket, b"<br>STA address: ").await;
        write_display(socket, config.address.address()).await;
        write_all(socket, b":8080").await;
    } else if sta_stack.is_link_up() {
        write_all(socket, b"<br>STA is connecting").await;
    } else {
        write_all(socket, b"<br>STA is not connected").await;
    }
    write_all(socket, b"</p>").await;

    if let Some(message) = message {
        write_all(socket, b"<div class=\"status\">").await;
        write_escaped(socket, message).await;
        write_all(socket, b"</div>").await;
    }

    write_all(
        socket,
        br#"<form method="post" action="/wifi"><label for="ssid">Wi-Fi SSID</label><input id="ssid" name="ssid" maxlength="32" required value=""#,
    )
    .await;
    if let Some(credentials) = wifi::current_credentials() {
        write_escaped(socket, credentials.ssid()).await;
    }
    write_all(
        socket,
        br#""><label for="password">Wi-Fi Password</label><input id="password" name="password" type="password" maxlength="64" autocomplete="current-password"><button type="submit">Save and reconnect</button></form></main></body></html>"#,
    )
    .await;
}

async fn write_text_response(socket: &mut TcpSocket<'_>, code: u16, reason: &str, body: &str) {
    match code {
        400 => write_all(socket, b"HTTP/1.0 400 Bad Request\r\n").await,
        404 => write_all(socket, b"HTTP/1.0 404 Not Found\r\n").await,
        _ => {
            write_all(socket, b"HTTP/1.0 ").await;
            write_all(socket, reason.as_bytes()).await;
            write_all(socket, b"\r\n").await;
        }
    }
    write_all(
        socket,
        b"Content-Type: text/plain\r\nConnection: close\r\n\r\n",
    )
    .await;
    write_all(socket, body.as_bytes()).await;
}

async fn write_escaped(socket: &mut TcpSocket<'_>, value: &str) {
    for byte in value.bytes() {
        match byte {
            b'&' => write_all(socket, b"&amp;").await,
            b'<' => write_all(socket, b"&lt;").await,
            b'>' => write_all(socket, b"&gt;").await,
            b'"' => write_all(socket, b"&quot;").await,
            b'\'' => write_all(socket, b"&#39;").await,
            _ => {
                let single = [byte];
                write_all(socket, &single).await;
            }
        }
    }
}

async fn write_all(socket: &mut TcpSocket<'_>, data: &[u8]) {
    if let Err(e) = socket.write_all(data).await {
        esp32::log_warn!("HTTP write error: {:?}", e);
    }
}

async fn write_display<T: core::fmt::Display>(socket: &mut TcpSocket<'_>, value: T) {
    let mut buffer = String::<64>::new();
    let _ = write!(buffer, "{}", value);
    write_all(socket, buffer.as_bytes()).await;
}
