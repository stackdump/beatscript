//! Finding djnet hosts (docs/NET-PROTOCOL.md §2): the host answers SSDP `M-SEARCH`es for
//! [`SSDP_ST`] and multicasts `NOTIFY` alive / byebye; a peer searches, as it searches for HEOS
//! zones.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::proto::{SSDP_ST, VERSION};

const GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 255, 250);
const PORT: u16 = 1900;

/// A host that answered.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FoundHost {
    pub ip: IpAddr,
    pub port: u16,
    pub id: String,
    pub name: String,
    pub version: u32,
}

impl FoundHost {
    pub fn addr(&self) -> SocketAddr {
        SocketAddr::new(self.ip, self.port)
    }
}

/// What the host says about itself.
#[derive(Clone, Debug)]
pub struct Advert {
    pub id: String,
    pub name: String,
    pub port: u16,
    pub server: String,
}

impl Advert {
    fn headers(&self, ip: IpAddr) -> String {
        format!(
            "CACHE-CONTROL: max-age=60\r\nLOCATION: djnet://{ip}:{port}/\r\nSERVER: {server} djnet/{VERSION}\r\nX-DJNET-NAME: {name}\r\nX-DJNET-PORT: {port}\r\nX-DJNET-VERSION: {VERSION}\r\n",
            port = self.port,
            server = self.server,
            name = self.name.replace(['\r', '\n'], " "),
        )
    }

    /// The unicast answer to a search from `peer`.
    pub fn response(&self, ip: IpAddr) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nST: {SSDP_ST}\r\nUSN: uuid:{id}::{SSDP_ST}\r\nEXT:\r\n{h}\r\n",
            id = self.id,
            h = self.headers(ip)
        )
    }

    /// `NOTIFY` with `nts` = `ssdp:alive` or `ssdp:byebye`.
    pub fn notify(&self, ip: IpAddr, nts: &str) -> String {
        format!(
            "NOTIFY * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nNT: {SSDP_ST}\r\nNTS: {nts}\r\nUSN: uuid:{id}::{SSDP_ST}\r\n{h}\r\n",
            id = self.id,
            h = self.headers(ip)
        )
    }
}

/// Headers of an SSDP message, lower-cased names.
pub fn headers(msg: &str) -> BTreeMap<String, String> {
    msg.lines()
        .skip(1)
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            Some((k.trim().to_ascii_lowercase(), v.trim().to_string()))
        })
        .collect()
}

/// A search or a notify naming a djnet host, from `from`: the host. None for anything else
/// (a byebye included: [`is_byebye`]).
pub fn parse_host(msg: &str, from: IpAddr) -> Option<FoundHost> {
    let h = headers(msg);
    let kind = h.get("st").or_else(|| h.get("nt"))?;
    if !kind.eq_ignore_ascii_case(SSDP_ST) || h.get("nts").is_some_and(|n| n.contains("byebye")) {
        return None;
    }
    let id = h
        .get("usn")?
        .trim_start_matches("uuid:")
        .split("::")
        .next()?
        .to_string();
    Some(FoundHost {
        ip: from,
        port: h.get("x-djnet-port").and_then(|p| p.parse().ok())?,
        id,
        name: h.get("x-djnet-name").cloned().unwrap_or_default(),
        version: h
            .get("x-djnet-version")
            .and_then(|v| v.parse().ok())
            .unwrap_or(1),
    })
}

/// Does an `M-SEARCH` ask for djnet hosts (or everything)?
pub fn wants_us(msg: &str) -> bool {
    if !msg.starts_with("M-SEARCH") {
        return false;
    }
    headers(msg)
        .get("st")
        .is_some_and(|st| st.eq_ignore_ascii_case(SSDP_ST) || st == "ssdp:all")
}

pub fn search_request() -> String {
    format!(
        "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 1\r\nST: {SSDP_ST}\r\n\r\n"
    )
}

/// Search for hosts for `wait`. Hosts answering on several addresses are listed once per id.
pub fn search(wait: Duration) -> Vec<FoundHost> {
    let Ok(s) = UdpSocket::bind("0.0.0.0:0") else {
        return Vec::new();
    };
    let _ = s.set_read_timeout(Some(Duration::from_millis(100)));
    let _ = s.set_multicast_ttl_v4(2);
    let req = search_request();
    let mut found: BTreeMap<String, FoundHost> = BTreeMap::new();
    let t0 = Instant::now();
    let mut sent = 0;
    let mut buf = [0u8; 2048];
    while t0.elapsed() < wait {
        // three searches spread over the wait: UDP on Wi-Fi drops
        if sent < 3 && t0.elapsed() >= wait / 4 * sent {
            let _ = s.send_to(req.as_bytes(), (GROUP, PORT));
            sent += 1;
        }
        if let Ok((n, from)) = s.recv_from(&mut buf) {
            if let Some(h) = parse_host(&String::from_utf8_lossy(&buf[..n]), from.ip()) {
                found.entry(h.id.clone()).or_insert(h);
            }
        }
    }
    found.into_values().collect()
}

/// The host's responder: answers searches, notifies alive every 30 s and byebye at the end.
pub struct Responder {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Responder {
    pub fn start(advert: Advert) -> Result<Responder, String> {
        let s = bind_shared(PORT).map_err(|e| format!("SSDP on UDP {PORT}: {e}"))?;
        s.join_multicast_v4(&GROUP, &Ipv4Addr::UNSPECIFIED)
            .map_err(|e| format!("SSDP join {GROUP}: {e}"))?;
        let _ = s.set_read_timeout(Some(Duration::from_millis(200)));
        let _ = s.set_multicast_ttl_v4(2);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = std::thread::Builder::new()
            .name("djnet-ssdp".into())
            .spawn(move || {
                let mut buf = [0u8; 2048];
                let mut last_notify: Option<Instant> = None;
                let me = || {
                    crate::heos::discover::local_ip_toward(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)))
                        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
                };
                while !flag.load(Ordering::Relaxed) {
                    if last_notify.is_none_or(|t| t.elapsed() > Duration::from_secs(30)) {
                        let _ = s.send_to(advert.notify(me(), "ssdp:alive").as_bytes(), (GROUP, PORT));
                        last_notify = Some(Instant::now());
                    }
                    let Ok((n, from)) = s.recv_from(&mut buf) else {
                        continue;
                    };
                    let msg = String::from_utf8_lossy(&buf[..n]);
                    if std::env::var_os("DJNET_DEBUG").is_some() {
                        eprintln!("djnet ssdp: {} bytes from {from}: {:?}", n, msg.lines().next());
                    }
                    if wants_us(&msg) {
                        let ip = crate::heos::discover::local_ip_toward(from.ip()).unwrap_or_else(me);
                        let _ = s.send_to(advert.response(ip).as_bytes(), from);
                    }
                }
                let _ = s.send_to(advert.notify(me(), "ssdp:byebye").as_bytes(), (GROUP, PORT));
            })
            .map_err(|e| e.to_string())?;
        Ok(Responder {
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for Responder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// UDP `port` on every interface, shared with the other SSDP listeners on the box (macOS's own
/// and browsers' hold 1900 with SO_REUSEPORT too).
#[cfg(unix)]
#[allow(unsafe_code)]
fn bind_shared(port: u16) -> std::io::Result<UdpSocket> {
    use std::os::unix::io::FromRawFd;
    // SAFETY: plain BSD socket calls on a descriptor we own until it is handed to UdpSocket.
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0);
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let one: libc::c_int = 1;
        let sz = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        let p = &one as *const libc::c_int as *const libc::c_void;
        libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_REUSEADDR, p, sz);
        libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_REUSEPORT, p, sz);
        let addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port);
        let mut sa: libc::sockaddr_in = std::mem::zeroed();
        sa.sin_family = libc::AF_INET as _;
        sa.sin_port = addr.port().to_be();
        sa.sin_addr = libc::in_addr { s_addr: 0 };
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        {
            sa.sin_len = std::mem::size_of::<libc::sockaddr_in>() as u8;
        }
        if libc::bind(
            fd,
            &sa as *const libc::sockaddr_in as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        ) < 0
        {
            let e = std::io::Error::last_os_error();
            libc::close(fd);
            return Err(e);
        }
        Ok(UdpSocket::from_raw_fd(fd))
    }
}

#[cfg(not(unix))]
fn bind_shared(port: u16) -> std::io::Result<UdpSocket> {
    UdpSocket::bind(("0.0.0.0", port))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn advert() -> Advert {
        Advert {
            id: "abc123".into(),
            name: "studio-mac".into(),
            port: 8795,
            server: "djapp/0.1".into(),
        }
    }

    #[test]
    fn a_response_and_a_notify_parse_back_to_the_host() {
        let ip: IpAddr = "192.0.2.9".parse().unwrap();
        let want = FoundHost {
            ip,
            port: 8795,
            id: "abc123".into(),
            name: "studio-mac".into(),
            version: 1,
        };
        assert_eq!(parse_host(&advert().response(ip), ip), Some(want.clone()));
        assert_eq!(parse_host(&advert().notify(ip, "ssdp:alive"), ip), Some(want));
        assert_eq!(parse_host(&advert().notify(ip, "ssdp:byebye"), ip), None);
        assert!(wants_us(&search_request()));
        assert!(!wants_us(&crate::heos::discover::ssdp_is_heos("").to_string()));
    }

    #[test]
    fn the_responder_answers_a_search_on_this_box() {
        let Ok(_r) = Responder::start(advert()) else {
            eprintln!("no SSDP port here; skipped");
            return;
        };
        let found = search(Duration::from_millis(1200));
        if found.is_empty() {
            // multicast loopback is off on some CI networks
            eprintln!("no multicast loopback here; skipped");
            return;
        }
        assert!(found.iter().any(|h| h.id == "abc123" && h.port == 8795), "{found:?}");
    }
}
