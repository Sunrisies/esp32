use embassy_futures::select::Either;
use embassy_net::{
    IpListenEndpoint, Stack,
    dns::DnsSocket,
    tcp::{
        TcpSocket,
        client::{TcpClient, TcpClientState},
    },
};
use embassy_time::{Duration, Timer};
use embedded_io_async::{Read, Write};
use reqwless::{
    client::HttpClient,
    request::{Method, RequestBuilder},
};

const HTTP_PORT: u16 = 8080;
const REQUEST_URL: &str = "http://httpbin.org/get?hello=Hello+esp-hal";
const REQUEST_HEADERS: [(&str, &str); 2] = [("Host", "httpbin.org"), ("Connection", "close")];

pub async fn serve(ap_stack: Stack<'static>, sta_stack: Stack<'static>) -> ! {
    let tcp_client = TcpClient::new(sta_stack, tcp_client_state());
    let dns_client = DnsSocket::new(sta_stack);

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
        esp32::log_info!("Wait for connection...");

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
        let (accept_result, server_socket) = match either_socket {
            Either::First(result) => (result, &mut ap_server_socket),
            Either::Second(result) => (result, &mut sta_server_socket),
        };
        esp32::log_info!("Connected...");

        if let Err(e) = accept_result {
            esp32::log_warn!("connect error: {:?}", e);
            continue;
        }

        read_request(server_socket).await;

        if sta_stack.is_link_up() {
            proxy_httpbin(server_socket, &tcp_client, &dns_client).await;
        } else {
            write_station_down_response(server_socket).await;
        }

        if let Err(e) = server_socket.flush().await {
            esp32::log_warn!("AP flush error: {:?}", e);
        }

        Timer::after(Duration::from_millis(1000)).await;
        server_socket.close();
        Timer::after(Duration::from_millis(1000)).await;
        server_socket.abort();
    }
}

fn tcp_client_state() -> &'static mut TcpClientState<1, 1500, 1500> {
    static TCP_CLIENT_STATE: static_cell::StaticCell<TcpClientState<1, 1500, 1500>> =
        static_cell::StaticCell::new();
    TCP_CLIENT_STATE
        .uninit()
        .write(TcpClientState::<1, 1500, 1500>::new())
}

async fn read_request(socket: &mut TcpSocket<'_>) {
    let mut buffer = [0u8; 1024];
    let mut pos = 0;

    loop {
        if pos >= buffer.len() {
            esp32::log_warn!("HTTP request header too large");
            break;
        }

        match socket.read(&mut buffer[pos..]).await {
            Ok(0) => {
                esp32::log_warn!("AP read EOF");
                break;
            }
            Ok(len) => {
                pos += len;
                match core::str::from_utf8(&buffer[..pos]) {
                    Ok(request) if request.contains("\r\n\r\n") => {
                        esp32::protocol_log!("HTTP request:\n{}", request);
                        break;
                    }
                    Ok(_) => {}
                    Err(_) if pos == buffer.len() => {
                        esp32::log_warn!("HTTP request is not valid UTF-8");
                        break;
                    }
                    Err(_) => {}
                }
            }
            Err(e) => {
                esp32::log_warn!("AP read error: {:?}", e);
                break;
            }
        };
    }
}

async fn proxy_httpbin(
    server_socket: &mut TcpSocket<'_>,
    tcp_client: &TcpClient<'_, 1, 1500, 1500>,
    dns_client: &DnsSocket<'_>,
) {
    esp32::log_info!("connecting via HttpClient...");
    let mut client = HttpClient::new(tcp_client, dns_client);
    let mut rx_buf = [0u8; 4096];

    let builder_result = client.request(Method::GET, REQUEST_URL).await;

    match builder_result {
        Ok(req_builder) => {
            let mut req_builder = req_builder.headers(&REQUEST_HEADERS);
            let response_result = req_builder.send(&mut rx_buf).await;

            match response_result {
                Ok(response) => {
                    esp32::log_info!("HTTP request successful, streaming body...");

                    let _ = server_socket.write_all(b"HTTP/1.0 200 OK\r\n").await;
                    let _ = server_socket
                        .write_all(b"Content-Type: application/json\r\n")
                        .await;
                    let _ = server_socket.write_all(b"Connection: close\r\n\r\n").await;

                    let mut body_reader = response.body().reader();
                    let mut chunk_buf = [0u8; 1024];

                    loop {
                        match body_reader.read(&mut chunk_buf).await {
                            Ok(0) => break,
                            Ok(n) => {
                                if let Err(e) = server_socket.write_all(&chunk_buf[..n]).await {
                                    esp32::log_warn!("AP write error: {:?}", e);
                                    break;
                                }
                            }
                            Err(e) => {
                                esp32::log_warn!("Body read error: {:?}", e);
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    esp32::log_warn!("Station request error: {:?}", e);
                    let _ = server_socket
                        .write_all(b"HTTP/1.0 500 Internal Server Error\r\n\r\nRequest failed")
                        .await;
                }
            }
        }
        Err(e) => {
            esp32::log_warn!("DNS/Connect error: {:?}", e);
            let _ = server_socket
                .write_all(b"HTTP/1.0 500 Internal Server Error\r\n\r\nDNS Error")
                .await;
        }
    }
}

async fn write_station_down_response(server_socket: &mut TcpSocket<'_>) {
    let result = server_socket
        .write_all(
            b"HTTP/1.0 200 OK\r\n\r\n\
            <html>\
                <body>\
                    <h1>Hello Rust! Hello esp-radio! Station is not connected.</h1>\
                </body>\
            </html>\r\n",
        )
        .await;

    if let Err(e) = result {
        esp32::log_warn!("AP write error: {:?}", e);
    }
}
