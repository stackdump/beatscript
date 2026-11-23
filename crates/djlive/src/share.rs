//! Self-sealing links. A program shared from beatscript.app is
//!
//! ```text
//! https://beatscript.app/#<CID>.<base64url(deflate(formatted text))>[.<sig>][*<carried programs>]
//! ```
//!
//! and beats' own form, which beats.bitwrap.io/p opens, is
//!
//! ```text
//! https://beats.bitwrap.io/p#<CID>.<base64url(deflate(canonical text))>[.<sig>][~<notes>]
//! ```
//!
//! The CID is the program's own (docs/BEATSCRIPT.md, "Canonical form"): whoever opens the link decodes the
//! text, canonicalises and compiles it, recomputes the CID, and refuses the link if it is not the one
//! it carries. So a link cannot say one thing and play another. The site's link packs the *formatted*
//! text (`format::format`: the canonical text with the `#!` header lines, the comments and the blank
//! lines put back), so the program opens as it was written; its CID is still the canonical text's, and
//! the comments and headers are not covered by it. beats' page hashes the text it unpacks as it is, so
//! its form packs the canonical text. A link made before the site packed the formatted text carries the
//! canonical text, which formats to itself: it opens as it always did. `sig` is the author's signature
//! over the CID (opaque here: the wallet makes and checks it). `notes` (beats' form only) are the
//! program's comments, deflated and tacked on after the sealed part: they are not covered by the CID or
//! the signature and are shown as unverified.

use djengine::codec::{base64_url, base64_url_decode};

use crate::compile::{check_with, program_cid, Check, Compiled};
use crate::env::Env;
use crate::print;

pub const BASE: &str = "https://beats.bitwrap.io/p";

/// Where a BeatScript link opens when the caller names no site: beatscript.app. (btw.bitwrap.io, the site's
/// old name, is only a legacy host the server still answers; no link is made on it.)
pub const ORIGIN: &str = "https://beatscript.app";

fn pack(text: &str) -> String {
    base64_url(&miniz_oxide::deflate::compress_to_vec(text.as_bytes(), 10))
}

fn unpack(s: &str) -> Result<String, String> {
    let bytes = base64_url_decode(s).ok_or("the link's text is not base64url")?;
    let raw = miniz_oxide::inflate::decompress_to_vec_with_limit(&bytes, 1 << 20).map_err(|e| format!("the link's text does not inflate ({e:?})"))?;
    String::from_utf8(raw).map_err(|_| "the link's text is not UTF-8".to_string())
}

/// The link for a compiled program. `sig`: the author's signature over the CID, if there is one.
/// `notes`: carry the comments (unsealed) too.
pub fn encode(c: &Compiled, sig: Option<&str>, notes: bool) -> String {
    encode_with_tracks(c, sig, notes).0
}

/// A link longer than this is warned of: some places cut a link that long. (It always carries what the program draws on, so
/// that opening it never depends on the machine: a program that draws on many is a long link.)
pub const INLINE_LIMIT: usize = 2000;

/// The most the programs a link carries may add up to, as text: [`unpack`] inflates 1 MiB at most, so a bundle past this is not
/// carried (and the link is only the program).
pub const MAX_CARRIED: usize = 512 << 10;

/// Like [`encode`], and says whether what the program draws on came along: a set's tracks, the snippets an `import` brings and
/// the programs a reference borrows from (`Compiled::plays`) are carried as their canonical texts (a `*` section, each checked
/// against its CID on opening), so a link opens on a machine that has none of them. False for a program that draws on nothing,
/// and for one whose programs add up to more than [`MAX_CARRIED`] (the caller says so: the link is not whole).
pub fn encode_with_tracks(c: &Compiled, sig: Option<&str>, notes: bool) -> (String, bool) {
    let notes = if notes { c.notes.as_slice() } else { &[] };
    with_tracks(c, sealed(&format!("{BASE}#"), &c.cid, &c.text, sig, notes))
}

/// The site's link for a compiled program: `<origin>/#<CID>.<packed formatted text>[.<sig>][*<carried>]`, and whether what
/// the program draws on came along (as [`encode_with_tracks`]). `formatted` is what [`formatted`] makes of the program's
/// source: the canonical text with its `#!` headers and comments kept, so the link opens as it was written. The CID is
/// the canonical text's, as in every link. No `~notes`: the comments are in the text. `origin` empty: [`ORIGIN`].
pub fn encode_formatted(c: &Compiled, formatted: &str, origin: &str, sig: Option<&str>) -> (String, bool) {
    let origin = origin.trim().trim_end_matches('/');
    let origin = if origin.is_empty() { ORIGIN } else { origin };
    with_tracks(c, sealed(&format!("{origin}/#"), &c.cid, formatted, sig, &[]))
}

/// What a link packs for a program: its source formatted (`format::format`), or the canonical text when the source does
/// not format. The formatter keeps the canonical text's CID (format.rs), so the link verifies either way.
pub fn formatted(src: &str, env: &Env, c: &Compiled) -> String {
    crate::format::format(src, env).unwrap_or_else(|_| c.text.clone())
}

fn with_tracks(c: &Compiled, link: String) -> (String, bool) {
    if c.plays.is_empty() || c.plays.iter().map(|(cid, t)| cid.len() + t.len() + 2).sum::<usize>() > MAX_CARRIED {
        return (link, false);
    }
    let bundle: Vec<String> = c.plays.iter().map(|(cid, t)| format!("{cid}\u{2}{t}")).collect();
    (format!("{link}*{}", pack(&bundle.join("\u{1}"))), true)
}

fn sealed(prefix: &str, cid: &str, text: &str, sig: Option<&str>, notes: &[String]) -> String {
    let mut s = format!("{prefix}{cid}.{}", pack(text));
    if let Some(sig) = sig {
        s.push('.');
        s.push_str(sig);
    }
    if !notes.is_empty() {
        s.push('~');
        s.push_str(&pack(&notes.join("\n")));
    }
    s
}

/// What a link says, before it is believed.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub cid: String,
    /// The text it carries (None: a CID alone, to be found in storage).
    pub text: Option<String>,
    pub sig: Option<String>,
    /// Unsealed comments.
    pub notes: Vec<String>,
    /// The programs the link carries: a set's tracks, the snippets an `import` brings, the programs a reference borrows from.
    /// CID to canonical text, each checked against its CID when used.
    pub tracks: std::collections::BTreeMap<String, String>,
}

/// A link, its fragment, `CID.text`, or a bare CID.
pub fn parse_link(s: &str) -> Result<Link, String> {
    let s = s.trim();
    let frag = match s.split_once('#') {
        Some((_, f)) => f,
        None => s,
    };
    let (frag, tracks) = match frag.split_once('*') {
        Some((a, t)) => (a, Some(t)),
        None => (frag, None),
    };
    let (sealed, notes) = match frag.split_once('~') {
        Some((a, n)) => (a, Some(n)),
        None => (frag, None),
    };
    let mut parts = sealed.split('.');
    let cid = parts.next().unwrap_or("").to_string();
    if cid.is_empty() || !cid.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("that is not a BeatScript link or CID (https://beats.bitwrap.io/p#<CID>.<text>)".into());
    }
    let text = match parts.next() {
        Some(t) if !t.is_empty() => Some(unpack(t)?),
        _ => None,
    };
    let sig = parts.next().filter(|s| !s.is_empty()).map(str::to_string);
    let notes = match notes {
        Some(n) => unpack(n)?.lines().map(str::to_string).collect(),
        None => Vec::new(),
    };
    let mut bundle = std::collections::BTreeMap::new();
    if let Some(t) = tracks {
        for item in unpack(t)?.split('\u{1}').filter(|i| !i.is_empty()) {
            let (c, text) = item.split_once('\u{2}').ok_or("the link's tracks are not CID and text pairs")?;
            bundle.insert(c.to_string(), text.to_string());
        }
    }
    Ok(Link { cid, text, sig, notes, tracks: bundle })
}

/// A link opened.
pub struct Opened {
    pub link: Link,
    /// The program, compiled from the link's text.
    pub check: Check,
    /// The CID recomputed from the text.
    pub cid: String,
    /// The text for an editor: the link's text formatted (`format::format`), with the `#!` headers and comments it carries
    /// (none in a link made before links packed the formatted text: that one is its canonical text).
    pub text: String,
    /// Things worth saying (the text was not in canonical form, a signature that cannot be checked here).
    pub warnings: Vec<String>,
}

/// Decode, canonicalise, compile, recompute the CID. An error when the text does not read, or when
/// the CID it carries is not the CID it has.
pub fn open(s: &str, env: &Env) -> Result<Opened, String> {
    let link = parse_link(s)?;
    let Some(text) = link.text.clone() else {
        return Err(format!("{} is a CID with no text in it: look it up in storage (`btw load`)", link.cid));
    };
    // what the program draws on comes from the link first (each checked against its CID), then from storage
    let inline = crate::set::Inline { tracks: link.tracks.clone(), store: env.snippets };
    let env = &Env { snippets: Some(&inline), ..*env };
    let parsed = crate::parse::parse_with(&text, env);
    let errors = parsed.diagnostics.iter().filter(|d| d.is_error()).count();
    if errors > 0 {
        let d = parsed.diagnostics.iter().find(|d| d.is_error()).unwrap();
        return Err(format!("the program in the link does not read: {} (line {})", d.message, d.position(&text).0));
    }
    let canonical = print::canonical(&parsed.program);
    let cid = program_cid(&canonical);
    if cid != link.cid {
        return Err(format!("refused: the link says {} but its text is {}", link.cid, cid));
    }
    // what an editor is given: the text formatted, which keeps what the link carries beside the program (its `#!` header
    // lines and comments) and has the same CID
    let formatted = crate::format::format(&text, env).unwrap_or_else(|_| canonical.clone());
    let mut warnings = Vec::new();
    if canonical != text && formatted != text {
        warnings.push("the text in the link was not in canonical form (formatting only: the CID is the same)".to_string());
    }
    if formatted != canonical {
        warnings.push("the link's comments and `#!` header lines came along unsealed: not covered by the CID".to_string());
    }
    if link.sig.is_some() {
        warnings.push("the link carries a signature: it is the author's wallet's to check, and is not checked here".to_string());
    }
    if !link.notes.is_empty() {
        warnings.push(format!("{} note line{} came along unsealed: not covered by the CID", link.notes.len(), if link.notes.len() == 1 { "" } else { "s" }));
    }
    let check = check_with(&text, env);
    Ok(Opened { link, check, cid, text: formatted, warnings })
}
