//! The `artifact` tool: one BeatScript program as a self-contained HTML page, for a client to publish as it is (a
//! claude.ai Artifact, or any page host), built on pflow.xyz's `pflow_artifact` (pflow-xyz cmd/webserver/mcp_artifact.go).
//!
//! The page carries the program's text and its CID, and plays it with `<beatscript-track>` from
//! `@stackdump/beatscript-player`, loaded from jsDelivr at one exact version with a Subresource Integrity hash and
//! `crossorigin="anonymous"`. The version and every file's hash are in `player-cdn.lock.json`; an import map gives the
//! modules the entry imports their hashes too, so a changed file anywhere in the module graph refuses to run. An
//! Artifact's CSP lets a page load scripts from cdn.jsdelivr.net (and the player needs `'wasm-unsafe-eval'`, which the
//! artifact probe found allowed); nothing is fetched from beatscript.app. Sound starts only from the Play button (the
//! element makes its AudioContext on that click). The page shows the CID and the program's rights and links back to
//! `<site>/p/<CID>`.
//!
//! Read-only, no sign-in: it stores nothing. Billed as a check (it checks the program, so the CID is the engine's).

use base64::Engine as _;
use serde_json::{json, Value};

use super::{bad, is_cid, name_ok, needs_fixing, private, q, sprop, tool, Args, Caller, Fail, Rep, Text, Tools, CID_PAT, NAME_PAT};

const LOCK: &str = include_str!("player-cdn.lock.json");
/// The station page's pin (`<beatscript-radio>`, from 0.4.0): a lock of its own, so that the radio moves to a new
/// version of the package without moving `<beatscript-track>`'s pages (and the VS Code and Jupyter copies of that lock).
const RADIO_LOCK: &str = include_str!("radio-cdn.lock.json");
/// The radio element's file in the package: the entry of a station page (radio-cdn.lock.json's).
pub const RADIO_ENTRY: &str = "beatscript-radio.js";
const JSDELIVR: &str = "https://cdn.jsdelivr.net/npm/";
/// audio "inline": the most base64 a page carries (above it, the page plays with the engine instead).
pub const MAX_INLINE_B64: usize = 6 << 20;
/// How long a call waits for a render that is not cached yet.
const RENDER_WAIT: std::time::Duration = std::time::Duration::from_secs(120);
/// What the summary says of AAC (Safari and iOS play no Ogg Opus everywhere).
const AAC_NOTE: &str = "beatscript-serve renders Ogg Opus and WAV only; it has no AAC/m4a encoder, so there is no m4a render. A browser without Ogg Opus (older Safari) gets audio \"engine\" or beatscript.app.";

/// The pinned `@stackdump/beatscript-player` the page loads (`player-cdn.lock.json`).
pub struct PlayerCdn {
    pub package: String,
    pub version: String,
    pub entry: String,
    /// (file in the package, SRI integrity), in the lock's order (sorted by file).
    pub files: Vec<(String, String)>,
}

impl PlayerCdn {
    /// `<beatscript-track>`'s pin (player-cdn.lock.json).
    pub fn get() -> &'static PlayerCdn {
        static P: std::sync::OnceLock<PlayerCdn> = std::sync::OnceLock::new();
        P.get_or_init(|| Self::parse(LOCK, "player-cdn.lock.json"))
    }

    /// `<beatscript-radio>`'s pin (radio-cdn.lock.json): its entry is [`RADIO_ENTRY`].
    pub fn radio() -> &'static PlayerCdn {
        static P: std::sync::OnceLock<PlayerCdn> = std::sync::OnceLock::new();
        P.get_or_init(|| Self::parse(RADIO_LOCK, "radio-cdn.lock.json"))
    }

    fn parse(lock: &str, name: &str) -> PlayerCdn {
        let v: Value = serde_json::from_str(lock).unwrap_or_else(|_| panic!("{name}"));
        let s = |k: &str| v[k].as_str().unwrap_or_else(|| panic!("{name}: {k}")).to_string();
        let files = v["files"].as_object().unwrap_or_else(|| panic!("{name}: files")).iter().map(|(f, x)| (f.clone(), x["integrity"].as_str().expect("integrity").to_string())).collect();
        PlayerCdn { package: s("package"), version: s("version"), entry: s("entry"), files }
    }

    /// The jsDelivr URL of one file of the pinned version.
    pub fn url(&self, file: &str) -> String {
        format!("{JSDELIVR}{}@{}/{file}", self.package, self.version)
    }

    pub fn integrity(&self, file: &str) -> &str {
        self.files.iter().find(|(f, _)| f == file).map(|(_, i)| i.as_str()).unwrap_or("")
    }

    /// The JavaScript modules the entry can import (every `.js` but the entry and the UMD wrapper).
    fn modules(&self) -> impl Iterator<Item = &(String, String)> {
        self.modules_of(&self.entry)
    }

    /// The JavaScript modules `entry` can import: every `.js` but it and the UMD wrapper (the radio's entry imports
    /// beatscript-track.js, for the module loader the two elements share).
    fn modules_of<'a>(&'a self, entry: &'a str) -> impl Iterator<Item = &'a (String, String)> + 'a {
        self.files.iter().filter(move |(f, _)| f.ends_with(".js") && f != entry && !f.ends_with(".umd.js"))
    }

    /// The import map and the entry's script tag, in that order (an import map must come before any module script).
    pub fn tags(&self) -> String {
        self.tags_for(&self.entry)
    }

    /// [`Self::tags`] with another file of the package as the entry (`beatscript-radio.js`, [`RADIO_ENTRY`]).
    pub fn tags_for(&self, entry: &str) -> String {
        let mut m = serde_json::Map::new();
        for (f, i) in self.modules_of(entry) {
            m.insert(self.url(f), json!(i));
        }
        let map = serde_json::to_string(&json!({"integrity": m})).unwrap();
        format!(
            "<script type=\"importmap\">{}</script>\n<script type=\"module\" src=\"{}\" integrity=\"{}\" crossorigin=\"anonymous\"></script>\n",
            map.replace('<', "\\u003c"),
            self.url(entry),
            self.integrity(entry)
        )
    }
}

/// tools/list's entry.
pub fn tool_def(p: &str, max_prog: usize) -> Value {
    let cdn = PlayerCdn::get();
    let radio = PlayerCdn::radio();
    let desc = format!(
        "One BeatScript program as a self-contained HTML page, for the person to hear and share: publish the returned html as a page (an Artifact) exactly as it is, without rewriting it, and do not hand-roll a player instead. \
The page embeds the program's text and its CID, starts sound only when the person presses Play, shows the CID and the program's rights (licence, credits, forked from), and links back to the program on {p}: {p}/p/<CID> when the site has it stored, else the self-sealing share link (the program is in the link), never a /p/<CID> that would not answer. Nothing is fetched from {p}. With store true a signed-in caller stores a text first, publicly and under its `#! license:` (default CC0-1.0, a Creative Commons licence only), as share does, so the page gets the /p/<CID> link and inline audio. \
Two ways to sound, chosen by audio: \"inline\" (the default for a stored program that renders) carries the site's Ogg Opus render of the program in the page as a data: URI in a plain <audio controls>, so the page loads nothing at all (no script, no engine); it is the offline render (notes and sounds, not control lines), at most {inline_mb} MB as base64, and a longer render, a text that is not stored, or a render that does not finish falls back to engine with audio_note saying why. \"engine\" (the default for a text) plays it live with the <beatscript-track> player ({pkg}@{ver}, the same engine as the editor, loaded from cdn.jsdelivr.net at that exact version with a Subresource Integrity hash for every module it loads); the host must allow scripts from cdn.jsdelivr.net and WebAssembly ('wasm-unsafe-eval'), as a claude.ai Artifact does. Ogg Opus does not play in older Safari; there is no AAC render. \
Give exactly one of text (the program; it must pass check; not stored unless store), cid (a program stored here or an example's CID: the page carries the stored rights), example (a name from list_examples) or station. title names the page (default the program's `#! name:`, else its stored name). \
station makes a radio page instead: a station (docs/beatscript/radio.md; its program text, the CID of a station stored here, or the name of an example that is a station) played endlessly by <beatscript-radio> ({pkg}@{rver}, pinned the same way), from position n (default 0): transport, now playing, section and bar, a level meter, the playlist of the station's next tracks, a CC0 bridge loop when the next track is not ready (never dead air), and a peek drawer with the playing program's text. The station's tracks are computed in the page by the engine (no server, nothing fetched), so the page is the station's text and its CID; sound starts only from Play. Its summary is {{title, station: {{cid, name, genre, tempo, key, step, forms}}, n, player, html_bytes, publish}}; audio, store and the other sources do not go with it. \
Answers two text blocks: the first a JSON summary {{title, cid, canonical_bytes, stored, rights, link, short_url, audio, player, render, audio_note, store_hint, html_bytes, publish}}, the second the page (a whole HTML document). A program that uses samples or loops, or says `use restricted`, has nothing to play in a page and is refused. \
No sign-in, and it stores nothing, unless store; billed as a check (a render not cached yet is limited as render's are, a store as share's). Text at most {max_prog} bytes.",
        inline_mb = MAX_INLINE_B64 >> 20,
        pkg = cdn.package,
        ver = cdn.version,
        rver = radio.version,
    );
    tool(
        "artifact",
        "A program as a page to publish",
        desc,
        [false, false, true, false],
        vec![
            ("text", sprop("The program text. It must pass check.", &[("maxLength", json!(max_prog))])),
            ("store", json!({"type": "boolean", "description": "With text: store it on the site first (public, its `#! license:` or CC0-1.0; needs a signed-in caller), as share does, so the page links /p/<CID> and can carry the render inline (default false: the page links the self-sealing link instead)."})),
            ("cid", sprop("CID of a program stored here, or of an example: the page carries the rights it was stored with.", &[("maxLength", json!(100)), ("pattern", json!(CID_PAT))])),
            ("example", sprop("Name of an example, from list_examples.", &[("maxLength", json!(64)), ("pattern", json!(NAME_PAT))])),
            ("audio", sprop("inline (the site's Opus render in the page, nothing loaded; default for a stored program) or engine (the live player from jsDelivr; default for a text).", &[("enum", json!(["inline", "engine"]))])),
            ("title", sprop("The page's title and the name to publish it under (default the program's `#! name:`, else its stored name, else \"BeatScript\").", &[("maxLength", json!(80))])),
            ("station", sprop("A radio page instead of one program: the station's program text (a `station { }` block, docs/beatscript/radio.md), the CID of a station stored here, or the name of an example that is a station. Not with text, cid, example, audio or store.", &[("maxLength", json!(max_prog))])),
            ("n", json!({"type": "integer", "minimum": 0, "description": "With station: the position to start at (default 0)."})),
        ],
        &[],
    )
}

/// A query value, percent-encoded (RFC 3986 unreserved characters kept).
fn urlencoding(s: &str) -> String {
    let mut o = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}

/// `#! <key>: X` of a text.
fn header(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|l| l.trim().strip_prefix("#!").map(str::trim).and_then(|h| h.strip_prefix(key)).and_then(|h| h.strip_prefix(':')).map(|n| n.trim().to_string()))
}

/// `#! name: X` of a text, else "".
fn header_name(text: &str) -> String {
    header(text, "name").unwrap_or_default()
}

/// One line, control characters gone, at most `max` characters.
fn clean(s: &str, max: usize) -> String {
    let t: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    t.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(max).collect()
}

/// HTML text and attribute escaping (quotes included, so it is safe in a double-quoted attribute).
pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c => o.push(c),
        }
    }
    o
}

const STYLE: &str = "body{margin:0;background:#0d1326;color:#eceef6;font:15px/1.45 system-ui,-apple-system,\"Segoe UI\",Roboto,sans-serif}\
main{max-width:560px;margin:0 auto;padding:24px 16px}h1{font-size:20px;margin:0 0 14px}a{color:#88aaff}\
beatscript-track{margin:0 0 14px}beatscript-track:not(:defined){display:block;padding:14px 16px;border:1px solid #243055;border-radius:4px;color:#8d95b0}\
beatscript-track:not(:defined)::before{content:\"Loading the player from cdn.jsdelivr.net\\2026  If this stays, this page's host blocks it: open the program on beatscript.app instead.\"}\
dl{display:grid;grid-template-columns:auto 1fr;gap:4px 12px;margin:0 0 14px;font-size:14px}dt{color:#8d95b0}dd{margin:0;min-width:0;overflow-wrap:anywhere}\
code{font:13px ui-monospace,SFMono-Regular,Menlo,monospace}details{margin:0 0 14px}summary{cursor:pointer;color:#8d95b0}\
pre{background:#16213e;border:1px solid #243055;border-radius:4px;padding:10px 12px;overflow:auto;font:13px/1.4 ui-monospace,SFMono-Regular,Menlo,monospace}\
.small{font-size:12px;color:#8d95b0}audio{display:block;width:100%;margin:0 0 14px}";

pub struct Page {
    pub title: String,
    pub cid: String,
    /// What the element plays: the canonical text, under a `#! name:` line when there is a name (a header line: the
    /// CID is of the canonical text, so it is unchanged).
    pub text: String,
    pub rights: Value,
    /// Where "Open on beatscript.app" goes: <site>/p/<CID> for a stored program, else the self-sealing link.
    pub link: String,
    pub site: String,
    pub stored: bool,
    /// audio "inline": the Opus render, embedded as a data: URI in a plain <audio controls> (no script at all); None:
    /// the live <beatscript-track> from jsDelivr.
    pub opus: Option<Vec<u8>>,
}

impl Page {
    pub fn html(&self) -> String {
        let cdn = PlayerCdn::get();
        let r = &self.rights;
        let lic = r["license"].as_str().unwrap_or("CC0-1.0");
        let mut rights = format!("<a href=\"https://spdx.org/licenses/{l}.html\" rel=\"license noopener\" target=\"_blank\">{l}</a>", l = esc(lic));
        if r["source"] == "text" {
            rights.push_str(" (as the text says, else BeatScript's default; this text is not stored on the site)");
        }
        let mut rows = format!(
            "<dt>CID</dt><dd><a href=\"{link}\" target=\"_blank\" rel=\"noopener\"><code>{cid}</code></a></dd>\n<dt>Rights</dt><dd>{rights}</dd>\n",
            link = esc(&self.link),
            cid = esc(&self.cid),
        );
        if let Some(cr) = r["credits"].as_array().filter(|c| !c.is_empty()) {
            let list: Vec<String> = cr.iter().filter_map(Value::as_str).map(esc).collect();
            rows.push_str(&format!("<dt>Credits</dt><dd>{}</dd>\n", list.join("; ")));
        }
        if let Some(f) = r["forked_from"].as_str().filter(|f| is_cid(f)) {
            rows.push_str(&format!("<dt>Forked from</dt><dd><a href=\"{site}/p/{f}\" target=\"_blank\" rel=\"noopener\"><code>{f}</code></a></dd>\n", site = esc(&self.site), f = esc(f)));
        }
        let mut open = format!("<a href=\"{}\" target=\"_blank\" rel=\"noopener\">Open on beatscript.app</a>", esc(&self.link));
        if !self.stored {
            open.push_str(" (a self-sealing link: the program is in the link, and opening it checks the CID)");
        }
        let (tags, player, about) = match &self.opus {
            None => (
                cdn.tags(),
                format!("<beatscript-track cid=\"{}\" text=\"{}\"></beatscript-track>", esc(&self.cid), esc(&self.text)),
                format!(
                    "Played in this page by <a href=\"https://www.npmjs.com/package/{pkg}/v/{ver}\" target=\"_blank\" rel=\"noopener\">{pkg}@{ver}</a> from cdn.jsdelivr.net, pinned with Subresource Integrity. Sound starts only when you press Play.",
                    pkg = esc(&cdn.package),
                    ver = esc(&cdn.version)
                ),
            ),
            Some(b) => (
                String::new(),
                format!("<audio controls preload=\"metadata\" src=\"data:audio/ogg;base64,{}\"></audio>", base64::engine::general_purpose::STANDARD.encode(b)),
                "The audio is the site's offline render of this program (Ogg Opus), carried in the page: nothing is loaded. It plays the notes and sounds, not the control lines, so it can sound plainer than the editor. Sound starts only when you press Play. A browser without Ogg Opus (older Safari) can play the program on beatscript.app.".to_string(),
            ),
        };
        format!(
            "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n<meta name=\"beatscript:cid\" content=\"{cid}\">\n<title>{title}</title>\n{tags}<style>{STYLE}</style>\n</head>\n<body>\n<main data-cid=\"{cid}\">\n<h1>{title}</h1>\n\
{player}\n<dl>\n{rows}</dl>\n<p>{open}</p>\n\
<details><summary>The program</summary><pre>{pre}</pre></details>\n\
<p class=\"small\">{about}</p>\n</main>\n</body>\n</html>\n",
            title = esc(&self.title),
            cid = esc(&self.cid),
            pre = esc(&self.text),
        )
    }
}

/// The rules a radio page adds to [`STYLE`].
const RADIO_STYLE: &str = "beatscript-radio{margin:0 0 14px}beatscript-radio:not(:defined){display:block;padding:14px 16px;border:1px solid #243055;border-radius:4px;color:#8d95b0}\
beatscript-radio:not(:defined)::before{content:\"Loading the radio from cdn.jsdelivr.net\\2026  If this stays, this page's host blocks it: open the station on beatscript.app instead.\"}";

/// A station as a page: `<beatscript-radio>` from jsDelivr, the station's text in its `station` attribute and the CID
/// the text must have in `cid` (the element refuses a text that is not that station). No child script: the text is an
/// attribute, escaped, so nothing in it can close a tag.
pub struct StationPage {
    pub title: String,
    pub cid: String,
    pub text: String,
    pub n: u64,
    /// `<site>/radio?s=<text>&n=<n>`: the station on the site (the text is in the link, so it opens whether or not it is stored).
    pub link: String,
}

impl StationPage {
    pub fn html(&self) -> String {
        let cdn = PlayerCdn::radio();
        let n = if self.n > 0 { format!(" n=\"{}\"", self.n) } else { String::new() };
        format!(
            "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n<meta name=\"beatscript:station\" content=\"{cid}\">\n<title>{title}</title>\n{tags}<style>{STYLE}{RADIO_STYLE}</style>\n</head>\n<body>\n<main data-station=\"{cid}\">\n<h1>{title}</h1>\n\
<beatscript-radio station=\"{text}\" cid=\"{cid}\"{n}></beatscript-radio>\n\
<dl>\n<dt>Station</dt><dd><a href=\"{link}\" target=\"_blank\" rel=\"noopener\"><code>{cid}</code></a></dd>\n<dt>Rights</dt><dd><a href=\"https://spdx.org/licenses/CC0-1.0.html\" rel=\"license noopener\" target=\"_blank\">CC0-1.0</a> (a station and every track of it)</dd>\n</dl>\n\
<p><a href=\"{link}\" target=\"_blank\" rel=\"noopener\">Open the station on beatscript.app</a></p>\n\
<details><summary>The station's program</summary><pre>{pre}</pre></details>\n\
<p class=\"small\">Position N of a station is always the same track: its tracks are computed in this page by the engine, and played by <a href=\"https://www.npmjs.com/package/{pkg}/v/{ver}\" target=\"_blank\" rel=\"noopener\">{pkg}@{ver}</a> from cdn.jsdelivr.net, pinned with Subresource Integrity. Sound starts only when you press Play.</p>\n</main>\n</body>\n</html>\n",
            tags = cdn.tags(),
            title = esc(&self.title),
            cid = esc(&self.cid),
            text = esc(&self.text),
            link = esc(&self.link),
            pre = esc(&self.text),
            pkg = esc(&cdn.package),
            ver = esc(&cdn.version),
        )
    }
}

impl Tools {
    /// artifact with `station`: a radio page. The station is answered here once (as the `station` tool answers it), so a text
    /// that is not a station is refused before it is put in a page, and the page carries the station's CID.
    fn station_artifact(&self, c: &Caller, a: &Args, title_arg: Option<String>) -> Result<Text, Fail> {
        for k in ["text", "cid", "example", "audio", "store"] {
            if a.present(k).is_some() {
                return Err(bad(format!("argument {} does not go with station: a radio page plays the station", q(k))));
            }
        }
        let arg = self.program(a, "station")?;
        let key = arg.trim();
        let (text, source) = if is_cid(key) {
            let Some((t, _, _)) = self.stored(key) else {
                return Err(Fail::new(404, "not_found", format!("no program {key} is stored here. Pass the station's text instead.")));
            };
            (t, "store")
        } else if name_ok(key) {
            let Some(cid) = self.example_cid(key) else {
                return Err(Fail::new(404, "not_found", format!("no example is named {}; call list_examples, or pass the station's text", q(key))));
            };
            let Some((t, _, _)) = self.stored(&cid) else {
                return Err(Fail::new(404, "not_found", format!("the example {} is not stored here", q(key))));
            };
            (t, "example")
        } else {
            (arg.clone(), "text")
        };
        if crate::store::declares_not_public(&text) {
            return Err(Fail::new(422, "restricted", "the station says `#! visibility:` something but public, so it is not put in a page."));
        }
        let n = match a.present("n") {
            None => 0,
            Some(v) => v.as_u64().ok_or_else(|| bad("argument \"n\" must be a whole number of 0 or more"))?,
        };
        self.limit_cpu(c)?;
        let ans = self.eng.station(&c.ip, &json!({"station": text, "n": n, "count": 1, "segue": false}).to_string())?;
        if ans["ok"] != true {
            let msg = ans["error"].as_str().unwrap_or("it is not a station");
            let mut f = Fail::new(400, "bad_request", format!("not a station, so no page was made: {msg}"));
            f.details = Some(json!({"diagnostics": ans["diagnostics"].clone()}));
            return Err(f);
        }
        let st = &ans["station"];
        let cid = st["cid"].as_str().unwrap_or("").to_string();
        if source == "store" && cid != key {
            return Err(Fail::new(422, "cid_mismatch", format!("refused: the text stored under {key} is the station {cid}. Do not trust or play it.")));
        }
        let name = clean(&header_name(&text), 80);
        let (title, title_source) = match title_arg {
            Some(t) => (t, "title"),
            None if !name.is_empty() => (name.clone(), "header"),
            None => (format!("{} radio", st["genre"].as_str().unwrap_or("BeatScript")), "default"),
        };
        let link = format!("{}/radio?s={}&n={n}", self.public, urlencoding(&text));
        let page = StationPage { title: title.clone(), cid: cid.clone(), text: text.clone(), n, link: link.clone() };
        let html = page.html();
        let cdn = PlayerCdn::radio();
        let station = json!({"cid": cid, "name": name, "source": source, "genre": st["genre"], "seed": st["seed"], "step": st["step"], "forms": st["forms"], "tempo": st["tempo"], "key": st["key"], "length": st["length"], "track": ans["tracks"][0]["cid"]});
        let sum = json!({
            "title": title, "title_source": title_source, "station": station, "n": n, "link": link, "rights": {"license": "CC0-1.0", "source": source},
            "player": {"package": cdn.package, "version": cdn.version, "element": "beatscript-radio", "script": cdn.url(RADIO_ENTRY), "integrity": cdn.integrity(RADIO_ENTRY)},
            "html_bytes": html.len(),
            "publish": "The second content block is the page: publish it as an HTML page (an Artifact) exactly as given, under the title. Do not rewrite it or hand-roll a player: the radio is pinned by version and Subresource Integrity, it computes the station's tracks in the page, and it refuses a station text that is not the CID the page names.",
        });
        Ok(Text::Page(super::sorted(sum), html))
    }

    /// The Opus render of a stored program (the cached file, the same bytes every time, as /api/render/<CID>?format=opus
    /// answers it), rendering it first when it is not cached: a new render is limited as the render tool's are. Err says
    /// why there is none.
    pub(super) fn inline_opus(&self, c: &Caller, cid: &str) -> Result<Vec<u8>, String> {
        use crate::service::{Acquired, Fmt, Key};
        let (p, _) = self.app.renderable(cid).map_err(|e| e.message.clone())?;
        let key = Key { cid: cid.to_string(), format: Fmt::Opus, sr: 44_100 };
        let read = |path: &std::path::Path| std::fs::read(path).map_err(|e| format!("the cached render could not be read: {e}"));
        if let (Some((path, _)), _) = self.app.rs.peek(&key) {
            return read(&path);
        }
        if self.render.allow(&c.key).and_then(|_| self.ip_ceiling(&self.render_ip, c)).is_err() {
            return Err("too many renders asked for; call again later for inline audio".into());
        }
        let job = match self.app.rs.acquire(&key, if c.ip.is_empty() { "unknown" } else { &c.ip }, &p.text).map_err(|e| e.message.clone())? {
            Acquired::Hit(path, _) => return read(&path),
            Acquired::Job(j, _) => j,
        };
        let t0 = std::time::Instant::now();
        let err = loop {
            let (_, done, err) = job.snapshot();
            if done {
                break err;
            }
            if t0.elapsed() > RENDER_WAIT {
                break Some(format!("the render did not finish in {} s", RENDER_WAIT.as_secs()));
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        };
        self.app.rs.leave(&job);
        if let Some(e) = err {
            return Err(e);
        }
        match self.app.rs.peek(&key) {
            (Some((path, _)), _) => read(&path),
            _ => Err("the render finished but is not in the cache".into()),
        }
    }

    pub(super) fn artifact_tool(&self, c: &Caller, a: Args) -> Result<Text, Fail> {
        a.only(&["text", "cid", "example", "title", "audio", "store", "station", "n"])?;
        if a.present("station").is_some() {
            let title_arg = a.str("title", 80)?.map(|t| clean(&t, 80)).filter(|t| !t.is_empty());
            return self.station_artifact(c, &a, title_arg);
        }
        if a.present("n").is_some() {
            return Err(bad("argument \"n\" is a station's position: it goes with station"));
        }
        let store = a.boolean("store")?;
        let audio = a.str("audio", 8)?.filter(|s| !s.is_empty());
        if audio.as_deref().is_some_and(|m| m != "inline" && m != "engine") {
            return Err(bad("argument \"audio\" is inline or engine"));
        }
        let text_arg = match a.present("text") {
            Some(_) => Some(self.program(&a, "text")?),
            None => None,
        };
        let cid_arg = a.str("cid", 100)?.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let example = a.str("example", 64)?.filter(|s| !s.is_empty());
        let title_arg = a.str("title", 80)?.map(|t| clean(&t, 80)).filter(|t| !t.is_empty());
        if [text_arg.is_some(), cid_arg.is_some(), example.is_some()].iter().filter(|b| **b).count() != 1 {
            return Err(bad("give exactly one of text, cid, example"));
        }
        // the CID a stored program must have
        let mut want_cid = String::new();
        // (the program, its rights, whether the site has it, the name it was stored under)
        let (text, rights, stored, stored_name) = if let Some(t) = text_arg {
            if crate::store::declares_not_public(&t) {
                return Err(Fail::new(422, "restricted", "the program says `#! visibility:` something but public, so it is not put in a page: private programs never leave the machine they were written on."));
            }
            // the rights a text says (`#! license:`), else BeatScript's default
            let lic = header(&t, "license").filter(|l| super::license_ok(l)).unwrap_or_else(|| "CC0-1.0".into());
            (t, json!({"license": lic, "source": "text"}), false, String::new())
        } else {
            let cid = match (cid_arg, example) {
                (Some(cid), _) => {
                    if !is_cid(&cid) {
                        return Err(bad("argument \"cid\" is not a CID (it starts with z)"));
                    }
                    cid
                }
                (None, Some(ex)) => {
                    if !name_ok(&ex) {
                        return Err(bad("argument \"example\" is an example name, as list_examples gives"));
                    }
                    self.example_cid(&ex).ok_or_else(|| Fail::new(404, "not_found", format!("no example is named {}; call list_examples", q(&ex))))?
                }
                (None, None) => unreachable!(),
            };
            want_cid = cid.clone();
            let Some((t, _, meta)) = self.stored(&cid) else {
                return Err(Fail::new(404, "not_found", format!("no program {cid} is stored here. Pass its text instead (artifact takes text), or share it signed in to store it.")));
            };
            super::queue_gate(&t, &meta)?;
            let m = crate::compose::go_meta(&meta);
            let mut r = json!({"license": m["license"].as_str().filter(|l| !l.is_empty()).unwrap_or("CC0-1.0"), "source": if meta["source"] == "example" { "example" } else { "store" }});
            for k in ["credits", "forked_from", "composed_by"] {
                if !m[k].is_null() {
                    r[k] = m[k].clone();
                }
            }
            let name = m["name"].as_str().unwrap_or("").to_string();
            (t, r, true, name)
        };
        let rep = self.check_program(c, &text)?;
        let rr = Rep(&rep);
        if !rr.ok() || rr.cid().is_empty() {
            return Err(needs_fixing(&rep, "no page was made"));
        }
        if let Some(e) = private(&rep, "it has nothing to play in a page") {
            return Err(e);
        }
        let cid = rr.cid().to_string();
        if stored && want_cid != cid {
            return Err(Fail::new(422, "cid_mismatch", format!("refused: the text stored under {want_cid} is program {cid}. Do not trust or play it.")));
        }
        let canonical = rr.canonical().to_string();
        // store true: store the text first, as share does (signed in, the storage gate, share's limits)
        let (mut stored, mut rights) = (stored, rights);
        if store && !stored {
            if c.uid.is_none() {
                return Err(Fail::new(401, "auth_required", format!("store needs a signed-in caller (artifact stores nothing otherwise; leave store out for a page with the self-sealing link). {}", self.auth_help())));
            }
            let lim = self.share.allow(&c.key).and_then(|_| self.ip_ceiling(&self.share_ip, c));
            if let Err(r) = lim {
                return Err(Fail::from(crate::ApiError::new(429, "rate_limited", "too many programs stored; call artifact without store for a page with the self-sealing link").retry(r)));
            }
            let meta = json!({"license": rights["license"], "visibility": "public", "source": "share"});
            let formatted = beatscript_serve::autoformat::autoformat(&text, self.max_prog);
            let (prog, _) = crate::compose::put(&self.app, &self.eng, &c.ip, &formatted, meta)?;
            if prog["cid"] != cid.as_str() {
                return Err(Fail::new(500, "internal", "the program was stored under another CID; nothing is put in a page"));
            }
            stored = true;
            let m = crate::compose::go_meta(&prog["meta"]);
            rights = json!({"license": m["license"].as_str().filter(|l| !l.is_empty()).unwrap_or("CC0-1.0"), "source": "store"});
        }
        let (title, title_source) = match (title_arg, clean(&header_name(&text), 80), clean(&stored_name, 80)) {
            (Some(t), _, _) => (t, "title"),
            (None, h, _) if !h.is_empty() => (h, "header"),
            (None, _, s) if !s.is_empty() => (s, "stored"),
            _ => ("BeatScript".to_string(), "default"),
        };
        let play = if title_source == "default" || header(&canonical, "name").is_some() { canonical.clone() } else { format!("#! name: {title}\n{canonical}") };
        let sealed = match self.eng.share(&c.ip, &canonical) {
            Ok(s) if s["ok"] == true => self.site_links(s["url"].as_str().unwrap_or(""), "").0,
            _ => String::new(),
        };
        let short = format!("{}/p/{cid}", self.public);
        // a /p/<CID> link answers only for a stored program: a text that is not gets the self-sealing link
        let link = if stored { short.clone() } else if sealed.is_empty() { self.public.clone() } else { sealed.clone() };
        // audio: inline (the render in the page) by default for a stored program that renders, else engine
        let mut audio_note = String::new();
        let opus = match (audio.as_deref(), stored) {
            (Some("engine"), _) => None,
            (Some("inline"), false) => {
                audio_note = "audio inline needs a stored program (the render is of a stored CID); this text is not stored, so the page plays it with the engine. Store it (render, or share signed in) and pass its cid for inline audio.".into();
                None
            }
            (_, false) => None,
            (_, true) => match self.inline_opus(c, &cid) {
                Ok(b) if b.len().div_ceil(3) * 4 <= MAX_INLINE_B64 => Some(b),
                Ok(b) => {
                    audio_note = format!("the render is {} bytes, {} as base64, over the {} inline limit, so the page plays the program with the engine instead.", b.len(), b.len().div_ceil(3) * 4, MAX_INLINE_B64);
                    None
                }
                Err(why) => {
                    audio_note = format!("no inline audio ({why}), so the page plays the program with the engine instead.");
                    None
                }
            },
        };
        let mode = if opus.is_some() { "inline" } else { "engine" };
        let opus_bytes = opus.as_ref().map(Vec::len);
        let page = Page { title: title.clone(), cid: cid.clone(), text: play, rights: rights.clone(), link: link.clone(), site: self.public.clone(), stored, opus };
        let html = page.html();
        let cdn = PlayerCdn::get();
        let mut sum = json!({
            "title": title, "title_source": title_source, "cid": cid, "canonical_bytes": canonical.len(), "stored": stored, "rights": rights, "link": link,
            "audio": mode,
            "player": if mode == "engine" { json!({"package": cdn.package, "version": cdn.version, "script": cdn.url(&cdn.entry), "integrity": cdn.integrity(&cdn.entry)}) } else { Value::Null },
            "render": if let Some(n) = opus_bytes { json!({"format": "opus", "container": "ogg", "bytes": n, "url": format!("{}/api/render/{cid}?format=opus", self.public), "aac": AAC_NOTE}) } else { Value::Null },
            "html_bytes": html.len(), "warnings": rr.warnings(),
            "publish": if mode == "engine" { "The second content block is the page: publish it as an HTML page (an Artifact) exactly as given, under the title. Do not rewrite it or hand-roll a player: the page's player is pinned by version and Subresource Integrity, and it checks the program against the CID before it plays." } else { "The second content block is the page: publish it as an HTML page (an Artifact) exactly as given, under the title. Do not rewrite it or re-encode the audio: the render is carried in the page as a data: URI, so it plays with nothing loaded." },
        });
        if !audio_note.is_empty() {
            sum["audio_note"] = json!(audio_note);
        }
        if stored {
            sum["short_url"] = json!(short);
        } else {
            sum["store_hint"] = json!(format!("Not stored on the site, so the page links the self-sealing link (link), never {short}, which would not answer. Signed in, call artifact again with store true (it stores the program publicly under its licence, as share does) for the /p/<CID> link and inline audio."));
        }
        if title_source == "default" {
            sum["title_hint"] = json!("The program has no `#! name:` and no title was given, so the page is called \"BeatScript\": pass title (two to four words) to name it.");
        }
        Ok(Text::Page(super::sorted(sum), html))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// What the HTML escaping in [`esc`] undoes, for reading an attribute back.
    fn unesc(s: &str) -> String {
        s.replace("&quot;", "\"").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
    }

    fn tools() -> (Arc<Tools>, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("btw-mcp-artifact-{}-{:?}", std::process::id(), std::thread::current().id()));
        std::fs::remove_dir_all(&dir).ok();
        let app = crate::test_app(&dir);
        (Arc::new(Tools::new(&app)), dir)
    }

    fn call(t: &Tools, args: Value) -> Result<(Value, String), Fail> {
        call_as(t, None, args)
    }

    fn call_as(t: &Tools, uid: Option<&str>, args: Value) -> Result<(Value, String), Fail> {
        let key = uid.map_or("ip:127.0.0.1".to_string(), |u| format!("u:{u}"));
        let c = Caller { uid: uid.map(str::to_string), ip: "127.0.0.1".into(), key };
        let Value::Object(m) = args else { panic!("args") };
        match t.dispatch("artifact", &c, Args(m))? {
            Text::Page(v, html) => Ok((v, html)),
            _ => panic!("artifact answers a page"),
        }
    }

    /// The value of `attr="..."` on the first `<tag ` in a page.
    fn attr(page: &str, tag: &str, attr: &str) -> Option<String> {
        let i = page.find(&format!("<{tag} "))?;
        let el = &page[i..i + page[i..].find('>')?];
        let k = el.find(&format!(" {attr}=\""))? + attr.len() + 3;
        Some(unesc(&el[k..k + el[k..].find('"')?]))
    }

    const PROG: &str = "#! name: Four on the floor\ntempo 124\ndeck A: kick x ~ x ~ | hihat ~ x ~ x\n";

    #[test]
    fn the_lock_pins_one_exact_version_with_sri() {
        let l = PlayerCdn::get();
        assert_eq!(l.package, "@stackdump/beatscript-player");
        let parts: Vec<&str> = l.version.split('.').collect();
        assert!(parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())), "version {:?} is not an exact x.y.z (never a range, a tag or latest)", l.version);
        assert!(l.files.iter().any(|(f, _)| *f == l.entry), "the lock has no integrity for its entry {}", l.entry);
        for (f, i) in &l.files {
            let b64 = i.strip_prefix("sha384-").unwrap_or_else(|| panic!("{f}: {i} is not an SRI sha384"));
            let raw = base64::engine::general_purpose::STANDARD.decode(b64).unwrap_or_else(|_| panic!("{f}: {i} is not base64"));
            assert_eq!(raw.len(), 48, "{f}: a sha384 is 48 bytes");
        }
        // the modules the entry loads are all pinned (the engine, its glue and the inlined wasm)
        for f in ["js/wasm.js", "js/engine.js", "js/embed-model.js", "wasm/beatscript.js", "wasm/beatscript_bg.wasm.b64.js"] {
            assert!(!l.integrity(f).is_empty(), "the lock has no integrity for {f}");
        }
    }

    #[test]
    fn the_page_loads_the_pinned_player_and_only_it() {
        let (t, dir) = tools();
        let l = PlayerCdn::get();
        let (sum, page) = call(&t, json!({"text": PROG})).unwrap();
        // the entry: a module script at the exact version, with the lock's integrity, crossorigin=anonymous
        let want = format!("<script type=\"module\" src=\"https://cdn.jsdelivr.net/npm/@stackdump/beatscript-player@{}/{}\" integrity=\"{}\" crossorigin=\"anonymous\"></script>", l.version, l.entry, l.integrity(&l.entry));
        let i = page.find(&want).unwrap_or_else(|| panic!("no pinned player script; want {want}"));
        // the import map: before the module script, an integrity for every other module, each the lock's
        let m0 = page.find("<script type=\"importmap\">").expect("an import map");
        assert!(m0 < i, "the import map must come before the module script");
        let body = &page[m0 + "<script type=\"importmap\">".len()..];
        let map: Value = serde_json::from_str(&body[..body.find("</script>").unwrap()]).expect("the import map is JSON");
        let integ = map["integrity"].as_object().expect("integrity");
        let mods: Vec<&(String, String)> = l.modules().collect();
        assert!(!mods.is_empty());
        assert_eq!(integ.len(), mods.len(), "one integrity per module");
        for (f, sri) in mods {
            assert_eq!(integ.get(&l.url(f)).and_then(Value::as_str), Some(sri.as_str()), "{f}");
        }
        assert_eq!(page.matches("<script").count(), 2, "the page has the import map and the player script, nothing else");
        assert_eq!(page.matches(" src=").count(), 1, "only the player is loaded");
        for bad in ["@latest", "@^", "@~", "autoplay", "beatscript.app/js/", "beatscript.app/wasm/", "<iframe"] {
            assert!(!page.contains(bad), "page contains {bad:?}");
        }
        // the program and its CID; sound only from the element's Play button (it has no attribute that plays)
        let cid = sum["cid"].as_str().unwrap();
        assert_eq!(attr(&page, "beatscript-track", "cid").as_deref(), Some(cid));
        let text = attr(&page, "beatscript-track", "text").expect("the program is in the page");
        let rep = t.eng.check("127.0.0.1", &text).unwrap();
        assert_eq!(Rep(&rep).cid(), cid, "the embedded text is the program the CID names");
        assert!(text.starts_with("#! name: Four on the floor\n"), "{text}");
        // CID, rights and the link back: not stored, so the self-sealing link and never /p/<CID> (it would not answer)
        assert!(page.contains(&format!("<code>{cid}</code>")));
        let link = sum["link"].as_str().unwrap();
        assert!(link.starts_with(&format!("{}/#{cid}.", t.public)), "{link}");
        assert!(page.contains(&format!("href=\"{}\"", esc(link))), "the page links the self-sealing link");
        assert!(!page.contains(&format!("/p/{cid}")), "an unstored program's page links /p/<CID>");
        assert!(sum["short_url"].is_null() && sum["store_hint"].is_string());
        assert!(page.contains("spdx.org/licenses/CC0-1.0.html"), "the rights");
        assert_eq!(sum["rights"]["license"], "CC0-1.0");
        assert_eq!(sum["title"], "Four on the floor");
        assert_eq!(sum["player"]["script"].as_str().unwrap(), l.url(&l.entry));
        assert_eq!(sum["player"]["integrity"].as_str().unwrap(), l.integrity(&l.entry));
        assert_eq!(sum["html_bytes"].as_u64().unwrap() as usize, page.len());
        assert_eq!(sum["stored"], false);
        assert!(sum["publish"].as_str().unwrap().contains("exactly as given"));
        // the same text, the same page
        assert_eq!(call(&t, json!({"text": PROG})).unwrap().1, page);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// With the published package at hand (BEATSCRIPT_PLAYER_DIR=<the unpacked npm tarball's package/>), every file's
    /// bytes are the ones its lock integrity names.
    #[test]
    fn the_lock_is_the_published_package() {
        let Some(d) = beatscript_serve::env_var("PLAYER_DIR") else {
            eprintln!("BEATSCRIPT_PLAYER_DIR not set: the package's bytes are not checked");
            return;
        };
        let d = std::path::PathBuf::from(d);
        let pkg: Value = serde_json::from_slice(&std::fs::read(d.join("package.json")).unwrap()).unwrap();
        let l = PlayerCdn::get();
        assert_eq!((pkg["name"].as_str().unwrap(), pkg["version"].as_str().unwrap()), (l.package.as_str(), l.version.as_str()));
        for (f, sri) in &l.files {
            use sha2::Digest;
            let b = std::fs::read(d.join(f)).unwrap_or_else(|e| panic!("{f}: {e}"));
            let got = format!("sha384-{}", base64::engine::general_purpose::STANDARD.encode(sha2::Sha384::digest(&b)));
            assert_eq!(&got, sri, "{f}");
        }
    }

    /// The radio's lock, as the track's: one exact version, SRI sha384 for every file, the radio's own modules pinned. With
    /// the package at hand (BEATSCRIPT_RADIO_PLAYER_DIR=<the unpacked tarball's package/>), every file's bytes are the
    /// ones its integrity names.
    #[test]
    fn the_radio_lock_pins_one_exact_version_and_is_the_package() {
        let l = PlayerCdn::radio();
        assert_eq!(l.package, "@stackdump/beatscript-player");
        let parts: Vec<&str> = l.version.split('.').collect();
        assert!(parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())), "version {:?}", l.version);
        for f in [RADIO_ENTRY, "beatscript-track.js", "js/station-model.js", "js/handoff-model.js", "js/driver.js", "js/engine.js", "wasm/beatscript.js", "wasm/beatscript_bg.wasm.b64.js"] {
            assert!(l.integrity(f).starts_with("sha384-"), "radio-cdn.lock.json has no integrity for {f}");
        }
        let Some(d) = beatscript_serve::env_var("RADIO_PLAYER_DIR") else {
            eprintln!("BEATSCRIPT_RADIO_PLAYER_DIR not set: the package's bytes are not checked");
            return;
        };
        let d = std::path::PathBuf::from(d);
        let pkg: Value = serde_json::from_slice(&std::fs::read(d.join("package.json")).unwrap()).unwrap();
        assert_eq!((pkg["name"].as_str().unwrap(), pkg["version"].as_str().unwrap()), (l.package.as_str(), l.version.as_str()));
        for (f, sri) in &l.files {
            use sha2::Digest;
            let b = std::fs::read(d.join(f)).unwrap_or_else(|e| panic!("{f}: {e}"));
            assert_eq!(&format!("sha384-{}", base64::engine::general_purpose::STANDARD.encode(sha2::Sha384::digest(&b))), sri, "{f}");
        }
    }

    #[test]
    fn refusals() {
        let (t, dir) = tools();
        let code = |a: Value| call(&t, a).err().map(|e| e.err.code.clone()).unwrap_or_else(|| "ok".into());
        assert_eq!(code(json!({})), "bad_request");
        assert_eq!(code(json!({"text": PROG, "example": "x"})), "bad_request");
        assert_eq!(code(json!({"cid": "nope"})), "bad_request");
        assert_eq!(code(json!({"example": "no-such-example"})), "not_found");
        assert_eq!(code(json!({"cid": "z4EBG9jAWvWyiVNF43gMjU5wYinuLmnAfWLhAVK41gqupdYGvVo"})), "not_found");
        assert_eq!(code(json!({"text": "tempo 120\ndeck A: kick x ~ q\n"})), "bad_request", "a program with errors");
        assert_eq!(code(json!({"text": "use restricted\ntempo 120\ndeck A: kick x ~ x ~\n"})), "restricted");
        assert_eq!(code(json!({"text": "#! visibility: private\ntempo 120\ndeck A: kick x ~ x ~\n"})), "restricted");
        assert_eq!(code(json!({"text": PROG, "extra": 1})), "bad_request");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn titles_and_text_are_escaped() {
        let (t, dir) = tools();
        let (sum, page) = call(&t, json!({"text": "#! name: a \"b\" </script><b>x</b>\n# it's & so\ntempo 120\ndeck A: kick x ~ x ~\n"})).unwrap();
        assert!(!page.contains("</script><b>"), "the title closed a tag");
        assert!(page.contains("<title>a &quot;b&quot; &lt;/script&gt;&lt;b&gt;x&lt;/b&gt;</title>"));
        let text = attr(&page, "beatscript-track", "text").unwrap();
        assert_eq!(Rep(&t.eng.check("127.0.0.1", &text).unwrap()).cid(), sum["cid"].as_str().unwrap());
        // no name: the page says so, and is called BeatScript
        let (sum, _) = call(&t, json!({"text": "tempo 120\ndeck A: kick x ~ x ~\n", "title": ""})).unwrap();
        assert_eq!(sum["title"], "BeatScript");
        assert!(sum["title_hint"].is_string());
        let (sum, page) = call(&t, json!({"text": "tempo 120\ndeck A: kick x ~ x ~\n", "title": "Kick test"})).unwrap();
        assert_eq!(sum["title"], "Kick test");
        assert!(page.contains("<h1>Kick test</h1>"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// audio inline: a stored program's Opus render in the page as a data: URI, and nothing loaded; engine on request.
    #[test]
    fn a_stored_program_carries_its_render_inline() {
        let (t, dir) = tools();
        let text = "#! name: Inline check\ntempo 120\nsection groove 2 bars {\n  deck A: kick x ~ x ~ | hihat x*8\n}\narrange groove\n";
        let (prog, _) = crate::compose::put(&t.app, &t.eng, "127.0.0.1", text, json!({"license": "CC-BY-4.0", "visibility": "public", "source": "put", "credits": ["a tester"]})).unwrap();
        let cid = prog["cid"].as_str().unwrap().to_string();
        let (sum, page) = call(&t, json!({"cid": cid})).unwrap();
        assert_eq!(sum["audio"], "inline", "{sum}");
        assert!(sum["player"].is_null());
        assert_eq!(sum["rights"]["license"], "CC-BY-4.0");
        assert!(page.contains("spdx.org/licenses/CC-BY-4.0.html") && page.contains("a tester"), "the stored rights");
        assert!(!page.contains("<script"), "an inline page loads nothing and runs nothing");
        assert!(!page.contains("jsdelivr.net/npm/@stackdump/beatscript-player@") || page.matches("cdn.jsdelivr").count() == 0);
        assert!(!page.contains("autoplay"));
        assert!(page.contains(&format!("<meta name=\"beatscript:cid\" content=\"{cid}\">")));
        assert!(page.contains(&format!("{}/p/{cid}", t.public)));
        let src = attr(&page, "audio", "src").expect("an <audio> with a src");
        let b64 = src.strip_prefix("data:audio/ogg;base64,").expect("a data: URI of Ogg");
        let ogg = base64::engine::general_purpose::STANDARD.decode(b64).unwrap();
        assert!(ogg.starts_with(b"OggS"), "the render is an Ogg stream");
        assert_eq!(sum["render"]["bytes"].as_u64().unwrap() as usize, ogg.len());
        assert!(b64.len() <= MAX_INLINE_B64);
        // the same bytes every time (the cached render)
        assert_eq!(call(&t, json!({"cid": cid})).unwrap().1, page);
        // the program text is in the page, and is the program
        let i = page.find("<pre>").unwrap() + 5;
        let pre = unesc(&page[i..i + page[i..].find("</pre>").unwrap()]);
        assert_eq!(Rep(&t.eng.check("127.0.0.1", &pre).unwrap()).cid(), cid);
        // engine on request: the live player, no audio
        let (sum, page) = call(&t, json!({"cid": cid, "audio": "engine"})).unwrap();
        assert_eq!(sum["audio"], "engine");
        assert!(page.contains("<beatscript-track ") && !page.contains("<audio"));
        // a text is not stored: inline falls back, and says so
        let (sum, page) = call(&t, json!({"text": PROG, "audio": "inline"})).unwrap();
        assert_eq!(sum["audio"], "engine");
        assert!(sum["audio_note"].as_str().unwrap().contains("stored"));
        assert!(page.contains("<beatscript-track "));
        assert_eq!(call(&t, json!({"text": PROG, "audio": "mp3"})).err().map(|e| e.err.code.clone()).as_deref(), Some("bad_request"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// store true: a signed-in caller stores the text first (as share does), so the page links /p/<CID> and carries the
    /// render; without a sign-in it is refused, and nothing is stored.
    #[test]
    fn store_stores_the_text_first() {
        let (t, dir) = tools();
        let text = "#! name: Store check\n#! license: CC-BY-4.0\ntempo 120\nsection groove 2 bars {\n  deck A: kick x ~ x ~ | snare ~ x ~ x\n}\narrange groove\n";
        let e = call(&t, json!({"text": text, "store": true})).err().expect("store needs a sign-in");
        assert_eq!(e.err.code, "auth_required");
        let (sum, _) = call(&t, json!({"text": text})).unwrap();
        let cid = sum["cid"].as_str().unwrap().to_string();
        assert!(t.stored(&cid).is_none(), "nothing stored without store");
        assert_eq!(sum["rights"]["license"], "CC-BY-4.0", "the text's licence");
        let (sum, page) = call_as(&t, Some("u-store"), json!({"text": text, "store": true})).unwrap();
        assert_eq!(sum["cid"].as_str(), Some(cid.as_str()));
        assert_eq!(sum["stored"], true);
        assert!(t.stored(&cid).is_some(), "stored");
        assert_eq!(sum["rights"]["license"], "CC-BY-4.0");
        assert_eq!(sum["link"].as_str().unwrap(), format!("{}/p/{cid}", t.public));
        assert_eq!(sum["short_url"], sum["link"]);
        assert_eq!(sum["audio"], "inline", "{sum}");
        assert!(page.contains(&format!("href=\"{}/p/{cid}\"", t.public)) && page.contains("data:audio/ogg;base64,"));
        std::fs::remove_dir_all(&dir).ok();
    }

    const STATION: &str = "#! name: short techno radio\n#! license: CC0-1.0\ntempo 140\nkey am\nstation {\n  from gen@2 techno seed=7 length=short\n  next seed=+1\n}\n";

    /// station: a radio page. `<beatscript-radio>` from the pinned package (its own entry, every other module in the import
    /// map with the lock's integrity), the station's text and CID in its attributes, nothing else loaded, nothing that plays
    /// by itself. BEATSCRIPT_STATION_PAGE_OUT=<file> writes the page (site/test/player-radio.mjs plays it in a browser).
    #[test]
    fn a_station_makes_a_radio_page() {
        let (t, dir) = tools();
        let l = PlayerCdn::radio();
        assert_eq!(l.entry, RADIO_ENTRY);
        assert!(!l.integrity(RADIO_ENTRY).is_empty(), "radio-cdn.lock.json has no {RADIO_ENTRY}: the pinned package predates <beatscript-radio>");
        let (sum, page) = call(&t, json!({"station": STATION})).unwrap();
        let cid = sum["station"]["cid"].as_str().unwrap().to_string();
        assert!(is_cid(&cid), "{sum}");
        assert_eq!(sum["title"], "short techno radio");
        assert_eq!(sum["station"]["genre"], "techno");
        assert_eq!(sum["station"]["tempo"], 140);
        assert_eq!(sum["n"], 0);
        assert_eq!(sum["player"]["element"], "beatscript-radio");
        assert_eq!(sum["player"]["script"].as_str().unwrap(), l.url(RADIO_ENTRY));
        assert_eq!(sum["player"]["integrity"].as_str().unwrap(), l.integrity(RADIO_ENTRY));
        assert_eq!(sum["html_bytes"].as_u64().unwrap() as usize, page.len());
        // the entry, pinned; the import map before it, with every other module (the track element's loader among them)
        let want = format!("<script type=\"module\" src=\"{}\" integrity=\"{}\" crossorigin=\"anonymous\"></script>", l.url(RADIO_ENTRY), l.integrity(RADIO_ENTRY));
        let i = page.find(&want).unwrap_or_else(|| panic!("no pinned radio script; want {want}"));
        let m0 = page.find("<script type=\"importmap\">").expect("an import map");
        assert!(m0 < i);
        let body = &page[m0 + "<script type=\"importmap\">".len()..];
        let map: Value = serde_json::from_str(&body[..body.find("</script>").unwrap()]).unwrap();
        let integ = map["integrity"].as_object().unwrap();
        let mods: Vec<&(String, String)> = l.modules_of(RADIO_ENTRY).collect();
        assert_eq!(integ.len(), mods.len());
        for (f, sri) in mods {
            assert_eq!(integ.get(&l.url(f)).and_then(Value::as_str), Some(sri.as_str()), "{f}");
        }
        assert!(integ.contains_key(&l.url("beatscript-track.js")), "the radio imports the track element's module loader");
        assert!(!integ.contains_key(&l.url(RADIO_ENTRY)), "the entry is not in its own import map");
        assert_eq!(page.matches("<script").count(), 2, "the import map and the radio, nothing else");
        assert_eq!(page.matches(" src=").count(), 1);
        for bad in ["@latest", "@^", "@~", "autoplay", "beatscript.app/js/", "beatscript.app/wasm/", "<iframe", "localStorage", "cookie"] {
            assert!(!page.contains(bad), "page contains {bad:?}");
        }
        // the station: its text (the program the CID names) and the CID the element holds it to
        assert_eq!(attr(&page, "beatscript-radio", "cid").as_deref(), Some(cid.as_str()));
        let text = attr(&page, "beatscript-radio", "station").expect("the station is in the page");
        assert_eq!(text, STATION);
        let again = t.eng.station("127.0.0.1", &json!({"station": text, "count": 1}).to_string()).unwrap();
        assert_eq!(again["station"]["cid"].as_str(), Some(cid.as_str()));
        assert!(attr(&page, "beatscript-radio", "n").is_none(), "position 0 says nothing");
        assert!(page.contains(&format!("<meta name=\"beatscript:station\" content=\"{cid}\">")));
        let link = sum["link"].as_str().unwrap();
        assert!(link.starts_with(&format!("{}/radio?s=", t.public)) && link.ends_with("&n=0"), "{link}");
        assert!(page.contains(&format!("href=\"{}\"", esc(link))));
        assert!(page.contains("spdx.org/licenses/CC0-1.0.html"));
        // the same station, the same page
        assert_eq!(call(&t, json!({"station": STATION})).unwrap().1, page);
        if let Some(out) = beatscript_serve::env_var("STATION_PAGE_OUT") {
            std::fs::write(&out, &page).unwrap();
            eprintln!("wrote {out}");
        }
        // a position and a title
        let (sum, page) = call(&t, json!({"station": STATION, "n": 7, "title": "Seven"})).unwrap();
        assert_eq!(sum["n"], 7);
        assert_eq!(attr(&page, "beatscript-radio", "n").as_deref(), Some("7"));
        assert!(page.contains("<h1>Seven</h1>") && sum["link"].as_str().unwrap().ends_with("&n=7"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The techno radio (examples/beatscript/techno-radio.beatscript, docs/beatscript/radio.md's example: its CID is the
    /// doc's), its text escaped into the attribute so that nothing in it closes a tag. BEATSCRIPT_STATION_DEMO_OUT=<file>
    /// writes its page.
    #[test]
    fn the_techno_radio_and_escaping() {
        let (t, dir) = tools();
        let techno = include_str!("../../../../examples/beatscript/techno-radio.beatscript");
        let (sum, page) = call(&t, json!({"station": techno})).unwrap();
        assert_eq!(sum["station"]["source"], "text");
        assert_eq!(sum["station"]["cid"], "z4EBG9j79FsQaBchMGcKHL55KLeH6FjsjLQ7V1AE9oj2PencNUE", "docs/beatscript/radio.md's techno radio");
        assert_eq!(sum["title"], "techno radio");
        assert_eq!(attr(&page, "beatscript-radio", "station").as_deref(), Some(techno));
        if let Some(out) = beatscript_serve::env_var("STATION_DEMO_OUT") {
            std::fs::write(&out, &page).unwrap();
            eprintln!("wrote {out}");
        }
        let hostile = "#! name: a \"b\" </script><b>x</b>\ntempo 120\nstation {\n  from gen@2 house seed=3\n}\n";
        let (_, page) = call(&t, json!({"station": hostile})).unwrap();
        assert!(!page.contains("</script><b>"), "the station's text closed a tag");
        assert_eq!(attr(&page, "beatscript-radio", "station").as_deref(), Some(hostile));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn station_refusals() {
        let (t, dir) = tools();
        let code = |a: Value| call(&t, a).err().map(|e| e.err.code.clone()).unwrap_or_else(|| "ok".into());
        assert_eq!(code(json!({"station": STATION, "text": PROG})), "bad_request", "station and text");
        assert_eq!(code(json!({"station": STATION, "audio": "inline"})), "bad_request", "station and audio");
        assert_eq!(code(json!({"station": STATION, "store": true})), "bad_request", "station and store");
        assert_eq!(code(json!({"text": PROG, "n": 3})), "bad_request", "n without station");
        assert_eq!(code(json!({"station": STATION, "n": -1})), "bad_request");
        assert_eq!(code(json!({"station": "tempo 120\ndeck A: kick x ~ x ~\n"})), "bad_request", "a program that is not a station");
        assert_eq!(code(json!({"station": "z4EBG9jAWvWyiVNF43gMjU5wYinuLmnAfWLhAVK41gqupdYGvVo"})), "not_found");
        assert_eq!(code(json!({"station": "no-such-station"})), "not_found");
        assert_eq!(code(json!({"station": "#! visibility: private\nstation {\n  from gen@2 house seed=3\n}\n"})), "restricted");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Every example the site ships makes a page whose embedded text is the example's program, with the example's rights.
    #[test]
    fn examples_make_pages() {
        let (t, dir) = tools();
        let names: Vec<String> = t.app.examples.list.iter().map(|e| e.name.clone()).collect();
        let mut made = 0;
        for n in &names {
            let (sum, page) = match call(&t, json!({"example": n, "audio": "engine"})) {
                Ok(x) => x,
                Err(e) if e.err.code == "restricted" => continue, // samples or loops: nothing to play in a page
                Err(e) => panic!("{n}: {}", e.err.message),
            };
            let cid = t.example_cid(n).unwrap();
            assert_eq!(sum["cid"].as_str(), Some(cid.as_str()), "{n}");
            assert_eq!(sum["stored"], true);
            assert_eq!(sum["rights"]["source"], "example");
            let text = attr(&page, "beatscript-track", "text").unwrap();
            assert_eq!(Rep(&t.eng.check("127.0.0.1", &text).unwrap()).cid(), cid, "{n}: the embedded text");
            made += 1;
        }
        eprintln!("{made} of {} examples make pages", names.len());
        std::fs::remove_dir_all(&dir).ok();
    }
}
