//! Self-sealing links (`djlive::share`) as JSON, for any origin: the page's own
//! `https://beatscript.app/#<CID>.<packed>` and the social form `/c/<CID>.<packed>` link unfurlers can fetch. Both pack
//! the formatted text (the `#!` headers and the comments kept); the CID is the canonical text's. BeatScript links are
//! made on beatscript.app only: there is no beats.bitwrap.io form (`beats_url` was dropped on 2026-11-22).

use djengine::json::{JsonObject, JsonValue};
use djlive::compile::check_with;
use djlive::share as links;

use crate::report::{diagnostics_json, obj, opt_str, strs};
use crate::snapshot::web_env;
use crate::utf16::Utf16Index;

/// The site's origin when the caller gives none: beatscript.app.
pub const DEFAULT_ORIGIN: &str = links::ORIGIN;

/// `share(text, sig, notes, origin)`: the link for a program. `sig` is the author's signature over the CID,
/// carried as given (the wallet makes and checks it). The link packs the program formatted, so its `#!` header lines
/// and comments come along (unsealed: the CID is the canonical text's). `notes` is accepted for the callers that pass
/// it and changes nothing: it only ever added the comments to the beats link, which is no longer made. A program with errors, or one that is private (`use restricted`, a restricted asset), gets no link.
pub fn share_json(text: &str, sig: Option<&str>, notes: bool, origin: &str) -> String {
    let env = web_env().env();
    let ix = Utf16Index::new(text);
    let chk = check_with(text, &env);
    let fail = |error: String| {
        obj([
            ("ok", JsonValue::Bool(false)),
            ("error", JsonValue::str(error)),
            ("url", JsonValue::Null),
            ("social_url", JsonValue::Null),
            ("cid", JsonValue::Null),
            ("chars", JsonValue::Null),
            ("canonical", JsonValue::Null),
            ("warnings", JsonValue::Array(Vec::new())),
            ("diagnostics", diagnostics_json(&chk.diagnostics, text, &ix)),
        ])
        .js_stringified()
    };
    let Some(c) = chk.compiled.as_ref() else { return fail("the program has errors: `check` says where".into()) };
    if c.rights.declared_private || c.rights.private_required {
        let why = c.rights.refusal().unwrap_or_else(|| "it declares `use restricted`: a private program is never shared".to_string());
        return fail(format!("not shared: {why}"));
    }
    let sig = sig.map(str::trim).filter(|s| !s.is_empty());
    // The link carries every program this one draws on (an `import`, a set's tracks), so that it opens on a machine that
    // has none of them. The link packs the formatted text
    let formatted = links::formatted(text, &env, c);
    let (url, carried) = links::encode_formatted(c, &formatted, origin, sig);
    if !c.plays.is_empty() && !carried {
        return fail(format!("not shared: the {} programs it draws on add up to more than a link carries ({} KiB)", c.plays.len(), links::MAX_CARRIED >> 10));
    }
    let _ = notes;
    let (origin, frag) = url.split_once("/#").unwrap_or((DEFAULT_ORIGIN, ""));
    let sealed = frag.split(['~', '*']).next().unwrap_or(frag);
    let social = format!("{origin}/c/{sealed}");
    let mut warnings: Vec<String> = Vec::new();
    if url.chars().count() > 2000 {
        warnings.push(format!("the link is {} characters: some places cut links over 2000", url.chars().count()));
    }
    if !c.rights.assets.is_empty() {
        warnings.push("the program names samples or loops: a link carries only their hashes, not the sounds".to_string());
    }
    if carried {
        warnings.push(format!("the link carries the {} program{} this one draws on", c.plays.len(), if c.plays.len() == 1 { "" } else { "s" }));
    }
    if sig.is_some() {
        warnings.push("the signature is carried as given: it is the wallet's to check".to_string());
    }
    obj([
        ("ok", JsonValue::Bool(true)),
        ("url", JsonValue::str(url.clone())),
        ("social_url", JsonValue::str(social)),
        ("cid", JsonValue::str(c.cid.clone())),
        ("chars", JsonValue::int(url.chars().count() as i64)),
        ("canonical", JsonValue::str(c.text.clone())),
        ("warnings", strs(warnings)),
    ])
    .js_stringified()
}

fn links_cid(text: &str) -> String {
    djlive::compile::program_cid(text)
}

/// What `open` is given, made into what `djlive::share::parse_link` reads: a fragment, `CID.packed`, a
/// bare CID, a full link with a `#`, or the social form `<origin>/c/<CID>.<packed>`.
fn normalize(s: &str) -> String {
    let s = s.trim();
    if !s.contains('#') {
        if let Some((_, rest)) = s.split_once("/c/") {
            // the social form carries the sealed part in the path (no notes, maybe a query or a slash after)
            let rest = rest.split(['?', '/']).next().unwrap_or("");
            return rest.to_string();
        }
    }
    s.to_string()
}

fn failure(error: String, claimed: Option<String>, has_text: bool) -> String {
    obj([
        ("ok", JsonValue::Bool(false)),
        ("cid", JsonValue::Null),
        ("canonical", JsonValue::Null),
        ("text", JsonValue::Null),
        ("verified", JsonValue::Bool(false)),
        ("warnings", JsonValue::Array(Vec::new())),
        ("notes_unverified", JsonValue::Array(Vec::new())),
        ("error", JsonValue::str(error)),
        ("claimed_cid", opt_str(claimed)),
        ("has_text", JsonValue::Bool(has_text)),
        ("sig", JsonValue::Null),
    ])
    .js_stringified()
}

/// `open(urlOrCidOrFragment)`: decode the text a link carries, canonicalise and compile it, recompute the
/// CID and refuse a link whose CID is not its text's. `canonical` is the program's canonical text (what is stored
/// and hashed); `text` is the same program formatted, with the `#!` header lines and comments the link carried, for
/// the editor (a link made before links carried them: the canonical text). A CID alone has no text to verify: the answer says so
/// (`claimed_cid`, `has_text: false`) for a caller with a store to look it up in.
pub fn open_json(s: &str) -> String {
    let env = web_env().env();
    let norm = normalize(s);
    let link = match links::parse_link(&norm) {
        Ok(l) => l,
        Err(e) => return failure(e, None, false),
    };
    let claimed = Some(link.cid.clone());
    match links::open(&norm, &env) {
        Err(e) => failure(e, claimed, link.text.is_some()),
        Ok(o) => {
            let text = link.text.clone().unwrap_or_default();
            // (read as `open` read it: what the link carries first, then what the host knows)
            let inline = djlive::set::Inline { tracks: link.tracks.clone(), store: env.snippets };
            let parsed = djlive::parse::parse_with(&text, &djlive::env::Env { snippets: Some(&inline), ..env });
            let canonical = djlive::print::canonical(&parsed.program);
            let mut warnings = o.warnings.clone();
            if let Some(d) = o.check.diagnostics.iter().find(|d| d.is_error()) {
                let (line, col, _) = d.position(&text);
                warnings.push(format!("the program does not check: {}:{}: {}", line, col, d.message));
            }
            obj([
                ("ok", JsonValue::Bool(true)),
                ("cid", JsonValue::str(o.cid.clone())),
                ("canonical", JsonValue::str(canonical)),
                // what an editor shows: the program formatted, with the `#!` headers and comments the link carried
                ("text", JsonValue::str(o.text.clone())),
                ("verified", JsonValue::Bool(true)),
                ("warnings", strs(warnings)),
                ("notes_unverified", strs(o.link.notes.iter().cloned())),
                ("error", JsonValue::Null),
                ("claimed_cid", JsonValue::str(link.cid)),
                ("has_text", JsonValue::Bool(true)),
                ("sig", opt_str(o.link.sig.clone())),
                // the programs the link carried, CID to canonical text, each the program its CID names: a
                // host keeps them, so that the program checks again where it is edited, with nothing fetched
                ("bundle", JsonValue::Object(JsonObject::from_pairs(o.link.tracks.iter().filter(|(c, t)| links_cid(t) == **c).map(|(c, t)| (c.as_str(), JsonValue::str(t.clone()))).collect::<Vec<_>>()))),
            ])
            .js_stringified()
        }
    }
}
