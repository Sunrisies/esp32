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
use esp_println::{print, println};
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
        println!("Wait for connection...");

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
        println!("Connected...");

        if let Err(e) = accept_result {
            println!("connect error: {:?}", e);
            continue;
        }

        read_request(server_socket).await;

        if sta_stack.is_link_up() {
            proxy_httpbin(server_socket, &tcp_client, &dns_client).await;
        } else {
            write_station_down_response(server_socket).await;
        }

        if let Err(e) = server_socket.flush().await {
            println!("AP flush error: {:?}", e);
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
            println!("HTTP request header too large");
            break;
        }

        match socket.read(&mut buffer[pos..]).await {
            Ok(0) => {
                println!("AP read EOF");
                break;
            }
            Ok(len) => {
                pos += len;
                match core::str::from_utf8(&buffer[..pos]) {
                    Ok(request) if request.contains("\r\n\r\n") => {
                        print!("{}", request);
                        println!();
                        break;
                    }
                    Ok(_) => {}
                    Err(_) if pos == buffer.len() => {
                        println!("HTTP request is not valid UTF-8");
                        break;
                    }
                    Err(_) => {}
                }
            }
            Err(e) => {
                println!("AP read error: {:?}", e);
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
    println!("connecting via HttpClient...");
    let mut client = HttpClient::new(tcp_client, dns_client);
    let mut rx_buf = [0u8; 4096];

    let builder_result = client.request(Method::GET, REQUEST_URL).await;

    match builder_result {
        Ok(req_builder) => {
            let mut req_builder = req_builder.headers(&REQUEST_HEADERS);
            let response_result = req_builder.send(&mut rx_buf).await;

            match response_result {
                Ok(response) => {
                    println!("HTTP request successful, streaming body...");

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
                                    println!("AP write error: {:?}", e);
                                    break;
                                }
                            }
                            Err(e) => {
                                println!("Body read error: {:?}", e);
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    println!("Station request error: {:?}", e);
                    let _ = server_socket
                        .write_all(b"HTTP/1.0 500 Internal Server Error\r\n\r\nRequest failed")
                        .await;
                }
            }
        }
        Err(e) => {
            println!("DNS/Connect error: {:?}", e);
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
        println!("AP write error: {:?}", e);
    }
}
