//! The programs a host knows by CID: what an `import` (and, borrowing, a program's references) is resolved from where
//! there is no snippet store of the checker's own.
//!
//! The language never fetches (R-06): a program is checked from its text and the programs it is given. Here they are given
//! two ways, and each answers only for a whole CID, and only with the text that CID names (it is re-hashed before it is
//! kept, and again when it is used):
//!
//! - [`know`]: a program the host holds, handed in as (CID, canonical text). The page does this for what it fetched from
//!   `/api/programs/<cid>` and for what a link carried; `beatscript check --with` does it for the files it is given.
//! - [`set_source`]: a store the process reads from, for a server that checks against the programs it keeps.
//!
//! Knowing more programs can only turn "not here" into the answer a host that had them gives: a CID names one text.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, RwLock};

use djlive::compile::program_cid;
use djlive::env::{not_here, Snippets};

/// The most programs, and the most text, a host is told about: a page that is handed more is told so, not slowed.
pub const MAX_KNOWN: usize = 1024;
pub const MAX_KNOWN_BYTES: usize = 8 << 20;

static KNOWN: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());

type Source = Arc<dyn Snippets + Send + Sync>;
static SOURCE: RwLock<Option<Source>> = RwLock::new(None);

/// Is `s` a whole CID (`z` and base58, as the store's `is_cid`)?
pub fn is_cid(s: &str) -> bool {
    let b = s.as_bytes();
    b.first() == Some(&b'z') && (21..=81).contains(&b.len()) && b[1..].iter().all(|c| matches!(c, b'1'..=b'9' | b'A'..=b'H' | b'J'..=b'N' | b'P'..=b'Z' | b'a'..=b'k' | b'm'..=b'z'))
}

/// Tell the host a program: its CID and its canonical text. Refused when the text is not the program the CID names, or when
/// the host holds as many programs as it will.
pub fn know(cid: &str, text: &str) -> Result<(), String> {
    if !is_cid(cid) {
        return Err(format!("{cid:?} is not a CID"));
    }
    if program_cid(text) != cid {
        return Err(format!("the text given for {} is not the program it names: refused", djengine::rights::short_cid(cid)));
    }
    let mut k = KNOWN.lock().map_err(|_| "the known programs are locked".to_string())?;
    if k.contains_key(cid) {
        return Ok(());
    }
    if k.len() >= MAX_KNOWN || k.values().map(String::len).sum::<usize>() + text.len() > MAX_KNOWN_BYTES {
        return Err(format!("{} programs are already known here, as many as a page keeps: reload to start again", k.len()));
    }
    k.insert(cid.to_string(), text.to_string());
    Ok(())
}

/// [`know`] as the JSON the page reads: `{"ok":true}` or `{"ok":false,"error":"..."}`.
pub fn know_json(cid: &str, text: &str) -> String {
    use djengine::json::{JsonObject, JsonValue};
    match know(cid, text) {
        Ok(()) => JsonValue::Object(JsonObject::from_pairs([("ok", JsonValue::Bool(true))])),
        Err(e) => JsonValue::Object(JsonObject::from_pairs([("ok", JsonValue::Bool(false)), ("error", JsonValue::str(e))])),
    }
    .js_stringified()
}

/// Whether a program is known (here, not in the source).
pub fn knows(cid: &str) -> bool {
    KNOWN.lock().map(|k| k.contains_key(cid)).unwrap_or(false)
}

/// Forget every program told with [`know`] (tests; a page that starts over).
pub fn forget_all() {
    if let Ok(mut k) = KNOWN.lock() {
        k.clear();
    }
}

/// Read programs from `source` too: a server's store. Its answers are re-hashed like any other.
pub fn set_source(source: Source) {
    if let Ok(mut s) = SOURCE.write() {
        *s = Some(source);
    }
}

/// What the web's checks resolve an `import` against: the programs told to [`know`], then the [`set_source`] store.
pub struct Known;

impl Snippets for Known {
    fn resolve(&self, query: &str) -> Result<(String, String), String> {
        if !is_cid(query) {
            // a name needs a store of names, which a web host has not; a CID written short is a CID, and is not here
            let cid_like = query.starts_with('z') && query.chars().all(|c| c.is_ascii_alphanumeric());
            return Err(if cid_like { not_here(query) } else { crate::snapshot::NO_IMPORT.to_string() });
        }
        let held = KNOWN.lock().ok().and_then(|k| k.get(query).cloned());
        if let Some(text) = held {
            return Ok((query.to_string(), text));
        }
        let source = SOURCE.read().ok().and_then(|s| s.clone());
        if let Some(src) = source {
            if let Ok((cid, text)) = src.resolve(query) {
                if cid == query && program_cid(&text) == cid {
                    return Ok((cid, text));
                }
            }
        }
        Err(not_here(query))
    }
}
