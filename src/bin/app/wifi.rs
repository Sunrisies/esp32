use core::cell::RefCell;

use embassy_futures::select::{Either, Either3, select, select3};
use embassy_net::{
    IpAddress, Ipv4Address, Ipv4Cidr, Runner, Stack, StaticConfigV4,
    udp::{PacketMetadata, UdpSocket},
};
use embassy_sync::{
    blocking_mutex::{Mutex, raw::CriticalSectionRawMutex},
    signal::Signal,
};
use embassy_time::{Duration, Timer};
use esp_hal::rng::Rng;
use esp_radio::wifi::{
    AccessPointStationEventInfo, Config, ControllerConfig, Interface, WifiController,
    ap::AccessPointConfig, sta::StationConfig,
};
use esp32::config::{WIFI_AP_IP, WIFI_AP_SSID, WIFI_PASSWORD, WIFI_SSID};
use heapless::{String, Vec};

pub const MAX_WIFI_SSID_LEN: usize = 32;
pub const MAX_WIFI_PASSWORD_LEN: usize = 64;

const PLACEHOLDER_WIFI_SSID: &str = "CHANGE_ME_WIFI_SSID";
const PLACEHOLDER_WIFI_PASSWORD: &str = "CHANGE_ME_WIFI_PASSWORD";
const DHCP_SERVER_PORT: u16 = 67;
const DHCP_CLIENT_PORT: u16 = 68;
const DHCP_PACKET_BUFFER_SIZE: usize = 768;
const DHCP_MAX_LEASES: usize = 8;
const DHCP_FIXED_HEADER_LEN: usize = 236;
const DHCP_OPTIONS_OFFSET: usize = 240;
const DHCP_MIN_RESPONSE_LEN: usize = 300;
const DHCP_LEASE_SECONDS: u32 = 3600;
const DHCP_MAGIC_COOKIE: [u8; 4] = [99, 130, 83, 99];
const DHCP_DISCOVER: u8 = 1;
const DHCP_OFFER: u8 = 2;
const DHCP_REQUEST: u8 = 3;
const DHCP_ACK: u8 = 5;
const DHCP_OPTION_PAD: u8 = 0;
const DHCP_OPTION_SUBNET_MASK: u8 = 1;
const DHCP_OPTION_ROUTER: u8 = 3;
const DHCP_OPTION_DNS: u8 = 6;
const DHCP_OPTION_REQUESTED_IP: u8 = 50;
const DHCP_OPTION_LEASE_TIME: u8 = 51;
const DHCP_OPTION_MESSAGE_TYPE: u8 = 53;
const DHCP_OPTION_SERVER_ID: u8 = 54;
const DHCP_OPTION_RENEWAL_TIME: u8 = 58;
const DHCP_OPTION_REBINDING_TIME: u8 = 59;
const DHCP_OPTION_BROADCAST_ADDRESS: u8 = 28;
const DHCP_OPTION_END: u8 = 255;

static WIFI_CONFIG_SIGNAL: Signal<CriticalSectionRawMutex, WifiCredentials> = Signal::new();
static WIFI_STATE: Mutex<CriticalSectionRawMutex, RefCell<WifiRuntimeState>> =
    Mutex::new(RefCell::new(WifiRuntimeState { credentials: None }));

#[derive(Clone)]
pub struct WifiCredentials {
    ssid: String<MAX_WIFI_SSID_LEN>,
    password: String<MAX_WIFI_PASSWORD_LEN>,
}

#[derive(Clone, Copy, Debug)]
pub enum WifiConfigError {
    EmptySsid,
    SsidTooLong,
    PasswordTooLong,
}

struct WifiRuntimeState {
    credentials: Option<WifiCredentials>,
}

enum ConnectOutcome {
    Connected,
    Failed,
    Updated(WifiCredentials),
}

enum MonitorOutcome {
    Disconnected,
    Updated(WifiCredentials),
}

#[derive(Clone, Copy)]
struct DhcpRequest {
    message_type: u8,
    xid: [u8; 4],
    flags: [u8; 2],
    chaddr: [u8; 16],
    client_mac: [u8; 6],
    requested_ip: Option<Ipv4Address>,
}

#[derive(Clone, Copy)]
struct DhcpLease {
    mac: [u8; 6],
    ip: Ipv4Address,
}

impl WifiCredentials {
    pub fn new(ssid: &str, password: &str) -> Result<Self, WifiConfigError> {
        if ssid.is_empty() {
            return Err(WifiConfigError::EmptySsid);
        }

        let mut owned_ssid = String::new();
        owned_ssid
            .push_str(ssid)
            .map_err(|_| WifiConfigError::SsidTooLong)?;

        let mut owned_password = String::new();
        owned_password
            .push_str(password)
            .map_err(|_| WifiConfigError::PasswordTooLong)?;

        Ok(Self {
            ssid: owned_ssid,
            password: owned_password,
        })
    }

    pub fn ssid(&self) -> &str {
        self.ssid.as_str()
    }

    fn password(&self) -> &str {
        self.password.as_str()
    }
}

pub fn current_credentials() -> Option<WifiCredentials> {
    WIFI_STATE.lock(|state| state.borrow().credentials.clone())
}

pub fn request_station_credentials(ssid: &str, password: &str) -> Result<(), WifiConfigError> {
    let credentials = WifiCredentials::new(ssid, password)?;
    set_current_credentials(Some(credentials.clone()));
    WIFI_CONFIG_SIGNAL.signal(credentials);
    Ok(())
}

pub fn start(
    wifi: esp_hal::peripherals::WIFI<'static>,
) -> (
    WifiController<'static>,
    Interface<'static>,
    Interface<'static>,
) {
    let credentials = build_time_credentials();
    set_current_credentials(credentials.clone());

    let access_point_station_config =
        Config::AccessPointStation(station_config(credentials.as_ref()), access_point_config());

    esp32::log_info!("Starting wifi");
    let (controller, interfaces) = esp_radio::wifi::new(
        wifi,
        ControllerConfig::default().with_initial_config(access_point_station_config),
    )
    .unwrap();
    esp32::log_info!("Wifi started!");

    (controller, interfaces.access_point, interfaces.station)
}

pub fn network_configs() -> (embassy_net::Config, embassy_net::Config) {
    let ap_config = embassy_net::Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(WIFI_AP_IP, 24),
        gateway: Some(WIFI_AP_IP),
        dns_servers: Default::default(),
    });
    let sta_config = embassy_net::Config::dhcpv4(Default::default());

    (ap_config, sta_config)
}

pub fn random_seed() -> u64 {
    let rng = Rng::new();
    (rng.random() as u64) << 32 | rng.random() as u64
}

pub async fn wait_for_access_point(ap_stack: Stack<'static>, sta_stack: Stack<'static>) {
    ap_stack.wait_config_up().await;

    esp32::log_info!(
        "Connect to the AP `{}` and open http://{}:8080/ after DHCP assigns your device an IP",
        WIFI_AP_SSID,
        WIFI_AP_IP
    );

    if let Some(config) = sta_stack.config_v4() {
        esp32::log_info!(
            "STA already has IP {}; local HTTP is also available there on port 8080",
            config.address.address()
        );
    } else {
        esp32::log_info!("STA is not connected yet; local HTTP is available through the AP");
    }
}

#[embassy_executor::task]
pub async fn connection(mut controller: WifiController<'static>) {
    esp32::log_info!("start connection task");
    let mut credentials = current_credentials();

    loop {
        let Some(active_credentials) = credentials.clone() else {
            esp32::log_info!("No STA Wi-Fi credentials configured; waiting for local setup");
            let updated = WIFI_CONFIG_SIGNAL.wait().await;
            set_current_credentials(Some(updated.clone()));
            credentials = Some(updated);
            continue;
        };

        if let Err(e) =
            apply_access_point_station_config(&mut controller, Some(&active_credentials))
        {
            esp32::log_warn!("Failed to apply Wi-Fi config: {:?}", e);
            credentials = wait_retry_or_update().await.or(credentials);
            continue;
        }

        match connect_or_update(&mut controller).await {
            ConnectOutcome::Connected => {
                esp32::log_info!("Wi-Fi connected; starting RSSI monitor.");
                match monitor_connection(&mut controller).await {
                    MonitorOutcome::Disconnected => {
                        esp32::log_warn!("STA disconnected; retrying Wi-Fi connection");
                    }
                    MonitorOutcome::Updated(updated) => {
                        esp32::log_info!("Received updated STA Wi-Fi credentials");
                        set_current_credentials(Some(updated.clone()));
                        let _ = controller.disconnect_async().await;
                        credentials = Some(updated);
                    }
                }
            }
            ConnectOutcome::Failed => {
                credentials = wait_retry_or_update().await.or(credentials);
            }
            ConnectOutcome::Updated(updated) => {
                esp32::log_info!("Received updated STA Wi-Fi credentials");
                set_current_credentials(Some(updated.clone()));
                let _ = controller.disconnect_async().await;
                credentials = Some(updated);
            }
        }
    }
}

#[embassy_executor::task(pool_size = 2)]
pub async fn net_task(mut runner: Runner<'static, Interface<'static>>) {
    runner.run().await
}

#[embassy_executor::task]
pub async fn dhcp_server_task(ap_stack: Stack<'static>) -> ! {
    ap_stack.wait_config_up().await;

    let mut rx_meta = [PacketMetadata::EMPTY; 4];
    let mut tx_meta = [PacketMetadata::EMPTY; 4];
    let mut rx_buffer = [0; DHCP_PACKET_BUFFER_SIZE];
    let mut tx_buffer = [0; DHCP_PACKET_BUFFER_SIZE];
    let mut response_buffer = [0; DHCP_PACKET_BUFFER_SIZE];
    let mut leases = Vec::<DhcpLease, DHCP_MAX_LEASES>::new();
    let mut socket = UdpSocket::new(
        ap_stack,
        &mut rx_meta,
        &mut rx_buffer,
        &mut tx_meta,
        &mut tx_buffer,
    );

    if let Err(e) = socket.bind(DHCP_SERVER_PORT) {
        esp32::log_error!("Failed to bind AP DHCP server: {:?}", e);
        loop {
            Timer::after(Duration::from_secs(60)).await;
        }
    }

    esp32::log_info!(
        "AP DHCP server ready on {}:{}",
        WIFI_AP_IP,
        DHCP_SERVER_PORT
    );

    loop {
        let request = socket
            .recv_from_with(|packet, _metadata| parse_dhcp_request(packet))
            .await;
        let Some(request) = request else {
            continue;
        };

        let client_ip = lease_client_ip(&mut leases, &request);
        let Some((response_len, response_type)) =
            write_dhcp_response(&mut response_buffer, &request, client_ip)
        else {
            esp32::log_warn!("Failed to build DHCP response");
            continue;
        };

        let remote = (
            IpAddress::Ipv4(Ipv4Address::new(255, 255, 255, 255)),
            DHCP_CLIENT_PORT,
        );
        match socket
            .send_to(&response_buffer[..response_len], remote)
            .await
        {
            Ok(()) => esp32::log_info!(
                "DHCP {} {:?} -> {}",
                dhcp_message_name(response_type),
                request.client_mac,
                client_ip
            ),
            Err(e) => esp32::log_warn!("Failed to send DHCP response: {:?}", e),
        }
    }
}

fn parse_dhcp_request(packet: &[u8]) -> Option<DhcpRequest> {
    if packet.len() < DHCP_OPTIONS_OFFSET {
        return None;
    }
    if packet[0] != 1 || packet[1] != 1 || packet[2] < 6 {
        return None;
    }
    if &packet[DHCP_FIXED_HEADER_LEN..DHCP_OPTIONS_OFFSET] != DHCP_MAGIC_COOKIE.as_slice() {
        return None;
    }

    let mut message_type = None;
    let mut requested_ip = None;
    let mut server_id = None;
    let mut offset = DHCP_OPTIONS_OFFSET;

    while offset < packet.len() {
        let code = packet[offset];
        offset += 1;

        match code {
            DHCP_OPTION_PAD => {}
            DHCP_OPTION_END => break,
            _ => {
                if offset >= packet.len() {
                    return None;
                }

                let len = packet[offset] as usize;
                offset += 1;
                if offset + len > packet.len() {
                    return None;
                }

                let data = &packet[offset..offset + len];
                match code {
                    DHCP_OPTION_MESSAGE_TYPE if len == 1 => message_type = Some(data[0]),
                    DHCP_OPTION_REQUESTED_IP if len == 4 => {
                        requested_ip = Some(Ipv4Address::new(data[0], data[1], data[2], data[3]));
                    }
                    DHCP_OPTION_SERVER_ID if len == 4 => {
                        server_id = Some(Ipv4Address::new(data[0], data[1], data[2], data[3]));
                    }
                    _ => {}
                }
                offset += len;
            }
        }
    }

    let message_type = message_type?;
    if message_type != DHCP_DISCOVER && message_type != DHCP_REQUEST {
        return None;
    }
    if let Some(server_id) = server_id {
        if server_id != WIFI_AP_IP {
            return None;
        }
    }

    let mut xid = [0; 4];
    xid.copy_from_slice(&packet[4..8]);
    let mut flags = [0; 2];
    flags.copy_from_slice(&packet[10..12]);
    let mut chaddr = [0; 16];
    chaddr.copy_from_slice(&packet[28..44]);
    let mut client_mac = [0; 6];
    client_mac.copy_from_slice(&packet[28..34]);

    Some(DhcpRequest {
        message_type,
        xid,
        flags,
        chaddr,
        client_mac,
        requested_ip,
    })
}

fn lease_client_ip(
    leases: &mut Vec<DhcpLease, DHCP_MAX_LEASES>,
    request: &DhcpRequest,
) -> Ipv4Address {
    if let Some(lease) = leases.iter().find(|lease| lease.mac == request.client_mac) {
        return lease.ip;
    }

    let mut ip = match request.requested_ip {
        Some(requested_ip) if is_ap_client_ip(requested_ip) => requested_ip,
        _ => suggested_client_ip(request.client_mac),
    };

    if leases.iter().any(|lease| lease.ip == ip) {
        ip = next_available_client_ip(leases.as_slice())
            .unwrap_or_else(|| suggested_client_ip(request.client_mac));
    }

    let _ = leases.push(DhcpLease {
        mac: request.client_mac,
        ip,
    });
    ip
}

fn next_available_client_ip(leases: &[DhcpLease]) -> Option<Ipv4Address> {
    let server = WIFI_AP_IP.octets();
    for last in 100u8..200u8 {
        if last == server[3] {
            continue;
        }

        let candidate = Ipv4Address::new(server[0], server[1], server[2], last);
        if !leases.iter().any(|lease| lease.ip == candidate) {
            return Some(candidate);
        }
    }
    None
}

fn suggested_client_ip(mac: [u8; 6]) -> Ipv4Address {
    let server = WIFI_AP_IP.octets();
    let mut hash = 0u8;
    for byte in mac {
        hash = hash.wrapping_mul(33).wrapping_add(byte);
    }

    let mut last = 100u8 + (hash % 100);
    if last == server[3] {
        last = if last == 199 { 100 } else { last + 1 };
    }

    Ipv4Address::new(server[0], server[1], server[2], last)
}

fn is_ap_client_ip(ip: Ipv4Address) -> bool {
    let server = WIFI_AP_IP.octets();
    let octets = ip.octets();

    octets[0] == server[0]
        && octets[1] == server[1]
        && octets[2] == server[2]
        && octets[3] != server[3]
        && octets[3] != 0
        && octets[3] != 255
}

fn write_dhcp_response(
    buffer: &mut [u8],
    request: &DhcpRequest,
    client_ip: Ipv4Address,
) -> Option<(usize, u8)> {
    if buffer.len() < DHCP_MIN_RESPONSE_LEN {
        return None;
    }

    buffer[..DHCP_MIN_RESPONSE_LEN].fill(0);

    let response_type = match request.message_type {
        DHCP_DISCOVER => DHCP_OFFER,
        DHCP_REQUEST => DHCP_ACK,
        _ => return None,
    };

    let server_ip = WIFI_AP_IP.octets();
    let client_ip = client_ip.octets();

    buffer[0] = 2;
    buffer[1] = 1;
    buffer[2] = 6;
    buffer[4..8].copy_from_slice(&request.xid);
    buffer[10..12].copy_from_slice(&request.flags);
    buffer[16..20].copy_from_slice(&client_ip);
    buffer[20..24].copy_from_slice(&server_ip);
    buffer[28..44].copy_from_slice(&request.chaddr);
    buffer[DHCP_FIXED_HEADER_LEN..DHCP_OPTIONS_OFFSET].copy_from_slice(&DHCP_MAGIC_COOKIE);

    let mut offset = DHCP_OPTIONS_OFFSET;
    push_dhcp_option(
        buffer,
        &mut offset,
        DHCP_OPTION_MESSAGE_TYPE,
        &[response_type],
    )?;
    push_dhcp_option(buffer, &mut offset, DHCP_OPTION_SERVER_ID, &server_ip)?;
    push_dhcp_option(
        buffer,
        &mut offset,
        DHCP_OPTION_LEASE_TIME,
        &DHCP_LEASE_SECONDS.to_be_bytes(),
    )?;
    push_dhcp_option(
        buffer,
        &mut offset,
        DHCP_OPTION_RENEWAL_TIME,
        &(DHCP_LEASE_SECONDS / 2).to_be_bytes(),
    )?;
    push_dhcp_option(
        buffer,
        &mut offset,
        DHCP_OPTION_REBINDING_TIME,
        &((DHCP_LEASE_SECONDS * 7) / 8).to_be_bytes(),
    )?;
    push_dhcp_option(
        buffer,
        &mut offset,
        DHCP_OPTION_SUBNET_MASK,
        &[255, 255, 255, 0],
    )?;
    push_dhcp_option(buffer, &mut offset, DHCP_OPTION_ROUTER, &server_ip)?;
    push_dhcp_option(buffer, &mut offset, DHCP_OPTION_DNS, &server_ip)?;

    let broadcast_ip = Ipv4Address::new(server_ip[0], server_ip[1], server_ip[2], 255).octets();
    push_dhcp_option(
        buffer,
        &mut offset,
        DHCP_OPTION_BROADCAST_ADDRESS,
        &broadcast_ip,
    )?;

    if offset >= buffer.len() {
        return None;
    }
    buffer[offset] = DHCP_OPTION_END;
    offset += 1;

    Some((offset.max(DHCP_MIN_RESPONSE_LEN), response_type))
}

fn push_dhcp_option(buffer: &mut [u8], offset: &mut usize, code: u8, value: &[u8]) -> Option<()> {
    if value.len() > u8::MAX as usize || *offset + 2 + value.len() > buffer.len() {
        return None;
    }

    buffer[*offset] = code;
    buffer[*offset + 1] = value.len() as u8;
    *offset += 2;
    buffer[*offset..*offset + value.len()].copy_from_slice(value);
    *offset += value.len();
    Some(())
}

fn dhcp_message_name(message_type: u8) -> &'static str {
    match message_type {
        DHCP_OFFER => "offer",
        DHCP_ACK => "ack",
        _ => "response",
    }
}

fn build_time_credentials() -> Option<WifiCredentials> {
    if WIFI_SSID.is_empty() || WIFI_SSID == PLACEHOLDER_WIFI_SSID {
        return None;
    }

    let password = if WIFI_PASSWORD == PLACEHOLDER_WIFI_PASSWORD {
        ""
    } else {
        WIFI_PASSWORD
    };

    WifiCredentials::new(WIFI_SSID, password).ok()
}

fn set_current_credentials(credentials: Option<WifiCredentials>) {
    WIFI_STATE.lock(|state| {
        state.borrow_mut().credentials = credentials;
    });
}

fn station_config(credentials: Option<&WifiCredentials>) -> StationConfig {
    match credentials {
        Some(credentials) => StationConfig::default()
            .with_ssid(credentials.ssid())
            .with_password(credentials.password().into()),
        None => StationConfig::default(),
    }
}

fn access_point_config() -> AccessPointConfig {
    AccessPointConfig::default().with_ssid(WIFI_AP_SSID)
}

fn apply_access_point_station_config(
    controller: &mut WifiController<'static>,
    credentials: Option<&WifiCredentials>,
) -> Result<(), esp_radio::wifi::WifiError> {
    let config = Config::AccessPointStation(station_config(credentials), access_point_config());
    controller.set_config(&config)
}

async fn connect_or_update(controller: &mut WifiController<'static>) -> ConnectOutcome {
    match select(controller.connect_async(), WIFI_CONFIG_SIGNAL.wait()).await {
        Either::First(Ok(_)) => ConnectOutcome::Connected,
        Either::First(Err(e)) => {
            esp32::log_warn!("Failed to connect to wifi: {:?}", e);
            ConnectOutcome::Failed
        }
        Either::Second(credentials) => ConnectOutcome::Updated(credentials),
    }
}

async fn monitor_connection(controller: &mut WifiController<'static>) -> MonitorOutcome {
    let mut ticker = embassy_time::Ticker::every(Duration::from_secs(10));

    loop {
        let monitor = select3(
            controller.wait_for_disconnect_async(),
            controller.wait_for_access_point_connected_event_async(),
            ticker.next(),
        );

        match select(monitor, WIFI_CONFIG_SIGNAL.wait()).await {
            Either::First(Either3::First(station_disconnected)) => {
                if let Ok(info) = station_disconnected {
                    esp32::log_warn!("Station disconnected: {:?}", info);
                }
                return MonitorOutcome::Disconnected;
            }
            Either::First(Either3::Second(event)) => {
                if let Ok(event) = event {
                    match event {
                        AccessPointStationEventInfo::Connected(info) => {
                            esp32::log_info!("Station connected: {:?}", info);
                        }
                        AccessPointStationEventInfo::Disconnected(info) => {
                            esp32::log_warn!("Station disconnected: {:?}", info);
                        }
                    }
                }
            }
            Either::First(Either3::Third(_)) => match controller.rssi() {
                Ok(rssi) => esp32::log_info!("Current WiFi RSSI: {} dBm", rssi),
                Err(e) => esp32::log_warn!("Failed to get RSSI: {:?}", e),
            },
            Either::Second(credentials) => return MonitorOutcome::Updated(credentials),
        }
    }
}

async fn wait_retry_or_update() -> Option<WifiCredentials> {
    match select(
        Timer::after(Duration::from_millis(20000)),
        WIFI_CONFIG_SIGNAL.wait(),
    )
    .await
    {
        Either::First(()) => None,
        Either::Second(credentials) => Some(credentials),
    }
}
