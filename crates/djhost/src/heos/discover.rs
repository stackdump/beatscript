//! Finding HEOS players on the LAN, with no address written down anywhere: an SSDP search for
//! Denon's device type and an mDNS question for `_heos-audio._tcp.local`, asked together. A
//! responder's own source address is its IP. One player is enough: `player/get_players` on it
//! lists every zone with its address.
//!
//! The mDNS question is a one-shot ("legacy unicast", RFC 6762 §6.7) from an ephemeral port, so
//! it needs no port 5353 of its own and doesn't disturb the system's responder.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

/// SSDP's search target for HEOS players.
pub const SSDP_ST: &str = "urn:schemas-denon-com:device:ACT-Denon:1";
/// The mDNS service HEOS players advertise.
pub const MDNS_SERVICE: &str = "_heos-audio._tcp.local";

/// A player that answered: its address and, from mDNS, its zone name.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Found {
    pub ip: IpAddr,
    pub name: Option<String>,
    /// `ssdp`, `mdns` or both.
    pub via: String,
}

fn ssdp_request(mx: u64) -> String {
    format!(
        "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: {mx}\r\nST: {SSDP_ST}\r\n\r\n"
    )
}

/// Does an SSDP answer name a HEOS player?
pub fn ssdp_is_heos(reply: &str) -> bool {
    reply.lines().any(|l| {
        let l = l.to_ascii_lowercase();
        (l.starts_with("st:") || l.starts_with("nt:")) && l.contains("act-denon")
    })
}

/// A DNS question for `name`'s PTR records, id 0, as one packet.
pub fn mdns_query(name: &str) -> Vec<u8> {
    let mut p = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in name.trim_end_matches('.').split('.') {
        p.push(label.len() as u8);
        p.extend_from_slice(label.as_bytes());
    }
    p.push(0);
    p.extend_from_slice(&[0, 12, 0, 1]); // PTR, IN
    p
}

/// A (possibly compressed) name at `at`; the offset after it.
fn read_name(p: &[u8], mut at: usize) -> Option<(String, usize)> {
    let mut labels = Vec::new();
    let mut end = None;
    for _ in 0..64 {
        let len = *p.get(at)? as usize;
        if len == 0 {
            return Some((labels.join("."), end.unwrap_or(at + 1)));
        }
        if len & 0xC0 == 0xC0 {
            let ptr = ((len & 0x3F) << 8) | *p.get(at + 1)? as usize;
            end.get_or_insert(at + 2);
            at = ptr;
            continue;
        }
        let l = p.get(at + 1..at + 1 + len)?;
        labels.push(String::from_utf8_lossy(l).into_owned());
        at += 1 + len;
    }
    None
}

/// From one mDNS answer: the instance names its PTR records give for `service` (the zone names,
/// `Kitchen` from `Kitchen._heos-audio._tcp.local`) and the IPv4 addresses its A records hold.
pub fn mdns_parse(p: &[u8], service: &str) -> (Vec<String>, Vec<Ipv4Addr>) {
    let (mut names, mut ips) = (Vec::new(), Vec::new());
    let count = |k: usize| {
        p.get(k..k + 2)
            .map_or(0, |b| u16::from_be_bytes([b[0], b[1]]) as usize)
    };
    if p.len() < 12 || p[2] & 0x80 == 0 {
        return (names, ips); // not a response
    }
    let (qd, records) = (count(4), count(6) + count(8) + count(10));
    let mut at = 12;
    for _ in 0..qd {
        let Some((_, next)) = read_name(p, at) else {
            return (names, ips);
        };
        at = next + 4;
    }
    let service = service.trim_end_matches('.').to_ascii_lowercase();
    for _ in 0..records {
        let Some((owner, next)) = read_name(p, at) else {
            break;
        };
        let Some(h) = p.get(next..next + 10) else {
            break;
        };
        let kind = u16::from_be_bytes([h[0], h[1]]);
        let len = u16::from_be_bytes([h[8], h[9]]) as usize;
        let data = next + 10;
        if p.len() < data + len {
            break;
        }
        match kind {
            12 if owner.to_ascii_lowercase() == service => {
                if let Some((target, _)) = read_name(p, data) {
                    let cut = target.len().saturating_sub(service.len() + 1);
                    if target
                        .to_ascii_lowercase()
                        .ends_with(&format!(".{service}"))
                        && cut > 0
                    {
                        names.push(target[..cut].to_string());
                    }
                }
            }
            1 if len == 4 => ips.push(Ipv4Addr::new(
                p[data],
                p[data + 1],
                p[data + 2],
                p[data + 3],
            )),
            _ => {}
        }
        at = data + len;
    }
    (names, ips)
}

fn listen(sock: &UdpSocket, until: Instant, mut each: impl FnMut(&[u8], SocketAddr)) {
    let mut buf = [0u8; 9000];
    while let Some(left) = until.checked_duration_since(Instant::now()) {
        let _ = sock.set_read_timeout(Some(left.max(Duration::from_millis(5))));
        match sock.recv_from(&mut buf) {
            Ok((n, from)) => each(&buf[..n], from),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                break
            }
            Err(_) => break,
        }
    }
}

fn ssdp(wait: Duration) -> Vec<IpAddr> {
    let Ok(sock) = UdpSocket::bind("0.0.0.0:0") else {
        return Vec::new();
    };
    let _ = sock.set_multicast_ttl_v4(2);
    let req = ssdp_request(wait.as_secs().clamp(1, 5));
    let until = Instant::now() + wait;
    let mut out = Vec::new();
    // UDP is lossy: ask twice
    for _ in 0..2 {
        let _ = sock.send_to(req.as_bytes(), "239.255.255.250:1900");
    }
    listen(&sock, until, |b, from| {
        if ssdp_is_heos(&String::from_utf8_lossy(b)) && !out.contains(&from.ip()) {
            out.push(from.ip());
        }
    });
    out
}

fn mdns(wait: Duration) -> Vec<(IpAddr, Vec<String>)> {
    let Ok(sock) = UdpSocket::bind("0.0.0.0:0") else {
        return Vec::new();
    };
    let _ = sock.set_multicast_ttl_v4(255);
    let q = mdns_query(MDNS_SERVICE);
    for _ in 0..2 {
        let _ = sock.send_to(&q, "224.0.0.251:5353");
    }
    let mut out: Vec<(IpAddr, Vec<String>)> = Vec::new();
    listen(&sock, Instant::now() + wait, |b, from| {
        let (names, _) = mdns_parse(b, MDNS_SERVICE);
        if names.is_empty() {
            return;
        }
        match out.iter_mut().find(|(ip, _)| *ip == from.ip()) {
            Some((_, n)) => n.extend(names),
            None => out.push((from.ip(), names)),
        }
    });
    out
}

/// Ask the LAN for HEOS players, over SSDP and mDNS at once, for `wait`.
pub fn discover(wait: Duration) -> Vec<Found> {
    let m = std::thread::spawn(move || mdns(wait));
    let s = ssdp(wait);
    let m = m.join().unwrap_or_default();
    let mut by: BTreeMap<IpAddr, Found> = BTreeMap::new();
    for ip in s {
        by.insert(
            ip,
            Found {
                ip,
                name: None,
                via: "ssdp".into(),
            },
        );
    }
    for (ip, names) in m {
        // a player answers for itself only, but one packet may name several: take the first
        let name = names.into_iter().next();
        by.entry(ip)
            .and_modify(|f| {
                f.via = "ssdp+mdns".into();
                f.name = name.clone();
            })
            .or_insert(Found {
                ip,
                name,
                via: "mdns".into(),
            });
    }
    by.into_values().collect()
}

/// This machine's address on the way to `peer` (the interface the route to it leaves by): what
/// a player must be told to fetch a stream from. No packet is sent.
pub fn local_ip_toward(peer: IpAddr) -> Option<IpAddr> {
    let s = UdpSocket::bind(if peer.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    })
    .ok()?;
    s.connect(SocketAddr::new(peer, super::proto::PORT)).ok()?;
    s.local_addr().ok().map(|a| a.ip())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssdp_answers_are_told_apart() {
        let heos = "HTTP/1.1 200 OK\r\nLOCATION: http://x:60006/upnp/desc/aios_device/aios_device.xml\r\nST: urn:schemas-denon-com:device:ACT-Denon:1\r\n\r\n";
        assert!(ssdp_is_heos(heos));
        assert!(!ssdp_is_heos(
            "HTTP/1.1 200 OK\r\nST: upnp:rootdevice\r\n\r\n"
        ));
        assert!(ssdp_request(1).contains(SSDP_ST));
    }

    /// A response as a player sends it: the question echoed, a PTR to its instance (the
    /// instance name compressed against the question), and its A record.
    #[test]
    fn mdns_answers_give_zone_names_and_addresses() {
        let mut p = vec![0, 0, 0x84, 0, 0, 1, 0, 1, 0, 0, 0, 1];
        let q = mdns_query(MDNS_SERVICE);
        p.extend_from_slice(&q[12..]); // the question, name at offset 12
                                       // answer: owner = pointer to 12, PTR IN, ttl, data = "Kitchen" + pointer to 12
        p.extend_from_slice(&[0xC0, 12, 0, 12, 0, 1, 0, 0, 0, 120]);
        let mut data = vec![7];
        data.extend_from_slice(b"Kitchen");
        data.extend_from_slice(&[0xC0, 12]);
        p.extend_from_slice(&(data.len() as u16).to_be_bytes());
        p.extend_from_slice(&data);
        // additional: an A record for "kitchen.local"
        p.extend_from_slice(&[7]);
        p.extend_from_slice(b"kitchen");
        p.extend_from_slice(&[5]);
        p.extend_from_slice(b"local");
        p.extend_from_slice(&[0, 0, 1, 0x80, 1, 0, 0, 0, 120, 0, 4, 192, 168, 1, 241]);
        let (names, ips) = mdns_parse(&p, MDNS_SERVICE);
        assert_eq!(names, ["Kitchen"]);
        assert_eq!(ips, [Ipv4Addr::new(192, 168, 1, 241)]);
        // a query (not a response) and junk give nothing, and don't panic
        let none: (Vec<String>, Vec<Ipv4Addr>) = (vec![], vec![]);
        assert_eq!(mdns_parse(&q, MDNS_SERVICE), none);
        assert_eq!(mdns_parse(&[0xFF; 7], MDNS_SERVICE), none);
        assert!(mdns_parse(&p[..40], MDNS_SERVICE).1.is_empty());
    }

    #[test]
    fn the_way_out_to_loopback_is_loopback() {
        assert_eq!(
            local_ip_toward(IpAddr::V4(Ipv4Addr::LOCALHOST)),
            Some(IpAddr::V4(Ipv4Addr::LOCALHOST))
        );
    }
}
