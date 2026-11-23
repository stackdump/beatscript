//! The access log, as the Go server writes it (`internal/btwsite/server/logging.go`): one JSON line per request in
//! `<dir>/access-YYYYMMDD.jsonl` (UTC day), 30 days kept, at most 100 MiB a day (then one note line and drops until
//! midnight), flushed every second.
//!
//! `{"time":…,"method":…,"path":…,"status":…,"ms":…,"bytes":…,"ip":"<hash>"}`
//!
//! Redacted the same way: no query strings, headers, cookies or bodies; the address is an HMAC-SHA256 (first 8 bytes,
//! hex) of its rate-limit key (IPv6 by /64) under `btw-log-ip-v1:<session secret>`, so the log cannot be reversed by
//! trying every IPv4 address and two servers with the same secret hash an address alike.
//!
//! Two differences, both on the side of saying less or saying it right:
//! - the path of `/api/render/<cid>.<packed>` is logged as `/api/render/<cid>.…`: the packed part IS the program text
//!   (Go logs its first 200 bytes);
//! - the hashed address is the client's (`X-Real-IP` behind nginx, as the limits use), where Go hashes the TCP peer,
//!   which behind nginx is always 127.0.0.1.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use sha2::Sha256;

pub const MAX_DAY_BYTES: u64 = 100 << 20;
const KEEP_DAYS: i64 = 30;

struct Open {
    day: String,
    w: BufWriter<File>,
    size: u64,
    dropped: u64,
}

pub struct AccessLog {
    dir: PathBuf,
    key: Vec<u8>,
    cur: Mutex<Option<Open>>,
}

impl AccessLog {
    pub fn new(dir: &Path, secret: &[u8]) -> std::io::Result<Arc<AccessLog>> {
        std::fs::create_dir_all(dir)?;
        let mut key = b"btw-log-ip-v1:".to_vec();
        key.extend_from_slice(secret);
        let l = Arc::new(AccessLog { dir: dir.to_path_buf(), key, cur: Mutex::new(None) });
        let weak = Arc::downgrade(&l);
        std::thread::Builder::new()
            .name("access log flush".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_secs(1));
                let Some(l) = weak.upgrade() else { return };
                let mut cur = l.cur.lock().unwrap();
                if let Some(o) = cur.as_mut() {
                    let _ = o.w.flush();
                }
            })
            .expect("log thread");
        Ok(l)
    }

    pub fn hash_ip(&self, ip_key: &str) -> String {
        let mut m = <Hmac<Sha256> as Mac>::new_from_slice(&self.key).expect("any key length");
        m.update(ip_key.as_bytes());
        m.finalize().into_bytes()[..8].iter().map(|b| format!("{b:02x}")).collect()
    }

    /// One request. `start` is when it arrived.
    pub fn write(&self, start: SystemTime, method: &str, path: &str, status: u16, ms: u64, bytes: u64, ip_key: &str) {
        let mut line = serde_json::Map::new();
        line.insert("time".into(), rfc3339(start).into());
        line.insert("method".into(), method.into());
        line.insert("path".into(), log_path(path).into());
        line.insert("status".into(), status.into());
        line.insert("ms".into(), ms.into());
        line.insert("bytes".into(), bytes.into());
        line.insert("ip".into(), self.hash_ip(ip_key).into());
        self.append(crate::go_json(&serde_json::Value::Object(line)));
    }

    fn append(&self, b: String) {
        let mut cur = self.cur.lock().unwrap();
        let day = day_of(SystemTime::now());
        if cur.as_ref().is_none_or(|o| o.day != day) {
            if let Some(mut o) = cur.take() {
                let _ = o.w.flush();
            }
            let Ok(f) = File::options().append(true).create(true).open(self.dir.join(format!("access-{day}.jsonl"))) else { return };
            let size = f.metadata().map(|m| m.len()).unwrap_or(0);
            *cur = Some(Open { day: day.clone(), w: BufWriter::with_capacity(16 << 10, f), size, dropped: 0 });
            self.prune(&day);
        }
        let o = cur.as_mut().unwrap();
        if o.size + b.len() as u64 + 1 > MAX_DAY_BYTES {
            if o.dropped == 0 {
                let note = format!("{{\"time\":\"{}\",\"method\":\"-\",\"path\":\"-\",\"status\":0,\"ms\":0,\"bytes\":0,\"ip\":\"\",\"note\":\"daily log cap reached: further lines are dropped until 00:00 UTC\"}}\n", rfc3339(SystemTime::now()));
                let _ = o.w.write_all(note.as_bytes());
                o.size += note.len() as u64;
            }
            o.dropped += 1;
            return;
        }
        let _ = o.w.write_all(b.as_bytes());
        let _ = o.w.write_all(b"\n");
        o.size += b.len() as u64 + 1;
    }

    fn prune(&self, today: &str) {
        let cut = day_of(SystemTime::now() - Duration::from_secs((KEEP_DAYS * 86_400) as u64));
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return };
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if let Some(d) = n.strip_prefix("access-").and_then(|r| r.strip_suffix(".jsonl")) {
                if d.len() == 8 && *d < *cut.as_str() && d != today {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
    }
}

/// The path as logged: at most 200 bytes, and a share link's packed program (the text itself) left out.
pub fn log_path(p: &str) -> String {
    let p = match p.strip_prefix("/api/render/") {
        Some(rest) => match rest.split_once('.') {
            Some((cid, tail)) => {
                // `<cid>.<packed>` and `<cid>.<packed>/status`: keep the CID and what follows the packed part's end
                let after = tail.find('/').map_or("", |i| &tail[i..]);
                format!("/api/render/{cid}.…{after}")
            }
            None => p.to_string(),
        },
        None => p.to_string(),
    };
    if p.len() > 200 {
        p[..p.floor_char_boundary(200)].to_string()
    } else {
        p
    }
}

fn civil(s: i64) -> (i64, i64, i64, i64) {
    let (days, rem) = (s.div_euclid(86_400), s.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d, rem)
}

fn secs(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

pub fn rfc3339(t: SystemTime) -> String {
    let (y, m, d, rem) = civil(secs(t));
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

fn day_of(t: SystemTime) -> String {
    let (y, m, d, _) = civil(secs(t));
    format!("{y:04}{m:02}{d:02}")
}

/// The secret the log key is made from, as the Go server finds its session secret (`auth.LoadSecret`):
/// `BEATSCRIPT_SESSION_SECRET` (raw, 16+ characters), else the hex file (`<data>/session.secret`), else a key for this
/// process only. Never created here: this process does not own the Go server's secret.
pub fn load_secret(file: &Path) -> Vec<u8> {
    if let Ok(s) = std::env::var("BEATSCRIPT_SESSION_SECRET") {
        if s.len() >= 16 {
            return s.into_bytes();
        }
    }
    if let Ok(t) = std::fs::read_to_string(file) {
        let t = t.trim();
        if t.len() >= 32 && t.len() % 2 == 0 {
            if let Some(raw) = (0..t.len()).step_by(2).map(|i| u8::from_str_radix(&t[i..i + 2], 16).ok()).collect::<Option<Vec<u8>>>() {
                return raw;
            }
        }
    }
    let mut k = format!("beatscript-serve:{}:", std::process::id()).into_bytes();
    k.extend_from_slice(&SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos().to_le_bytes());
    k
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_programs_are_not_logged() {
        let cid = "z4EBG9jExZJgAUPbCsG48G2PuevB6F3oCMBc8zhrv6rLtyywbrL";
        assert_eq!(log_path(&format!("/api/render/{cid}.eJxLTEpOSU1JTQ")), format!("/api/render/{cid}.…"));
        assert_eq!(log_path(&format!("/api/render/{cid}/status")), format!("/api/render/{cid}/status"));
        assert_eq!(log_path(&format!("/api/render/{cid}")), format!("/api/render/{cid}"));
        assert!(log_path(&"/x".repeat(300)).len() <= 200);
    }

    #[test]
    fn lines_have_gos_shape_and_no_address() {
        let d = std::env::temp_dir().join(format!("beatscript-serve-log-{}", std::process::id()));
        let l = AccessLog::new(&d, b"0123456789abcdef").unwrap();
        l.write(SystemTime::now(), "GET", "/api/render/x?format=wav", 200, 12, 345, "203.0.113.9");
        if let Some(o) = l.cur.lock().unwrap().as_mut() {
            o.w.flush().unwrap();
        }
        let f = std::fs::read_dir(&d).unwrap().flatten().next().unwrap().path();
        let s = std::fs::read_to_string(&f).unwrap();
        let v: serde_json::Value = serde_json::from_str(s.lines().next().unwrap()).unwrap();
        let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, ["time", "method", "path", "status", "ms", "bytes", "ip"]);
        assert!(!s.contains("203.0.113.9"));
        assert_eq!(v["ip"].as_str().unwrap().len(), 16);
        let _ = std::fs::remove_dir_all(&d);
    }
}
