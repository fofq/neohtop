//! Listening-port role probing
//!
//! Answers "what does this listener actually speak" with a battery of
//! tiny protocol pings: a TLS ClientHello, a SOCKS5 method greeting, an
//! ordinary HTTP `HEAD /`, a clash-style `GET /version` and a DNS query
//! framed for DNS-over-TCP. Every probe opens its own short-lived
//! connection with sub-second timeouts, and the battery only ever runs
//! for one listener at a time when the user asks — never as a background
//! scan — so services see nothing beyond the equivalent of `curl -I`.
//!
//! The findings here are mechanical facts (did it answer X); deciding
//! which human-readable role they imply happens in the frontend, where
//! the owning process is known.

use serde::Serialize;
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::time::Duration;

/// How long to wait for the TCP connection itself
const CONNECT_TIMEOUT: Duration = Duration::from_millis(700);

/// How long to wait for each probe's reply before declaring it silent
const READ_TIMEOUT: Duration = Duration::from_millis(450);

/// Cap on bytes read from a reply so a chatty service cannot stall us
const MAX_REPLY: usize = 2048;

/// Mechanical findings of one probe battery. Every probe targets its own
/// fresh connection; a silent listener simply leaves the field false.
#[derive(Serialize, Debug, Default)]
pub struct PortProbe {
    /// Answered a TLS ClientHello (ServerHello or alert)
    pub tls: bool,
    /// Completed the SOCKS5 method-negotiation greeting
    pub socks5: bool,
    /// Answered an ordinary HTTP request (HEAD /)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http: Option<HttpProbe>,
    /// Answered a DNS-over-TCP query
    pub dns: bool,
}

/// What an HTTP-speaking listener revealed about itself
#[derive(Serialize, Debug)]
pub struct HttpProbe {
    /// Status code of the `HEAD /` response
    pub status: u16,
    /// Server response header, when the listener sends one
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    /// WWW-Authenticate header present (auth-guarded REST API)
    pub www_authenticate: bool,
    /// `GET /version` answered with a version-shaped JSON body (the
    /// clash/mihomo external-controller signature)
    pub version_json: bool,
}

/// Probes one listener and returns the mechanical findings. `host` must
/// be a plain IP literal (the frontend resolves wildcard binds to
/// 127.0.0.1 before calling).
pub fn identify_port(host: &str, port: u16) -> Result<PortProbe, String> {
    let ip: IpAddr = host
        .parse()
        .map_err(|_| format!("invalid probe address: {}", host))?;
    let addr = SocketAddr::new(ip, port);
    Ok(PortProbe {
        tls: probe_tls(addr),
        socks5: probe_socks5(addr),
        http: probe_http(addr),
        dns: probe_dns(addr),
    })
}

/// Opens one connection, sends `request` and collects the reply until the
/// peer closes, the read timeout fires or MAX_REPLY is reached. None when
/// the connect/send fails or nothing came back in time.
fn exchange(addr: SocketAddr, request: &[u8]) -> Option<Vec<u8>> {
    let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).ok()?;
    stream.set_read_timeout(Some(READ_TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(READ_TIMEOUT)).ok()?;
    stream.write_all(request).ok()?;
    let mut reply = Vec::with_capacity(512);
    let mut chunk = [0u8; 512];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                reply.extend_from_slice(&chunk[..n]);
                if reply.len() >= MAX_REPLY {
                    break;
                }
            }
            // Read timeout: keep whatever the peer sent before going quiet
            Err(_) => break,
        }
    }
    if reply.is_empty() {
        None
    } else {
        Some(reply)
    }
}

/// Minimal TLS 1.2 ClientHello (one cipher suite, no extensions); only
/// the reply's record-type bytes matter (0x15 alert / 0x16 handshake)
/// and every TLS stack answers it, while non-TLS listeners either close
/// or answer with something that does not start with a record header.
const TLS_CLIENT_HELLO: &[u8] = &[
    0x16, 0x03, 0x01, 0x00, 0x2d, // record: handshake, len 45
    0x01, 0x00, 0x00, 0x29, // ClientHello, len 41
    0x03, 0x03, // client version TLS 1.2
    0x52, 0x5b, 0x44, 0xd4, 0x98, 0xc2, 0x25, 0x99, 0x51, 0x60, 0xd3, 0x85, 0x5e, 0x8c, 0x14,
    0x15, 0x0b, 0x4b, 0x9e, 0x4f, 0x5e, 0x37, 0xae, 0x24, 0x87, 0x74, 0xd5, 0x35, 0x2a, 0xd3,
    0x76, 0x3e, // 32 random bytes
    0x00, // session id length 0
    0x00, 0x02, // cipher suites length 2
    0x00, 0x2f, // TLS_RSA_WITH_AES_128_CBC_SHA
    0x01, 0x00, // compression: null only
];

fn probe_tls(addr: SocketAddr) -> bool {
    exchange(addr, TLS_CLIENT_HELLO)
        .map(|reply| {
            reply.len() >= 2 && (reply[0] == 0x15 || reply[0] == 0x16) && reply[1] == 0x03
        })
        .unwrap_or(false)
}

/// SOCKS5 greeting: version 5, one method, no-auth. Any reply starting
/// with 0x05 (chosen method or no-acceptable-methods) proves SOCKS.
const SOCKS5_GREETING: &[u8] = &[0x05, 0x01, 0x00];

fn probe_socks5(addr: SocketAddr) -> bool {
    exchange(addr, SOCKS5_GREETING)
        .map(|reply| reply.len() >= 2 && reply[0] == 0x05)
        .unwrap_or(false)
}

fn probe_http(addr: SocketAddr) -> Option<HttpProbe> {
    let reply = http_request(addr, "HEAD /")?;
    let text = String::from_utf8_lossy(&reply).into_owned();
    let status = parse_status(&text)?;
    let server = header_value(&text, "server");
    let www_authenticate = header_value(&text, "www-authenticate").is_some();
    let version_json = http_request(addr, "GET /version")
        .map(|version_reply| {
            String::from_utf8_lossy(&version_reply).contains("\"version\"")
        })
        .unwrap_or(false);
    Some(HttpProbe {
        status,
        server,
        www_authenticate,
        version_json,
    })
}

/// Sends one HTTP request line + headers to the listener
fn http_request(addr: SocketAddr, target: &str) -> Option<Vec<u8>> {
    let host_header = match addr.ip() {
        IpAddr::V4(ip) => ip.to_string(),
        IpAddr::V6(ip) => format!("[{}]", ip),
    };
    let request = format!(
        "{} HTTP/1.1\r\nHost: {}\r\nUser-Agent: neohtop-probe\r\nConnection: close\r\n\r\n",
        target, host_header
    );
    exchange(addr, request.as_bytes())
}

/// Parses the status code out of an HTTP/1.x response head
fn parse_status(text: &str) -> Option<u16> {
    let mut parts = text.splitn(3, ' ');
    let version = parts.next()?;
    if !version.starts_with("HTTP/") {
        return None;
    }
    parts.next()?.get(..3)?.parse().ok()
}

/// Case-insensitive single-header lookup in the response head
fn header_value(text: &str, name: &str) -> Option<String> {
    let head = text.split("\r\n\r\n").next().unwrap_or(text);
    for line in head.lines() {
        if let Some((key, value)) = line.split_once(':') {
            if key.trim().eq_ignore_ascii_case(name) {
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

/// Minimal A query for "neohtop.test" with transaction id 0x4E48,
/// carrying the two-byte length prefix DNS-over-TCP requires. A DNS
/// listener echoes the id right after its own length prefix.
const DNS_QUERY: &[u8] = &[
    0x00, 0x1e, // TCP length prefix (30)
    0x4e, 0x48, // transaction id
    0x01, 0x00, // standard query, recursion desired
    0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // one question
    0x07, b'n', b'e', b'o', b'h', b't', b'o', b'p', // neohtop
    0x04, b't', b'e', b's', b't', // test
    0x00, // root label
    0x00, 0x01, // type A
    0x00, 0x01, // class IN
];

fn probe_dns(addr: SocketAddr) -> bool {
    exchange(addr, DNS_QUERY)
        .map(|reply| reply.len() >= 4 && reply[2] == 0x4e && reply[3] == 0x48)
        .unwrap_or(false)
}
