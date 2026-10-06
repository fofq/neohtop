//! Listening-port role probing
//!
//! Answers "what does this listener actually speak" with a battery of
//! tiny protocol pings: a TLS ClientHello, a SOCKS5 method greeting, an
//! ordinary HTTP `HEAD /` (plus `GET /version` for clash-style
//! controllers and `GET /_ping` for Docker), a DNS query framed for
//! DNS-over-TCP, a server-speaks-first banner read (SSH/FTP/SMTP/VNC/
//! MySQL/telnet), a RESP `PING`, a PostgreSQL SSLRequest, a MongoDB
//! legacy `isMaster` OP_QUERY and an X.224 connection request for RDP.
//! Every probe opens its own short-lived connection on its own thread,
//! so the battery's wall clock stays near one round trip; it only ever
//! runs for one listener at a time when the user asks — never as a
//! background scan — so services see nothing beyond the equivalent of
//! `curl -I`.
//!
//! The findings here are mechanical facts (did it answer X); deciding
//! which human-readable role they imply happens in the frontend, where
//! the owning process is known.

use serde::Serialize;
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::time::Duration;

/// How long to wait for the TCP connection itself
const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);

/// How long to wait for each probe's reply before declaring it silent.
/// Probes only ever target the machine's own listeners, so latency is
/// negligible and this stays snappy.
const READ_TIMEOUT: Duration = Duration::from_millis(300);

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
    /// Sent a server-speaks-first banner identifying its protocol
    #[serde(skip_serializing_if = "Option::is_none")]
    pub greeting: Option<GreetingKind>,
    /// Answered a RESP PING with a +PONG or a -error line
    pub redis: bool,
    /// Answered the PostgreSQL SSLRequest with a single S/N byte
    pub postgres: bool,
    /// Answered a legacy `isMaster` OP_QUERY with an OP_REPLY
    pub mongodb: bool,
    /// Answered an X.224 connection request with a TPKT confirm
    pub rdp: bool,
}

/// Protocols whose server greets the client before any input, read off
/// the first bytes of one unsolicited banner
#[derive(Serialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum GreetingKind {
    /// "SSH-2.0-..." banner
    Ssh,
    /// "220 " greeting answered 250 to EHLO
    Smtp,
    /// "220 " greeting that rejected EHLO
    Ftp,
    /// "RFB 00x.00y" banner
    Vnc,
    /// Length-prefixed handshake with protocol version 10
    Mysql,
    /// First byte is an IAC escape (0xFF)
    Telnet,
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
    /// `GET /version` answered 401: a secret-guarded core API refuses the
    /// unauthenticated probe, and the 401 body carries no version field —
    /// this is the controller signature an open probe can still see
    pub version_auth: bool,
    /// `GET /_ping` answered 200 with an OK body (the Docker Engine API)
    pub ping_ok: bool,
}

/// Probes one listener and returns the mechanical findings. `host` must
/// be a plain IP literal (the frontend resolves wildcard binds to
/// 127.0.0.1 before calling). `protocol` selects the battery: UDP
/// listeners only ever get the DNS-over-UDP probe (every other probe in
/// the battery speaks TCP), TCP listeners get the full fan-out. The TCP
/// probes run on short-lived threads — each owns its connection, and the
/// battery's wall clock stays near one round trip instead of the sum of
/// nine.
pub fn identify_port(
    host: &str,
    port: u16,
    protocol: Option<&str>,
) -> Result<PortProbe, String> {
    let ip: IpAddr = host
        .parse()
        .map_err(|_| format!("invalid probe address: {}", host))?;
    let addr = SocketAddr::new(ip, port);
    if protocol.map(|p| p.eq_ignore_ascii_case("UDP")).unwrap_or(false) {
        let dns = probe_dns_udp(addr, port);
        return Ok(PortProbe {
            dns,
            ..PortProbe::default()
        });
    }
    let probe = std::thread::scope(|s| {
        let tls = s.spawn(|| probe_tls(addr));
        let socks5 = s.spawn(|| probe_socks5(addr));
        let http = s.spawn(|| probe_http(addr));
        let dns = s.spawn(|| probe_dns(addr));
        let greeting = s.spawn(|| probe_greeting(addr));
        let redis = s.spawn(|| probe_redis(addr));
        let postgres = s.spawn(|| probe_postgres(addr));
        let mongodb = s.spawn(|| probe_mongodb(addr));
        let rdp = s.spawn(|| probe_rdp(addr));
        PortProbe {
            tls: tls.join().unwrap_or(false),
            socks5: socks5.join().unwrap_or(false),
            http: http.join().unwrap_or_default(),
            dns: dns.join().unwrap_or(false),
            greeting: greeting.join().unwrap_or_default(),
            redis: redis.join().unwrap_or(false),
            postgres: postgres.join().unwrap_or(false),
            mongodb: mongodb.join().unwrap_or(false),
            rdp: rdp.join().unwrap_or(false),
        }
    });
    Ok(probe)
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
    let version_reply = http_request(addr, "GET /version")
        .map(|version_reply| String::from_utf8_lossy(&version_reply).into_owned());
    let version_json = version_reply
        .as_deref()
        .map(|text| text.contains("\"version\""))
        .unwrap_or(false);
    let version_auth = version_reply
        .as_deref()
        .and_then(|text| parse_status(text))
        .map(|code| code == 401)
        .unwrap_or(false);
    let ping_reply = http_request(addr, "GET /_ping")
        .map(|ping_reply| String::from_utf8_lossy(&ping_reply).into_owned());
    let ping_ok = ping_reply
        .as_deref()
        .and_then(|ping_text| {
            parse_status(ping_text)
                .map(|code| code == 200 && text_body(ping_text).contains("OK"))
        })
        .unwrap_or(false);
    Some(HttpProbe {
        status,
        server,
        www_authenticate,
        version_json,
        version_auth,
        ping_ok,
    })
}

/// Body of an HTTP response head+text reply (after the blank line)
fn text_body(text: &str) -> &str {
    text.split("\r\n\r\n").nth(1).unwrap_or("")
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

/// mDNS service-enumeration query (PTR _services._dns-sd._udp.local) —
/// a plain A query would be ignored on 5353, and every mDNS responder
/// answers this one
const MDNS_QUERY: &[u8] = &[
    0x4e, 0x48, // transaction id
    0x00, 0x00, // standard query
    0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // one question
    0x09, b'_', b's', b'e', b'r', b'v', b'i', b'c', b'e', b's',
    0x07, b'_', b'd', b'n', b's', b'-', b's', b'd',
    0x04, b'_', b'u', b'd', b'p',
    0x05, b'l', b'o', b'c', b'a', b'l',
    0x00, // root label
    0x00, 0x0c, // type PTR
    0x00, 0x01, // class IN
];

/// DNS-over-UDP probe. The echo of our transaction id with the QR bit
/// set proves a DNS responder lives on this bind. Port 5353 speaks mDNS
/// and ignores A queries, so it gets the service-enumeration PTR query.
fn probe_dns_udp(addr: SocketAddr, port: u16) -> bool {
    // The DoT payload minus its two-byte TCP length prefix is exactly the
    // datagram a UDP DNS listener expects
    let query: &[u8] = if port == 5353 {
        MDNS_QUERY
    } else {
        &DNS_QUERY[2..]
    };
    let socket = std::net::UdpSocket::bind(if addr.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    })
    .ok()?;
    socket.connect(addr).ok()?;
    socket.set_read_timeout(Some(READ_TIMEOUT)).ok()?;
    socket.send(query).ok()?;
    let mut reply = [0u8; 512];
    let n = socket.recv(&mut reply).ok()?;
    // QR (response) flag high, our transaction id echoed back
    n >= 4 && reply[2] & 0x80 != 0 && reply[0] == query[0] && reply[1] == query[1]
}

/// Server-speaks-first banner read: one connection that only listens.
/// Protocols that wait for the client (HTTP, SOCKS, Redis, PostgreSQL,
/// MongoDB, RDP) stay silent here and fall through to their own probes.
fn probe_greeting(addr: SocketAddr) -> Option<GreetingKind> {
    let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).ok()?;
    stream.set_read_timeout(Some(READ_TIMEOUT)).ok()?;
    let mut chunk = [0u8; 512];
    let n = stream.read(&mut chunk).ok()?;
    if n == 0 {
        return None;
    }
    let banner = &chunk[..n];
    if banner.starts_with(b"SSH-") {
        return Some(GreetingKind::Ssh);
    }
    if banner.starts_with(b"RFB ") {
        return Some(GreetingKind::Vnc);
    }
    // MySQL handshake: 3-byte payload length + sequence 0x00 + protocol
    // version 10
    if banner.len() >= 5 && banner[3] == 0x00 && banner[4] == 0x0a {
        return Some(GreetingKind::Mysql);
    }
    if banner[0] == 0xff {
        return Some(GreetingKind::Telnet);
    }
    if banner.starts_with(b"220") {
        // "220 " opens both FTP and ESMTP; EHLO splits them — SMTP
        // answers 250, FTP rejects the unknown verb with 5xx
        stream.write_all(b"EHLO neohtop\r\n").ok()?;
        let mut buf = [0u8; 256];
        let follow = stream.read(&mut buf).unwrap_or(0);
        if follow > 0 && buf.starts_with(b"250") {
            return Some(GreetingKind::Smtp);
        }
        return Some(GreetingKind::Ftp);
    }
    None
}

/// RESP PING: Redis answers +PONG when open, and a -NOAUTH/-ERR/-DENIED
/// line when guarded — any of them is unmistakably RESP
fn probe_redis(addr: SocketAddr) -> bool {
    exchange(addr, b"PING\r\n")
        .map(|reply| {
            reply.starts_with(b"+PONG")
                || reply.starts_with(b"-NOAUTH")
                || reply.starts_with(b"-ERR")
                || reply.starts_with(b"-WRONGPASS")
                || reply.starts_with(b"-DENIED")
        })
        .unwrap_or(false)
}

/// PostgreSQL StartupMessage needs a length-prefixed payload; the 8-byte
/// SSLRequest is the cheapest well-formed one, and the server answers
/// exactly one byte: 'S' (SSL ok) or 'N' (plain)
const PG_SSL_REQUEST: &[u8] = &[
    0x00, 0x00, 0x00, 0x08, // length 8
    0x04, 0xd2, 0x16, 0x2f, // magic 80877103
];

fn probe_postgres(addr: SocketAddr) -> bool {
    exchange(addr, PG_SSL_REQUEST)
        .map(|reply| reply.len() == 1 && (reply[0] == b'S' || reply[0] == b'N'))
        .unwrap_or(false)
}

/// Legacy OP_QUERY {isMaster: 1} on admin.$cmd — the one message every
/// MongoDB generation still answers with an OP_REPLY. 58 bytes total:
/// 16 header (len 58, reqId 1, respTo 0, opCode 2004) + 4 flags +
/// "admin.$cmd\0" + skip 0 + return -1 + the 19-byte document.
const MONGO_ISMASTER: &[u8] = &[
    0x3a, 0x00, 0x00, 0x00, // message length 58
    0x01, 0x00, 0x00, 0x00, // request id 1
    0x00, 0x00, 0x00, 0x00, // response to 0
    0xd4, 0x07, 0x00, 0x00, // opCode 2004 (OP_QUERY)
    0x00, 0x00, 0x00, 0x00, // flags
    b'a', b'd', b'm', b'i', b'n', b'.', b'$', b'c', b'm', b'd', 0x00,
    0x00, 0x00, 0x00, 0x00, // number to skip
    0xff, 0xff, 0xff, 0xff, // number to return -1
    0x13, 0x00, 0x00, 0x00, // document length 19
    0x10, // element type int32
    b'i', b's', b'M', b'a', b's', b't', b'e', b'r', 0x00,
    0x01, 0x00, 0x00, 0x00, // value 1
    0x00, // document terminator
];

fn probe_mongodb(addr: SocketAddr) -> bool {
    exchange(addr, MONGO_ISMASTER)
        .map(|reply| {
            reply.len() >= 16
                && reply[8..12] == [0x01, 0x00, 0x00, 0x00]
                && reply[12..16] == [0x01, 0x00, 0x00, 0x00]
        })
        .unwrap_or(false)
}

/// X.224 connection request with a routing cookie and RDP negotiation
/// request — Windows RDP ignores shorter forms and only confirms this
/// one; the confirm arrives in the same 03 00 TPKT framing (verified
/// against a live listener). 42 bytes total.
const RDP_CONN_REQUEST: &[u8] = &[
    0x03, 0x00, 0x00, 0x2a, // TPKT: version 3, length 42
    0x25, 0xe0, // X.224 LI 37, CR code
    0x00, 0x00, 0x00, 0x00, 0x00, // dst/src refs, class 0
    b'C', b'o', b'o', b'k', b'i', b'e', b':', b' ', b'm', b's', b't', b's',
    b'h', b'a', b's', b'h', b'=', b'n', b'm', b'a', b'p', 0x0d, 0x0a,
    0x01, 0x00, 0x08, 0x00, // RDP_NEG_REQ: type, flags, length 8
    0x00, 0x00, 0x00, 0x00, // requestedProtocols: classic security
];

fn probe_rdp(addr: SocketAddr) -> bool {
    exchange(addr, RDP_CONN_REQUEST)
        .map(|reply| {
            reply.len() >= 4 && reply[0] == 0x03 && reply[1] == 0x00 && reply[2] == 0x00
        })
        .unwrap_or(false)
}
