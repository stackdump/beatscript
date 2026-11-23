//! A playlist: an ordered list of program CIDs, owned by one account.
//!
//! Items point at programs by CID and never copy their text (the program is the public, CC0 `programs/<cid>.json`
//! the Go server stores). Order matters and a CID may appear more than once, so every item has its own `id`: that is
//! what a move or a remove names, and what the browser's item-level merge matches on.
//!
//! `version` is a content hash of what a client can change (name, visibility, share token, items): any two copies
//! with the same version are the same playlist, so a device that sends `If-Match: "<version>"` learns at once
//! (409, with the server's copy) that another device changed it in between.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    pub cid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub added_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The CID the item named before the rename (BeatScript 1), when `migrate-rename` pointed it at the program's
    /// BeatScript 2 twin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renamed_from: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Playlist {
    pub id: String,
    pub owner: String,
    pub name: String,
    pub items: Vec<Item>,
    pub created: String,
    pub updated: String,
    /// private | link | public. `link` and `public` both open by the share token; `public` is reserved for a
    /// listing that does not exist yet, and is otherwise the same as `link`.
    pub visibility: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub share_token: Option<String>,
    pub version: String,
}

/// One account's file: `<data>/playlists/<key>.json`.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct AccountFile {
    pub v: u32,
    pub owner: String,
    pub playlists: Vec<Playlist>,
}

/// The limits, per account. `-playlists-*` flags.
#[derive(Clone, Debug)]
pub struct Limits {
    pub max_playlists: usize,
    pub max_items: usize,
    /// The account file's size on disk.
    pub max_account_bytes: usize,
    /// Every account file together.
    pub max_total_bytes: u64,
    pub writes_per_min: usize,
    pub requests_per_min: usize,
    pub resolve_per_min: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits { max_playlists: 100, max_items: 500, max_account_bytes: 512 << 10, max_total_bytes: 256 << 20, writes_per_min: 60, requests_per_min: 300, resolve_per_min: 120 }
    }
}

pub const MAX_NAME: usize = 80;
pub const MAX_TITLE: usize = 120;
pub const MAX_NOTE: usize = 500;

pub const VISIBILITIES: [&str; 3] = ["private", "link", "public"];

impl Playlist {
    /// The content hash: sha256 over the canonical JSON of what clients change, first 128 bits in hex.
    pub fn compute_version(&self) -> String {
        let canon = serde_json::json!({
            "id": self.id,
            "name": self.name,
            "visibility": self.visibility,
            "share": self.share_token,
            "items": self.items,
        });
        let h = Sha256::digest(serde_json::to_vec(&canon).unwrap());
        hex::encode(&h[..16])
    }

    pub fn seal(&mut self) {
        self.version = self.compute_version();
    }

    /// Readable by a share token.
    pub fn shared(&self) -> bool {
        self.visibility != "private" && self.share_token.is_some()
    }
}

/// Go's `reCID` (store.go): `z` + base58btc, 20 to 80 characters.
pub fn valid_cid(s: &str) -> bool {
    let n = s.len();
    (21..=81).contains(&n) && s.starts_with('z') && s[1..].bytes().all(|b| b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz".contains(&b))
}

/// `[A-Za-z0-9_-]{min,max}`: the ids and tokens this module makes (base64url).
pub fn valid_b64id(s: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

pub fn valid_playlist_id(s: &str) -> bool {
    s.strip_prefix("pl_").is_some_and(|r| valid_b64id(r, 16, 32))
}

/// An item id a client may choose (the browser makes them offline): 8 to 40 base64url characters.
pub fn valid_item_id(s: &str) -> bool {
    valid_b64id(s, 8, 40)
}

pub fn valid_share_token(s: &str) -> bool {
    valid_b64id(s, 22, 22)
}

/// Go's `cleanLine`: trimmed, 1..=max characters, valid text with no control characters.
pub fn clean_line(s: &str, max: usize) -> Option<String> {
    let t = s.trim();
    if t.is_empty() || t.chars().count() > max || t.chars().any(|c| (c as u32) < 0x20 || c as u32 == 0x7f) {
        return None;
    }
    Some(t.to_string())
}

/// An RFC 3339 UTC timestamp as a client might send it (`2026-11-19T12:34:56Z`, fractions allowed).
pub fn valid_time(s: &str) -> bool {
    let b = s.as_bytes();
    if !(20..=35).contains(&b.len()) || *b.last().unwrap() != b'Z' {
        return false;
    }
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    digits(0..4) && b[4] == b'-' && digits(5..7) && b[7] == b'-' && digits(8..10) && b[10] == b'T' && digits(11..13) && b[13] == b':' && digits(14..16) && b[16] == b':' && digits(17..19)
        && (b.len() == 20 || (b[19] == b'.' && b[20..b.len() - 1].iter().all(u8::is_ascii_digit) && b.len() > 21))
}

/// Now, as RFC 3339 UTC with seconds (what Go's `time.Now().UTC().Format(time.RFC3339)` prints).
pub fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    rfc3339(secs)
}

/// Unix seconds as RFC 3339 UTC (days from civil, Howard Hinnant's algorithm).
pub fn rfc3339(secs: i64) -> String {
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// `n` random bytes as base64url without padding.
pub fn rand_b64(n: usize) -> String {
    use base64::Engine;
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).expect("the OS has no entropy");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1795000000), "2026-11-18T11:06:40Z");
        assert_eq!(rfc3339(951782400), "2000-02-29T00:00:00Z");
        assert!(valid_time("2026-11-19T12:34:56Z"));
        assert!(valid_time("2026-11-19T12:34:56.123Z"));
        assert!(!valid_time("2026-11-19T12:34:56.Z"));
        assert!(!valid_time("2026-11-19 12:34:56Z"));
        assert!(!valid_time("yesterday"));
    }

    #[test]
    fn ids() {
        assert!(valid_cid("z4EBG9j2Jd8NiGWk1tAmfMtgBnovr1cZU95BGGSMtDGQzhvUjnr"));
        assert!(!valid_cid("b4EBG9j2Jd8NiGWk1tAmfMtgBnovr1cZU95"));
        assert!(!valid_cid("z0OIl00000000000000000000"));
        assert!(!valid_cid("z123"));
        assert!(valid_playlist_id(&format!("pl_{}", rand_b64(12))));
        assert!(!valid_playlist_id("pl_../../etc"));
        assert!(valid_share_token(&rand_b64(16)));
        assert_eq!(clean_line("  hi  ", 5).as_deref(), Some("hi"));
        assert_eq!(clean_line("a\u{7}b", 5), None);
        assert_eq!(clean_line("", 5), None);
        assert_eq!(clean_line("éééééé", 5), None);
    }

    #[test]
    fn version_is_content() {
        let mut p = Playlist { id: "pl_aaaaaaaaaaaaaaaa".into(), owner: "o".into(), name: "n".into(), items: vec![], created: "c".into(), updated: "u".into(), visibility: "private".into(), share_token: None, version: String::new() };
        p.seal();
        let v1 = p.version.clone();
        p.updated = "later".into();
        assert_eq!(p.compute_version(), v1, "a timestamp is not content");
        p.name = "m".into();
        assert_ne!(p.compute_version(), v1);
        assert_eq!(v1.len(), 32);
    }
}
