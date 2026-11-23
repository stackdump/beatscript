//! Open data: the public CC0 programs, findable and downloadable whole (docs/OPEN-DATA.md).
//!
//! - [`Catalog`]: every stored program that is public and CC0, newest first, from the programs store (`<cid>.json`,
//!   checked against its CID as every read of the store is). A private program, a non-CC0 one, one whose text says it is
//!   not public (`#! visibility:`) or one that does not hash to its name never appears: not in the sitemap, the feeds
//!   or a dump.
//! - [`sitemap`], [`json_feed`], [`atom_feed`]: the discovery documents, shaped as beats.bitwrap.io's `/feed.rss` items
//!   are (title, link, guid = CID, date, an enclosure per format) and its snapshot manifest is.
//! - [`build_dump`]: a content-addressed archive of all of them (`programs/<cid>.beatscript`, `programs/<cid>.jsonld`, and
//!   `manifest.jsonld`, the list of CIDs): deterministic bytes, so the same set of programs is the same archive and the
//!   same name. `beatscript-dump` (src/bin) writes it; nothing here schedules it.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// One public CC0 program.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub cid: String,
    /// The canonical text (what the CID hashes).
    pub text: String,
    pub name: String,
    pub author: String,
    pub forked_from: String,
    /// When it was stored: the meta's `created`, else the file's time (RFC 3339 UTC).
    pub created: String,
    /// The doc comments of the text as it was shared (the meta's `docs`: kind, name, doc); the canonical text has none.
    pub docs: Vec<(String, String, String)>,
    /// The `# from:` lines of the text as it was shared (the meta's `based_on`: `<call> -> <cid>`): its JSON-LD's vendored
    /// `isBasedOn` entries.
    pub based_on: Vec<String>,
}

/// `^z[1-9A-HJ-NP-Za-km-z]{20,80}$`, as the store's names are.
pub fn is_cid(s: &str) -> bool {
    let b = s.as_bytes();
    b.first() == Some(&b'z') && (21..=81).contains(&b.len()) && b[1..].iter().all(|c| matches!(c, b'1'..=b'9' | b'A'..=b'H' | b'J'..=b'N' | b'P'..=b'Z' | b'a'..=b'k' | b'm'..=b'z'))
}

/// A `#! visibility:` line that says anything but `public` (the store's `DeclaresNotPublic`).
fn text_not_public(text: &str) -> bool {
    text.lines().take_while(|l| l.starts_with("#!") || l.trim().is_empty()).any(|l| {
        let l = l.trim_start_matches("#!").trim();
        l.split_once(':').is_some_and(|(k, v)| k.trim().eq_ignore_ascii_case("visibility") && !v.trim().eq_ignore_ascii_case("public"))
    })
}

/// The program in one store file, when it may be published: public, CC0, and its text's CID is its name.
pub fn read_entry(path: &Path) -> Option<Entry> {
    let cid = path.file_name()?.to_str()?.strip_suffix(".json")?;
    if !is_cid(cid) {
        return None;
    }
    let b = std::fs::read(path).ok()?;
    let v: Value = serde_json::from_slice(&b).ok()?;
    let text = v["text"].as_str()?;
    if v["cid"].as_str() != Some(cid) || text.is_empty() || djlive::compile::program_cid(text) != cid {
        return None;
    }
    let m = &v["meta"];
    let s = |k: &str| m.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
    let vis = s("visibility");
    let lic = s("license");
    if !(vis.is_empty() || vis.eq_ignore_ascii_case("public")) || !(lic.is_empty() || lic == "CC0-1.0") || text_not_public(text) {
        return None;
    }
    let created = match s("created") {
        c if c.len() >= 20 && c.as_bytes()[4] == b'-' => c,
        _ => rfc3339(std::fs::metadata(path).and_then(|md| md.modified()).unwrap_or(SystemTime::UNIX_EPOCH)),
    };
    let docs = m
        .get("docs")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|d| {
                    let f = |k: &str| d.get(k).and_then(Value::as_str).map(str::to_string);
                    Some((f("kind")?, f("name")?, f("doc")?))
                })
                .collect()
        })
        .unwrap_or_default();
    let based_on = m.get("based_on").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    Some(Entry { cid: cid.to_string(), text: text.to_string(), name: s("name"), author: s("author"), forked_from: s("forked_from"), created, docs, based_on })
}

/// Every public CC0 program in `dir`, newest first (then by CID, so the order is total).
/// A BeatScript 1 program whose BeatScript 2 twin is stored too (the rename's migration writes one for every stored program)
/// is listed once, as the twin: its old address redirects there.
pub fn scan(dir: &Path) -> Vec<Entry> {
    let mut out: Vec<Entry> = std::fs::read_dir(dir).into_iter().flatten().flatten().filter_map(|e| read_entry(&e.path())).collect();
    let have: std::collections::HashSet<String> = out.iter().map(|e| e.cid.clone()).collect();
    out.retain(|e| djlive::version::of_canonical(&e.text) == djlive::version::Version::V2 || !have.contains(&djlive::compile::renamed_cid(&e.text)));
    out.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| a.cid.cmp(&b.cid)));
    out
}

/// [`scan`], cached for a minute (the store grows by shares; a feed a minute late is fine).
pub struct Catalog {
    dir: PathBuf,
    cache: Mutex<Option<(Instant, Arc<Vec<Entry>>)>>,
}

impl Catalog {
    pub fn new(dir: PathBuf) -> Catalog {
        Catalog { dir, cache: Mutex::new(None) }
    }

    pub fn entries(&self) -> Arc<Vec<Entry>> {
        let mut c = self.cache.lock().unwrap();
        if let Some((at, e)) = c.as_ref() {
            if at.elapsed() < Duration::from_secs(60) {
                return e.clone();
            }
        }
        let e = Arc::new(scan(&self.dir));
        *c = Some((Instant::now(), e.clone()));
        e
    }
}

/// RFC 3339 UTC, seconds.
pub fn rfc3339(t: SystemTime) -> String {
    crate::playlists::model::rfc3339(t.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64))
}

fn xml(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            c if (c as u32) < 0x20 && c != '\n' && c != '\t' => {}
            c => o.push(c),
        }
    }
    o
}

/// The name a person sees: the program's, else "BeatScript" and the end of its CID (as the pages say it).
pub fn title_of(e: &Entry) -> String {
    if e.name.is_empty() {
        format!("BeatScript {}", &e.cid[e.cid.len().saturating_sub(6)..])
    } else {
        e.name.clone()
    }
}

/// A program's URLs, one per format.
pub struct Urls {
    pub page: String,
    pub beatscript: String,
    pub jsonld: String,
    pub render: String,
}

pub fn urls(origin: &str, cid: &str) -> Urls {
    Urls { page: format!("{origin}/p/{cid}"), beatscript: format!("{origin}/p/{cid}.beatscript"), jsonld: format!("{origin}/p/{cid}.jsonld"), render: format!("{origin}/api/render/{cid}?format=opus") }
}

/// The URL of the newest dump's pointer.
pub fn dump_latest(origin: &str) -> String {
    format!("{origin}/dumps/latest.json")
}

/// `/sitemap.xml`: the site's pages and every public CC0 program's page (at most 50,000, the protocol's cap).
pub fn sitemap(origin: &str, entries: &[Entry]) -> String {
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n");
    for p in ["/", "/docs/", "/docs/found-phrases", "/docs/forms", "/docs/radio", "/docs/embed", "/grammar", "/schema", "/authorship", "/phrasebook", "/themes"] {
        s.push_str(&format!("<url><loc>{}{p}</loc></url>\n", xml(origin)));
    }
    for e in entries.iter().take(49_000) {
        s.push_str(&format!("<url><loc>{}</loc><lastmod>{}</lastmod></url>\n", xml(&urls(origin, &e.cid).page), xml(&e.created)));
    }
    s.push_str("</urlset>\n");
    s
}

/// How many programs a feed lists (the newest).
pub const FEED_ITEMS: usize = 50;

/// `/feed.json`: JSON Feed 1.1 of the newest public CC0 programs. Each item is a program: its page, its text, an
/// attachment per format (the `.beatscript`, the JSON-LD, the audio render); `_beatscript` carries the CID and licence.
pub fn json_feed(origin: &str, entries: &[Entry]) -> Value {
    let items: Vec<Value> = entries
        .iter()
        .take(FEED_ITEMS)
        .map(|e| {
            let u = urls(origin, &e.cid);
            let mut it = json!({
                "id": u.page,
                "url": u.page,
                "title": title_of(e),
                "content_text": e.text,
                "date_published": e.created,
                "attachments": [
                    {"url": u.beatscript, "mime_type": "text/beatscript", "title": "the program (.beatscript, canonical text)"},
                    {"url": u.jsonld, "mime_type": "application/ld+json", "title": "the program as linked data"},
                    {"url": u.render, "mime_type": "audio/ogg", "title": "the program rendered (Ogg Opus)"},
                ],
                "_beatscript": {"cid": e.cid, "license": "https://creativecommons.org/publicdomain/zero/1.0/", "licenseId": "CC0-1.0"},
            });
            if !e.author.is_empty() {
                it["authors"] = json!([{"name": e.author}]);
            }
            if !e.forked_from.is_empty() {
                it["_beatscript"]["forkedFrom"] = json!(e.forked_from);
            }
            it
        })
        .collect();
    json!({
        "version": "https://jsonfeed.org/version/1.1",
        "title": "BeatScript: new public programs",
        "home_page_url": format!("{origin}/"),
        "feed_url": format!("{origin}/feed.json"),
        "description": "The newest public BeatScript programs, all CC0 (public domain). Every program is a CID: its text, its linked data and its render are at /p/<CID>.beatscript, /p/<CID>.jsonld and /api/render/<CID>. All of them at once: the dump.",
        "language": "en",
        "_beatscript": {"license": "https://creativecommons.org/publicdomain/zero/1.0/", "dump": dump_latest(origin), "sitemap": format!("{origin}/sitemap.xml"), "atom": format!("{origin}/feed.atom"), "schema": format!("{origin}/schema")},
        "items": items,
    })
}

/// `/feed.atom`: the same as Atom 1.0.
pub fn atom_feed(origin: &str, entries: &[Entry]) -> String {
    let updated = entries.first().map_or_else(|| "1970-01-01T00:00:00Z".to_string(), |e| e.created.clone());
    let o = xml(origin);
    let mut s = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<feed xmlns=\"http://www.w3.org/2005/Atom\">\n<title>BeatScript: new public programs</title>\n<id>{o}/feed.atom</id>\n<link rel=\"self\" type=\"application/atom+xml\" href=\"{o}/feed.atom\"/>\n<link rel=\"alternate\" type=\"text/html\" href=\"{o}/\"/>\n<link rel=\"alternate\" type=\"application/feed+json\" href=\"{o}/feed.json\"/>\n<link rel=\"related\" type=\"application/json\" href=\"{}\"/>\n<rights>CC0 1.0: https://creativecommons.org/publicdomain/zero/1.0/</rights>\n<updated>{}</updated>\n",
        xml(&dump_latest(origin)),
        xml(&updated)
    );
    for e in entries.iter().take(FEED_ITEMS) {
        let u = urls(origin, &e.cid);
        s.push_str(&format!("<entry>\n<title>{}</title>\n<id>{}</id>\n<updated>{}</updated>\n<published>{}</published>\n", xml(&title_of(e)), xml(&u.page), xml(&e.created), xml(&e.created)));
        if !e.author.is_empty() {
            s.push_str(&format!("<author><name>{}</name></author>\n", xml(&e.author)));
        }
        s.push_str(&format!("<link rel=\"alternate\" type=\"text/html\" href=\"{}\"/>\n<link rel=\"enclosure\" type=\"text/beatscript\" href=\"{}\"/>\n<link rel=\"describedby\" type=\"application/ld+json\" href=\"{}\"/>\n<link rel=\"enclosure\" type=\"audio/ogg\" href=\"{}\"/>\n", xml(&u.page), xml(&u.beatscript), xml(&u.jsonld), xml(&u.render)));
        s.push_str(&format!("<rights>CC0-1.0</rights>\n<content type=\"text\">{}</content>\n</entry>\n", xml(&e.text)));
    }
    s.push_str("</feed>\n");
    s
}

// ---------- the dump ----------

/// A program's block document (djlive progld), for its `.jsonld` in a dump and at `/p/<cid>.jsonld?block`.
pub fn block_json(e: &Entry, ontology: Option<&dyn djlive::linked::Ontology>) -> Option<String> {
    let c = djlive::compile::check(&e.text).compiled?;
    let docs: Vec<djlive::doc::DocComment> = e
        .docs
        .iter()
        .filter(|(k, _, _)| k == "section")
        .map(|(_, n, d)| djlive::doc::DocComment { kind: "section", name: n.clone(), doc: d.clone(), item: djlive::Span::new(0, 0), comment: djlive::Span::new(0, 0) })
        .collect();
    let f = djlive::progld::Facts {
        docs: &docs,
        based_on: &e.based_on,
        name: Some(e.name.as_str()).filter(|s| !s.is_empty()),
        license: Some("CC0-1.0"),
        author: Some(e.author.as_str()).filter(|s| !s.is_empty()),
        forked_from: Some(e.forked_from.as_str()).filter(|s| !s.is_empty()),
        ontology,
        ..Default::default()
    };
    Some(djlive::progld::document(&c, &f).js_stringified())
}

/// The dump: the archive bytes (`.tar.gz`), its name (`beatscript-cc0-<sha256 prefix>.tar.gz`, from its bytes), its full
/// sha256 and its manifest (which is in the archive too). The manifest says what the archive holds and nothing about
/// when it was made, so the same programs are the same bytes, the same name: a dump that changes nothing is a no-op.
pub struct Dump {
    pub name: String,
    pub sha256: String,
    pub bytes: Vec<u8>,
    pub manifest: Value,
}

/// The dump of `entries` (sorted by CID inside). Fields follow beats' snapshot manifest (`version`, `count`, `cids`,
/// `docs`), with `@type` its own.
pub fn build_dump(origin: &str, entries: &[Entry], ontology: Option<&dyn djlive::linked::Ontology>) -> Dump {
    let mut es: Vec<&Entry> = entries.iter().collect();
    es.sort_by(|a, b| a.cid.cmp(&b.cid));
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut cids = Vec::new();
    for e in es {
        let Some(ld) = block_json(e, ontology) else { continue };
        files.push((format!("programs/{}.beatscript", e.cid), e.text.clone().into_bytes()));
        files.push((format!("programs/{}.jsonld", e.cid), ld.into_bytes()));
        cids.push(json!(e.cid));
    }
    let manifest = json!({
        "@context": ["https://beatscript.app/schema", "https://schema.org"],
        "@type": ["ProgramDump", "Dataset"],
        "version": 1,
        "name": "BeatScript public programs (CC0)",
        "license": "https://creativecommons.org/publicdomain/zero/1.0/",
        "licenseId": "CC0-1.0",
        "count": cids.len(),
        "layout": {"text": "programs/<cid>.beatscript", "jsonld": "programs/<cid>.jsonld"},
        "cids": cids,
        "docs": format!("{origin}/docs/open-data"),
    });
    let mut man = serde_json::to_string_pretty(&manifest).unwrap();
    man.push('\n');
    files.insert(0, ("manifest.jsonld".to_string(), man.into_bytes()));
    let tar = tar(&files);
    let gz = gzip(&tar);
    let sha = hex::encode(Sha256::digest(&gz));
    Dump { name: format!("beatscript-cc0-{}.tar.gz", &sha[..16]), sha256: sha, bytes: gz, manifest }
}

/// A POSIX ustar archive of regular files, every header the same but for name and size (mode 0644, uid/gid 0, mtime 0,
/// no user names): the bytes are a function of the files alone. A name over the 100 bytes of the name field is split at a
/// `/` into ustar's prefix field (155 bytes) and the name: `programs/<cid>.beatscript` with the longest CID the store takes
/// (81 characters) is 101 bytes.
pub fn tar(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, data) in files {
        let mut h = [0u8; 512];
        let (prefix, rest) = tar_split(name);
        h[..rest.len()].copy_from_slice(rest.as_bytes());
        h[345..345 + prefix.len()].copy_from_slice(prefix.as_bytes());
        let octal = |h: &mut [u8; 512], at: usize, len: usize, v: u64| {
            let s = format!("{:0w$o}\0", v, w = len - 1);
            h[at..at + len].copy_from_slice(s.as_bytes());
        };
        octal(&mut h, 100, 8, 0o644);
        octal(&mut h, 108, 8, 0);
        octal(&mut h, 116, 8, 0);
        octal(&mut h, 124, 12, data.len() as u64);
        octal(&mut h, 136, 12, 0);
        h[148..156].copy_from_slice(b"        ");
        h[156] = b'0';
        h[257..263].copy_from_slice(b"ustar\0");
        h[263..265].copy_from_slice(b"00");
        let sum: u32 = h.iter().map(|b| u32::from(*b)).sum();
        let s = format!("{sum:06o}\0 ");
        h[148..156].copy_from_slice(s.as_bytes());
        out.extend_from_slice(&h);
        out.extend_from_slice(data);
        out.resize(out.len().div_ceil(512) * 512, 0);
    }
    out.resize(out.len() + 1024, 0);
    out
}

/// A path as ustar's (prefix, name): the name alone when it fits the 100-byte field, else split at the last `/` that leaves
/// a name of at most 100 bytes and a prefix of at most 155.
fn tar_split(path: &str) -> (&str, &str) {
    if path.len() <= 100 {
        return ("", path);
    }
    let at = path.char_indices().filter(|&(i, c)| c == '/' && i <= 155 && path.len() - i - 1 <= 100).map(|(i, _)| i).last();
    match at {
        Some(i) if i > 0 => (&path[..i], &path[i + 1..]),
        _ => panic!("tar name too long for ustar: {path}"),
    }
}

/// gzip (RFC 1952) around raw DEFLATE: no name, mtime 0, OS 255, so the bytes depend on the input alone.
pub fn gzip(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 255];
    out.extend(miniz_oxide::deflate::compress_to_vec(data, 9));
    out.extend_from_slice(&crc32(data).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out
}

/// CRC-32 (IEEE), for gzip's trailer.
pub fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for b in data {
        c ^= u32::from(*b);
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

/// Write `d` into `dir` as `<name>` (once: the name is its content) and point `latest.json` at it, with the time it was
/// made and the previous dumps kept. Returns the pointer's JSON.
pub fn write_dump(dir: &Path, origin: &str, d: &Dump, now: SystemTime) -> std::io::Result<Value> {
    std::fs::create_dir_all(dir)?;
    let file = dir.join(&d.name);
    if !file.exists() {
        let tmp = dir.join(format!(".{}.tmp", d.name));
        std::fs::write(&tmp, &d.bytes)?;
        std::fs::rename(&tmp, &file)?;
    }
    let latest_path = dir.join("latest.json");
    let prev: Value = std::fs::read(&latest_path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null);
    let created = if prev["name"] == json!(d.name) { prev["createdAt"].clone() } else { json!(rfc3339(now)) };
    let latest = json!({
        "@context": ["https://beatscript.app/schema", "https://schema.org"],
        "@type": ["ProgramDump", "Dataset"],
        "name": d.name,
        "url": format!("{origin}/dumps/{}", d.name),
        "sha256": d.sha256,
        "bytes": d.bytes.len(),
        "count": d.manifest["count"],
        "createdAt": created,
        "license": "https://creativecommons.org/publicdomain/zero/1.0/",
        "licenseId": "CC0-1.0",
        "docs": format!("{origin}/docs/open-data"),
    });
    let tmp = dir.join(".latest.json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&latest).unwrap())?;
    std::fs::rename(&tmp, &latest_path)?;
    Ok(latest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(dir: &Path, text: &str, meta: Value) -> String {
        let cid = djlive::compile::program_cid(text);
        std::fs::write(dir.join(format!("{cid}.json")), serde_json::to_vec(&json!({"cid": cid, "text": text, "meta": meta})).unwrap()).unwrap();
        cid
    }

    #[test]
    fn a_renamed_program_is_listed_once_as_its_beatscript_2_twin() {
        let d = tempfile::tempdir().unwrap();
        let old = put(d.path(), "tempo 120\ndeck A: kick x ~ x ~\n", json!({"created": "2026-11-20T10:00:00Z"}));
        let twin = put(d.path(), "#! beatscript 2\ntempo 120\ndeck A: kick x ~ x ~\n", json!({"created": "2026-11-20T10:00:00Z", "forked_from": old, "derivation": "renamed"}));
        let alone = put(d.path(), "tempo 121\ndeck A: kick x ~ x ~\n", json!({"created": "2026-11-19T10:00:00Z"}));
        let got: Vec<String> = scan(d.path()).into_iter().map(|e| e.cid).collect();
        assert_eq!(got, [twin, alone], "the old one is not listed beside its twin; a BeatScript 1 program with no twin still is");
    }

    #[test]
    fn only_public_cc0_programs_are_listed() {
        let d = tempfile::tempdir().unwrap();
        let a = put(d.path(), "tempo 120\ndeck A: kick x ~ x ~\n", json!({"license": "CC0-1.0", "name": "a", "created": "2026-11-20T10:00:00Z"}));
        let b = put(d.path(), "tempo 121\ndeck A: kick x ~ x ~\n", json!({"created": "2026-11-21T10:00:00Z"}));
        put(d.path(), "tempo 122\ndeck A: kick x ~ x ~\n", json!({"visibility": "private"}));
        put(d.path(), "tempo 123\ndeck A: kick x ~ x ~\n", json!({"license": "CC-BY-4.0"}));
        put(d.path(), "tempo 124\ndeck A: kick x ~ x ~\n", json!({"license": "all-rights-reserved"}));
        // a file whose text is not its name's
        let other = djlive::compile::program_cid("tempo 99\n");
        std::fs::write(d.path().join(format!("{other}.json")), serde_json::to_vec(&json!({"cid": other, "text": "tempo 98\n"})).unwrap()).unwrap();
        std::fs::write(d.path().join("not-a-cid.json"), b"{}").unwrap();
        let got: Vec<String> = scan(d.path()).into_iter().map(|e| e.cid).collect();
        assert_eq!(got, [b.clone(), a.clone()], "newest first, and nothing else");
        let es = scan(d.path());
        let sm = sitemap("https://beatscript.app", &es);
        assert!(sm.contains(&format!("<loc>https://beatscript.app/p/{a}</loc><lastmod>2026-11-20T10:00:00Z</lastmod>")));
        assert_eq!(sm.matches("<url>").count(), 11 + 2);
        assert!(sm.contains("<url><loc>https://beatscript.app/docs/forms</loc></url>") && sm.contains("<url><loc>https://beatscript.app/themes</loc></url>"));
        assert!(sm.contains("<url><loc>https://beatscript.app/docs/found-phrases</loc></url>"));
        assert!(sm.contains("<url><loc>https://beatscript.app/phrasebook</loc></url>"));
        assert!(sm.contains("<url><loc>https://beatscript.app/grammar</loc></url>"));
        assert!(sm.contains("<url><loc>https://beatscript.app/authorship</loc></url>"));
        let jf = json_feed("https://beatscript.app", &es);
        assert_eq!(jf["items"].as_array().unwrap().len(), 2);
        assert_eq!(jf["items"][1]["title"], "a");
        assert_eq!(jf["items"][1]["attachments"][0]["url"], format!("https://beatscript.app/p/{a}.beatscript"));
        assert_eq!(jf["items"][0]["title"], format!("BeatScript {}", &b[b.len() - 6..]));
        let at = atom_feed("https://beatscript.app", &es);
        assert_eq!(at.matches("<entry>").count(), 2);
        assert!(at.contains(&format!("<link rel=\"enclosure\" type=\"text/beatscript\" href=\"https://beatscript.app/p/{a}.beatscript\"/>")));
        assert_eq!(jf["items"][1]["attachments"][0]["mime_type"], "text/beatscript");
        assert!(at.contains("<updated>2026-11-21T10:00:00Z</updated>"));
    }

    #[test]
    fn the_dump_is_content_addressed_and_holds_every_program_twice() {
        let d = tempfile::tempdir().unwrap();
        let a = put(d.path(), "tempo 120\ndeck A: kick x ~ x ~\n", json!({"name": "a", "created": "2026-11-20T10:00:00Z"}));
        let b = put(d.path(), "tempo 121\nkey am\ndeck A: kick x ~ x ~\n", json!({"created": "2026-11-21T10:00:00Z"}));
        put(d.path(), "tempo 122\ndeck A: kick x\n", json!({"visibility": "private"}));
        let es = scan(d.path());
        let one = build_dump("https://beatscript.app", &es, None);
        let again = build_dump("https://beatscript.app", &scan(d.path()), None);
        assert_eq!(one.bytes, again.bytes, "the same programs are the same bytes");
        assert_eq!(one.name, again.name);
        assert_eq!(one.name, format!("beatscript-cc0-{}.tar.gz", &one.sha256[..16]));
        assert_eq!(one.manifest["count"], 2);
        let mut cids = vec![a.clone(), b.clone()];
        cids.sort();
        assert_eq!(one.manifest["cids"], json!(cids));
        // the archive: gunzip and read the tar back
        let raw = miniz_oxide::inflate::decompress_to_vec(&one.bytes[10..one.bytes.len() - 8]).unwrap();
        assert_eq!(crc32(&raw).to_le_bytes(), one.bytes[one.bytes.len() - 8..one.bytes.len() - 4]);
        let mut names = Vec::new();
        let mut at = 0;
        while at + 512 <= raw.len() && raw[at] != 0 {
            let h = &raw[at..at + 512];
            let name = tar_name(h);
            let size = usize::from_str_radix(std::str::from_utf8(&h[124..135]).unwrap(), 8).unwrap();
            let body = &raw[at + 512..at + 512 + size];
            if let Some(cid) = name.strip_prefix("programs/").and_then(|n| n.strip_suffix(".beatscript")) {
                assert_eq!(djlive::compile::program_cid(std::str::from_utf8(body).unwrap()), cid, "the text is its CID's");
            }
            if let Some(cid) = name.strip_prefix("programs/").and_then(|n| n.strip_suffix(".jsonld")) {
                let r = djlive::progld::read_document(std::str::from_utf8(body).unwrap()).unwrap();
                assert_eq!(r.cid, cid);
            }
            names.push(name);
            at += 512 + size.div_ceil(512) * 512;
        }
        assert_eq!(names[0], "manifest.jsonld");
        assert_eq!(names.len(), 1 + 2 * 2);
        // written once, and the pointer keeps its time while nothing changes
        let out = tempfile::tempdir().unwrap();
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_795_000_000);
        let l1 = write_dump(out.path(), "https://beatscript.app", &one, t0).unwrap();
        let l2 = write_dump(out.path(), "https://beatscript.app", &again, t0 + Duration::from_secs(3600)).unwrap();
        assert_eq!(l1["createdAt"], l2["createdAt"]);
        assert_eq!(l1["url"], format!("https://beatscript.app/dumps/{}", one.name));
        assert!(out.path().join(&one.name).exists());
        // a new program: a new archive, a new name
        put(d.path(), "tempo 140\ndeck A: kick x x x x\n", json!({}));
        let two = build_dump("https://beatscript.app", &scan(d.path()), None);
        assert_ne!(two.name, one.name);
    }

    /// A ustar header's path: its prefix field, a `/`, its name field (the name alone when there is no prefix).
    fn tar_name(h: &[u8]) -> String {
        let field = |b: &[u8]| String::from_utf8(b.iter().take_while(|c| **c != 0).copied().collect()).unwrap();
        let (prefix, name) = (field(&h[345..500]), field(&h[..100]));
        if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        }
    }

    /// The longest CID the store takes (`z` and 80 characters) names a 101-byte entry, `programs/<cid>.beatscript`: it goes
    /// into ustar's prefix and name fields and reads back whole; a name that fits stays in the name field alone.
    #[test]
    fn the_longest_cid_fits_the_archive() {
        let cid = format!("z{}", "a".repeat(80));
        assert!(is_cid(&cid));
        let long = format!("programs/{cid}.beatscript");
        assert_eq!(long.len(), 101);
        let short = "programs/zShort.beatscript".to_string();
        let t = tar(&[(long.clone(), b"tempo 120\n".to_vec()), (short.clone(), b"x".to_vec())]);
        assert_eq!(tar_name(&t[..512]), long);
        assert_eq!(&t[345..353], b"programs");
        assert_eq!(tar_name(&t[1024..1536]), short);
        assert!(t[1024 + 345..1024 + 500].iter().all(|b| *b == 0), "a name that fits has no prefix");
        // the checksum still covers the header (prefix included), as a reader checks it
        let h = &t[..512];
        let sum: u32 = h.iter().enumerate().map(|(i, b)| if (148..156).contains(&i) { 32 } else { u32::from(*b) }).sum();
        assert_eq!(u32::from_str_radix(std::str::from_utf8(&h[148..154]).unwrap(), 8).unwrap(), sum);
    }

    /// The `# from:` lines a save kept (`meta.based_on`) are in the dump's JSON-LD as vendored `isBasedOn`: the text is canonical.
    #[test]
    fn the_dumps_json_ld_says_what_a_program_vendored() {
        let d = tempfile::tempdir().unwrap();
        let pin = "z4EBG9jBATRr9XWGL7beKXF7dm5Je1rPzaSymt6pvg12kd1nqHf";
        let line = format!("{pin}.drop -> {pin}");
        let cid = put(d.path(), "tempo 120\ndeck A: kick x ~ x ~\n", json!({"license": "CC0-1.0", "based_on": [line]}));
        let plain = put(d.path(), "tempo 121\ndeck A: kick x ~ x ~\n", json!({"license": "CC0-1.0"}));
        let es = scan(d.path());
        let e = es.iter().find(|e| e.cid == cid).unwrap();
        assert_eq!(e.based_on, [line]);
        let j = block_json(e, None).unwrap();
        assert!(j.contains(&format!(r#""isBasedOn":[{{"@type":"CreativeWork","name":"{pin}.drop","resolves":"{pin}","derivation":"vendored"}}]"#)), "{j}");
        let p = es.iter().find(|e| e.cid == plain).unwrap();
        assert!(p.based_on.is_empty() && !block_json(p, None).unwrap().contains("isBasedOn"));
    }

    #[test]
    fn gzip_and_tar_are_standard() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        let t = tar(&[("a.txt".into(), b"hi".to_vec())]);
        assert_eq!(t.len(), 512 + 512 + 1024);
        assert_eq!(&t[257..262], b"ustar");
        let g = gzip(&t);
        assert_eq!(&g[..3], &[0x1f, 0x8b, 8]);
    }
}
