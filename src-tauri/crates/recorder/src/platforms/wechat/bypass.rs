//! Lets the capture proxy work while a TUN-mode tool (Quantumult X, Shadowrocket, Clash, a VPN)
//! owns the default route.
//!
//! In rule mode the user's tool forwards Tencent hosts to the capture proxy on a fixed port. If the
//! proxy then connected upstream through the default route, the tunnel would hand the connection
//! straight back to it. So when the default route is a virtual interface, upstream connections are
//! pinned to the physical interface (`IP_BOUND_IF`), and names are resolved by asking a DNS server
//! over that interface, because the system resolver returns the tunnel's fake addresses.
use hudsucker::hyper_util::client::legacy::connect::{dns::Name, HttpConnector};
use std::{
    future::Future,
    net::{Ipv4Addr, SocketAddr},
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

/// Port the user's tool forwards to. Fixed so that their rule keeps working across runs.
pub const RULES_PORT: u16 = 17901;
const POLICY: &str = "LIVE视频号";
const SERVER: &str = "LIVE抓取";
/// What QX, Shadowrocket and Clash use to test a proxy. The capture proxy answers it itself, so the
/// fallback to direct depends only on whether LIVE is listening.
pub const CHECK_URL: &str = "http://cp.cloudflare.com/generate_204";

/// The physical interface to pin upstream connections to, when a virtual one has the default route.
#[derive(Clone, Debug, PartialEq)]
pub struct Bypass {
    pub interface: String,
    pub dns_servers: Vec<Ipv4Addr>,
}

const PUBLIC_DNS: [Ipv4Addr; 2] = [Ipv4Addr::new(223, 5, 5, 5), Ipv4Addr::new(119, 29, 29, 29)];

pub fn is_virtual_interface(name: &str) -> bool {
    ["utun", "ipsec", "ppp", "tun", "tap"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

/// `networksetup -listnetworkserviceorder`: devices in the user's service order.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_service_devices(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| line.split("Device: ").nth(1))
        .map(|rest| rest.trim_end_matches(')').trim().to_string())
        .filter(|device| !device.is_empty() && !is_virtual_interface(device))
        .collect()
}

#[cfg(target_os = "macos")]
fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(program).args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// The interface of the default route, e.g. `en0` or a tunnel's `utun6`.
#[cfg(target_os = "macos")]
pub fn default_route_interface() -> Option<String> {
    command_output("route", &["-n", "get", "default"])?
        .lines()
        .find_map(|line| line.trim().strip_prefix("interface:"))
        .map(|name| name.trim().to_string())
}

#[cfg(not(target_os = "macos"))]
pub fn default_route_interface() -> Option<String> {
    None
}

/// `None` when the default route is already physical: nothing needs to be bypassed.
#[cfg(target_os = "macos")]
pub fn detect() -> Option<Bypass> {
    if !is_virtual_interface(&default_route_interface()?) {
        return None;
    }
    let order = command_output("networksetup", &["-listnetworkserviceorder"])?;
    let interface = parse_service_devices(&order)
        .into_iter()
        .find(|device| command_output("ipconfig", &["getifaddr", device]).is_some_and(|ip| !ip.is_empty()))?;
    let mut dns_servers: Vec<Ipv4Addr> =
        command_output("ipconfig", &["getoption", &interface, "domain_name_server"])
            .and_then(|text| text.parse().ok())
            .into_iter()
            .collect();
    dns_servers.extend(PUBLIC_DNS);
    Some(Bypass {
        interface,
        dns_servers,
    })
}

#[cfg(not(target_os = "macos"))]
pub fn detect() -> Option<Bypass> {
    None
}

// --- DNS over the physical interface ------------------------------------------------------------

fn encode_query(id: u16, name: &str) -> Option<Vec<u8>> {
    let mut packet = Vec::with_capacity(32 + name.len());
    packet.extend_from_slice(&id.to_be_bytes());
    packet.extend_from_slice(&[0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0]);
    for label in name.trim_end_matches('.').split('.') {
        if label.is_empty() || label.len() > 63 {
            return None;
        }
        packet.push(label.len() as u8);
        packet.extend_from_slice(label.as_bytes());
    }
    packet.extend_from_slice(&[0, 0, 1, 0, 1]);
    Some(packet)
}

/// Skips a possibly compressed name, returning the offset just after it.
fn skip_name(packet: &[u8], mut pos: usize) -> Option<usize> {
    loop {
        let len = *packet.get(pos)?;
        if len & 0xC0 == 0xC0 {
            return Some(pos + 2);
        }
        if len == 0 {
            return Some(pos + 1);
        }
        pos += 1 + len as usize;
    }
}

/// The A records of a response to query `id`; CNAME and other records are skipped.
fn parse_a_records(packet: &[u8], id: u16) -> Option<Vec<Ipv4Addr>> {
    if packet.len() < 12 || u16::from_be_bytes([packet[0], packet[1]]) != id {
        return None;
    }
    if packet[3] & 0x0F != 0 {
        return Some(Vec::new());
    }
    let questions = u16::from_be_bytes([packet[4], packet[5]]);
    let answers = u16::from_be_bytes([packet[6], packet[7]]);
    let mut pos = 12;
    for _ in 0..questions {
        pos = skip_name(packet, pos)? + 4;
    }
    let mut found = Vec::new();
    for _ in 0..answers {
        pos = skip_name(packet, pos)?;
        let header = packet.get(pos..pos + 10)?;
        let kind = u16::from_be_bytes([header[0], header[1]]);
        let length = u16::from_be_bytes([header[8], header[9]]) as usize;
        pos += 10;
        let data = packet.get(pos..pos + length)?;
        if kind == 1 && length == 4 {
            found.push(Ipv4Addr::new(data[0], data[1], data[2], data[3]));
        }
        pos += length;
    }
    Some(found)
}

#[cfg(target_os = "macos")]
fn bound_udp_socket(interface: &str) -> std::io::Result<tokio::net::UdpSocket> {
    let index = std::ffi::CString::new(interface)
        .ok()
        .map(|name| unsafe { libc_if_nametoindex(name.as_ptr()) })
        .filter(|index| *index != 0)
        .ok_or_else(|| std::io::Error::other(format!("找不到网卡 {interface}")))?;
    let socket = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::DGRAM, None)?;
    socket.bind_device_by_index_v4(std::num::NonZeroU32::new(index))?;
    socket.set_nonblocking(true)?;
    socket.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)).into())?;
    tokio::net::UdpSocket::from_std(socket.into())
}

#[cfg(target_os = "macos")]
extern "C" {
    #[link_name = "if_nametoindex"]
    fn libc_if_nametoindex(name: *const std::ffi::c_char) -> u32;
}

/// Asks each server in turn over the physical interface; the first non-empty answer wins.
#[cfg(target_os = "macos")]
async fn resolve_via(bypass: &Bypass, name: &str) -> std::io::Result<Vec<Ipv4Addr>> {
    let mut last_error = std::io::Error::other(format!("无法解析 {name}"));
    for server in &bypass.dns_servers {
        let id: u16 = rand::random();
        let Some(query) = encode_query(id, name) else {
            return Err(std::io::Error::other(format!("域名无效：{name}")));
        };
        let attempt = async {
            let socket = bound_udp_socket(&bypass.interface)?;
            socket.send_to(&query, (*server, 53)).await?;
            let mut buffer = [0u8; 1500];
            loop {
                let (read, _) = socket.recv_from(&mut buffer).await?;
                if let Some(addresses) = parse_a_records(&buffer[..read], id) {
                    return Ok::<_, std::io::Error>(addresses);
                }
            }
        };
        match tokio::time::timeout(Duration::from_secs(3), attempt).await {
            Ok(Ok(addresses)) if !addresses.is_empty() => return Ok(addresses),
            Ok(Ok(_)) => last_error = std::io::Error::other(format!("{name} 没有地址")),
            Ok(Err(error)) => last_error = error,
            Err(_) => last_error = std::io::Error::other(format!("解析 {name} 超时")),
        }
    }
    Err(last_error)
}

#[cfg(not(target_os = "macos"))]
async fn resolve_via(_bypass: &Bypass, name: &str) -> std::io::Result<Vec<Ipv4Addr>> {
    Err(std::io::Error::other(format!("无法解析 {name}")))
}

/// The system resolver normally; DNS over the physical interface while bypassing a tunnel.
#[derive(Clone)]
pub struct Resolver(Option<Bypass>);

type Addresses = std::vec::IntoIter<SocketAddr>;

impl tower_service::Service<Name> for Resolver {
    type Response = Addresses;
    type Error = std::io::Error;
    type Future = Pin<Box<dyn Future<Output = std::io::Result<Addresses>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, name: Name) -> Self::Future {
        let bypass = self.0.clone();
        Box::pin(async move {
            let name = name.as_str().to_string();
            // The connector fills in the port.
            let addresses: Vec<SocketAddr> = match bypass {
                Some(bypass) => resolve_via(&bypass, &name)
                    .await?
                    .into_iter()
                    .map(|ip| SocketAddr::from((ip, 0)))
                    .collect(),
                None => tokio::net::lookup_host((name.as_str(), 0)).await?.collect(),
            };
            Ok(addresses.into_iter())
        })
    }
}

/// The upstream connector: TLS with the public roots, over TCP pinned to `bypass` when given.
pub fn connector(
    bypass: Option<Bypass>,
) -> Result<hyper_rustls::HttpsConnector<HttpConnector<Resolver>>, String> {
    use hyper_rustls::ConfigBuilderExt;
    let mut http = HttpConnector::new_with_resolver(Resolver(bypass.clone()));
    http.enforce_http(false);
    #[cfg(target_os = "macos")]
    if let Some(bypass) = &bypass {
        http.set_interface(bypass.interface.clone());
    }
    let tls = hudsucker::rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
        hudsucker::rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|e| format!("无法启动本地代理：{e}"))?
    .with_webpki_roots()
    .with_no_client_auth();
    Ok(hyper_rustls::HttpsConnectorBuilder::new()
        .with_tls_config(tls)
        .https_or_http()
        .enable_http1()
        .enable_http2()
        .wrap_connector(http))
}

// --- rules for the user's tool --------------------------------------------------------------------

#[derive(Clone, Debug, serde::Serialize)]
pub struct RuleSnippets {
    pub port: u16,
    pub quantumult_x: String,
    pub shadowrocket: String,
    pub clash: String,
}

/// Rules that send the capture domains to LIVE and fall back to direct when LIVE is not capturing.
pub fn rule_snippets(suffixes: &[&str]) -> RuleSnippets {
    let port = RULES_PORT;
    let qx_rules: String = suffixes
        .iter()
        .map(|suffix| format!("host-suffix, {suffix}, {POLICY}\n"))
        .collect();
    let surge_rules: String = suffixes
        .iter()
        .map(|suffix| format!("DOMAIN-SUFFIX,{suffix},{POLICY}\n"))
        .collect();
    let clash_rules: String = suffixes
        .iter()
        .map(|suffix| format!("  - DOMAIN-SUFFIX,{suffix},{POLICY}\n"))
        .collect();
    RuleSnippets {
        port,
        quantumult_x: format!(
            "[server_local]\nhttp=127.0.0.1:{port}, tag={SERVER}\n\n[policy]\navailable={POLICY}, {SERVER}, direct\n\n[filter_local]\n# 放在其他规则的最前面\n{qx_rules}"
        ),
        shadowrocket: format!(
            "[Proxy]\n{SERVER} = http,127.0.0.1,{port}\n\n[Proxy Group]\n{POLICY} = fallback,{SERVER},DIRECT,url={CHECK_URL},interval=60,timeout=3\n\n[Rule]\n# 放在其他规则的最前面\n{surge_rules}"
        ),
        clash: format!(
            "proxies:\n  - {{name: {SERVER}, type: http, server: 127.0.0.1, port: {port}}}\n\nproxy-groups:\n  - {{name: {POLICY}, type: fallback, proxies: [{SERVER}, DIRECT], url: '{CHECK_URL}', interval: 60}}\n\nrules:\n  # 放在其他规则的最前面\n{clash_rules}"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_follow_service_order_and_skip_tunnels() {
        let text = "An asterisk (*) denotes that a network service is disabled.\n(1) Thunderbolt Bridge\n(Hardware Port: Thunderbolt Bridge, Device: bridge0)\n\n(2) Wi-Fi\n(Hardware Port: Wi-Fi, Device: en0)\n\n(3) Quantumult X 2\n(Hardware Port: com.crossutility.quantumult-x, Device: )\n\n(4) VPN\n(Hardware Port: VPN, Device: utun3)\n";
        assert_eq!(parse_service_devices(text), vec!["bridge0", "en0"]);
        assert!(is_virtual_interface("utun6"));
        assert!(!is_virtual_interface("en0"));
    }

    #[test]
    fn a_records_are_read_past_cnames_and_compression() {
        let query = encode_query(0x1234, "finder.video.qq.com").unwrap();
        let mut response = query.clone();
        response[2] = 0x81;
        response[3] = 0x80;
        response[7] = 2;
        // CNAME pointing at the question name, then an A record with a compressed name.
        response.extend_from_slice(&[0xC0, 0x0C, 0, 5, 0, 1, 0, 0, 0, 60, 0, 4, 3, b'c', b'd', b'n']);
        response.extend_from_slice(&[0xC0, 0x0C, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 1, 2, 3, 4]);
        assert_eq!(parse_a_records(&response, 0x1234), Some(vec![Ipv4Addr::new(1, 2, 3, 4)]));
        assert_eq!(parse_a_records(&response, 0x9999), None);
        let mut missing = query;
        missing[3] = 0x83;
        assert_eq!(parse_a_records(&missing, 0x1234), Some(vec![]));
        assert_eq!(encode_query(1, "bad..name"), None);
    }

    #[test]
    fn snippets_route_every_capture_domain_with_a_direct_fallback() {
        let snippets = rule_snippets(&["qq.com", "wxlivecdn.com"]);
        assert!(snippets.quantumult_x.contains("http=127.0.0.1:17901"));
        assert!(snippets.quantumult_x.contains("host-suffix, wxlivecdn.com, LIVE视频号"));
        assert!(snippets.quantumult_x.contains("available=LIVE视频号, LIVE抓取, direct"));
        assert!(snippets.shadowrocket.contains("fallback,LIVE抓取,DIRECT"));
        assert!(snippets.clash.contains("DOMAIN-SUFFIX,qq.com,LIVE视频号"));
        assert!(snippets.clash.contains("type: fallback"));
    }

    /// Needs a TUN tool on and network access: resolves and connects to Tencent around the tunnel.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore = "Needs a TUN tool running and network access"]
    async fn reaches_tencent_around_the_tunnel() {
        let bypass = detect().expect("no TUN owns the default route");
        println!("{bypass:?}");
        let addresses = resolve_via(&bypass, "finder.video.qq.com").await.unwrap();
        assert!(addresses.iter().all(|ip| ip.octets()[..2] != [198, 18] && ip.octets()[..2] != [198, 19]));
        crate::platforms::wechat::capture::install_crypto_provider();
        let client = hudsucker::hyper_util::client::legacy::Client::builder(
            hudsucker::hyper_util::rt::TokioExecutor::new(),
        )
        .build::<_, http_body_util::Empty<hudsucker::hyper::body::Bytes>>(connector(Some(bypass)).unwrap());
        let response = client
            .get("https://finder.video.qq.com/".parse().unwrap())
            .await
            .unwrap();
        println!("status {}", response.status());
    }
}
