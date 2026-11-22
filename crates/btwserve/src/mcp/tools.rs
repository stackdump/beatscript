//! Go `mcpsrv/tools.go`, `queue.go`, the landing page: the fourteen tools, their schemas (tools/list is pinned byte
//! for byte by `btwsite/testdata/mcp/tools-list.json`; `listen` and get's `mixer` are this server's own, past Go's), the argument rules, the two families of limits (checker calls
//! a minute, programs stored an hour, each per caller and four times that per address), and the answers, as Go's
//! encoding/json writes them (a Go map's keys sorted, a struct's in field order).
//!
//! The checker tools (check, fmt, share, open, render, record) call `compose::engine::Engine`, btwapi in-process with
//! Go's output; the hub tools (eval, load, get, stop, devices, queue) call `hubapi::HubApi`.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{header, Response};
use serde_json::{json, Map, Value};

use super::hubapi::{EmptyHub, HubApi, QueueItem, QueueOp, SessionEvent};
use super::limiter::Limiter;
use super::text::{self, fill};
use crate::compose::engine::{Engine, Rep};
use crate::compose::gojson::{self, sorted};

/// The landing page's styles, served at `/mcp.css` (the site's CSP is `style-src 'self'`: no inline `<style>`).
pub const LANDING_CSS: &str = include_str!("landing.css");
use crate::compose::{self as compose, Fail};
use crate::AppState;

// listen: a child of this module (listen.rs), so it shares the argument rules and the checker
#[path = "listen.rs"]
mod listen;

pub const TOOL_NAMES: [&str; 20] = ["check", "fmt", "expand", "generate", "eval", "load", "get", "share", "open", "render", "record", "list_examples", "instruments", "stop", "devices", "queue", "listen", "lineage", "genre_template", "push_private"];

/// The tools that need a signed-in caller.
/// The account a `/mcp` bearer token acts as: an OAuth access token bound to `/mcp` (oauth.rs), or a pasted `btw_`
/// token from the MCP panel.
pub fn token_user(app: &crate::AppState, tok: &str) -> Option<String> {
    if tok.starts_with(crate::oauth::ACCESS_PREFIX) {
        return crate::oauth::get().and_then(|o| o.access_user(tok));
    }
    app.authsvc.get().and_then(|a| a.verify.user_from_token(tok))
}

pub fn auth_tool(n: &str) -> bool {
    matches!(n, "eval" | "load" | "get" | "stop" | "devices" | "queue" | "beta_signups")
}

/// What the HTTP request gives a call.
pub struct CallInfo {
    pub token: String,
    pub ip: String,
}

/// Who is calling.
struct Caller {
    uid: Option<String>,
    ip: String,
    key: String,
}

const IP_CEILING: usize = 4;
/// How many harvested words list_examples(lexicon) returns.
const LEXICON_TOP: usize = 50;
const MAX_CREDITS: usize = 8;
const MAX_CREDIT_CHARS: usize = 80;

pub struct Tools {
    app: AppState,
    public: String,
    max_prog: usize,
    check_per_min: usize,
    render_per_hour: usize,
    queue: (usize, usize, usize, usize, usize),
    cpu: Limiter,
    cpu_ip: Limiter,
    render: Limiter,
    render_ip: Limiter,
    share: Limiter,
    share_ip: Limiter,
    queue_lim: Limiter,
    queue_ip: Limiter,
    inst: Mutex<Option<(Instant, String)>>,
    eng: Engine,
    hub: Arc<dyn HubApi>,
    list: Vec<Value>,
    listen: listen::Listen,
    /// The lexicon harvest (docs/btw/lexicon-harvest.md): what the checking tools record as a side effect.
    harvest: Arc<btwserve::diagnostics::Harvest>,
}

/// The tools whose answers carry `lexicon` ({harvested, new}): every one that checks a program, push_private excepted
/// (it records nothing). Their descriptions end with [`text::LEXICON_DESC`].
pub const HARVEST_TOOLS: [&str; 9] = ["check", "eval", "load", "listen", "render", "share", "queue", "genre_template", "generate"];

type R = Result<Value, Fail>;

fn bad(msg: impl Into<String>) -> Fail {
    Fail::new(400, "bad_request", msg)
}

fn q(s: &str) -> String {
    super::hubapi::go_quote(s)
}

/// `^z[1-9A-HJ-NP-Za-km-z]{20,100}$`
pub fn is_cid(s: &str) -> bool {
    let b = s.as_bytes();
    b.first() == Some(&b'z') && (21..=101).contains(&b.len()) && b[1..].iter().all(|c| matches!(c, b'1'..=b'9' | b'A'..=b'H' | b'J'..=b'N' | b'P'..=b'Z' | b'a'..=b'k' | b'm'..=b'z'))
}

fn id_ok(s: &str) -> bool {
    (6..=64).contains(&s.len()) && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

/// `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`
fn name_ok(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty() && b.len() <= 64 && b[0].is_ascii_alphanumeric() && b.iter().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
}

/// `^[A-Za-z0-9][A-Za-z0-9.+-]{0,39}$`
fn license_ok(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty() && b.len() <= 40 && b[0].is_ascii_alphanumeric() && b.iter().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'+' | b'-'))
}

/// `bareCID`: a bare CID, `#<CID>`, or a CID-only short link (`<origin>/p/<CID>`, `/c/<CID>`, `/#<CID>`).
pub fn bare_cid(u: &str) -> Option<String> {
    let u = u.trim();
    if is_cid(u) {
        return Some(u.into());
    }
    if let Some(c) = u.strip_prefix('#').filter(|c| is_cid(c)) {
        return Some(c.into());
    }
    let rest = u.strip_prefix("https://").or_else(|| u.strip_prefix("http://"))?;
    let slash = rest.find('/')?;
    let host = &rest[..slash];
    if host.is_empty() || host.bytes().any(|c| matches!(c, b'?' | b'#') || c.is_ascii_whitespace()) {
        return None;
    }
    let path = &rest[slash + 1..];
    let tail_ok = |t: &str| t.is_empty() || (matches!(t.as_bytes()[0], b'?' | b'#') && !t.bytes().any(|c| c.is_ascii_whitespace()));
    let take = |s: &str| -> Option<(String, usize)> {
        // z and 20 to 100 base58 characters, then the end, a slash, ? or #
        let m = s.strip_prefix('z')?.bytes().take_while(|c| matches!(c, b'1'..=b'9' | b'A'..=b'H' | b'J'..=b'N' | b'P'..=b'Z' | b'a'..=b'k' | b'm'..=b'z')).count();
        (20..=100).contains(&m).then(|| (s[..1 + m].to_string(), 1 + m))
    };
    if let Some(p) = path.strip_prefix("p/").or_else(|| path.strip_prefix("c/")) {
        let (cid, n) = take(p)?;
        let mut t = &p[n..];
        if let Some(x) = t.strip_prefix('/') {
            t = x;
        }
        return tail_ok(t).then_some(cid);
    }
    if let Some(p) = path.strip_prefix('#') {
        let (cid, n) = take(p)?;
        return tail_ok(&p[n..]).then_some(cid);
    }
    None
}

/// `base64.StdEncoding.DecodeString`, with Go's error offsets (`CorruptInputError`).
fn go_base64(src: &[u8]) -> Result<Vec<u8>, usize> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::new();
    let mut si = 0;
    loop {
        let mut dbuf = [0u8; 4];
        let mut dlen = 4;
        let mut j = 0;
        let mut err = None;
        while j < 4 {
            if si == src.len() {
                if j == 0 {
                    return Ok(out);
                }
                return Err(si - j);
            }
            let c = src[si];
            si += 1;
            if let Some(v) = val(c) {
                dbuf[j] = v;
                j += 1;
                continue;
            }
            if c == b'\n' || c == b'\r' {
                continue;
            }
            if c != b'=' {
                return Err(si - 1);
            }
            match j {
                0 | 1 => return Err(si - 1),
                2 => {
                    while si < src.len() && (src[si] == b'\n' || src[si] == b'\r') {
                        si += 1;
                    }
                    if si == src.len() {
                        return Err(src.len());
                    }
                    if src[si] != b'=' {
                        return Err(si - 1);
                    }
                    si += 1;
                }
                _ => {}
            }
            while si < src.len() && (src[si] == b'\n' || src[si] == b'\r') {
                si += 1;
            }
            if si < src.len() {
                err = Some(si);
            }
            dlen = j;
            break;
        }
        let v = (u32::from(dbuf[0]) << 18) | (u32::from(dbuf[1]) << 12) | (u32::from(dbuf[2]) << 6) | u32::from(dbuf[3]);
        let b = [(v >> 16) as u8, (v >> 8) as u8, v as u8];
        out.extend_from_slice(&b[..dlen.saturating_sub(1)]);
        if let Some(e) = err {
            return Err(e);
        }
        if dlen < 4 {
            return Ok(out);
        }
    }
}

/// `oneLine`.
fn one_line(v: &str, max: usize) -> Option<String> {
    let v = v.trim();
    if v.chars().count() > max || v.chars().any(|c| (c as u32) < 0x20 || c == '\u{7f}') {
        return None;
    }
    Some(v.to_string())
}

/// Go's `%v` of a float64.
fn gv(f: f64) -> String {
    gojson::float(f)
}

// ---------- arguments ----------

struct Args(Map<String, Value>);

impl Args {
    fn only(&self, keys: &[&str]) -> Result<(), Fail> {
        for k in self.0.keys() {
            if !keys.contains(&k.as_str()) {
                let mut ks = keys.to_vec();
                ks.sort();
                return Err(bad(format!("unknown argument {}; this tool takes: {}", q(k), ks.join(", "))));
            }
        }
        Ok(())
    }

    fn present(&self, k: &str) -> Option<&Value> {
        self.0.get(k).filter(|v| !v.is_null())
    }

    fn str(&self, k: &str, max: usize) -> Result<Option<String>, Fail> {
        let Some(raw) = self.present(k) else { return Ok(None) };
        let Some(v) = raw.as_str() else { return Err(bad(format!("argument {} must be a string", q(k)))) };
        if v.len() > max {
            return Err(Fail::new(413, "too_large", format!("argument {} is {} bytes; the limit is {max}", q(k), v.len())));
        }
        if v.contains('\0') {
            return Err(bad(format!("argument {} contains a NUL byte", q(k))));
        }
        Ok(Some(v.to_string()))
    }

    fn num(&self, k: &str, lo: f64, hi: f64) -> Result<Option<f64>, Fail> {
        let Some(raw) = self.present(k) else { return Ok(None) };
        let Some(f) = raw.as_f64() else { return Err(bad(format!("argument {} must be a number", q(k)))) };
        if f < lo || f > hi {
            return Err(bad(format!("argument {} must be between {} and {}", q(k), gv(lo), gv(hi))));
        }
        Ok(Some(f))
    }

    fn boolean(&self, k: &str) -> Result<bool, Fail> {
        match self.present(k) {
            None => Ok(false),
            Some(Value::Bool(b)) => Ok(*b),
            Some(_) => Err(bad(format!("argument {} must be true or false", q(k)))),
        }
    }
}

// ---------- schemas ----------

fn sprop(desc: &str, extra: &[(&str, Value)]) -> Value {
    let mut m = Map::new();
    m.insert("type".into(), json!("string"));
    for (k, v) in extra {
        m.insert((*k).into(), v.clone());
    }
    m.insert("description".into(), json!(desc));
    Value::Object(m)
}

/// The `genre` argument of check.
const GENRE_ARG: &str = "Optional: a genre of the bitwrap ontology (techno, house, jazz, ... or its https://ontology.bitwrap.io/genre/ IRI); else the program's own `#! genre:` line. The answer gains style {genre, label, warnings[{severity: \"style\", code, message, line?, col?}]}: advice from the genre's conventions in the ontology (a tempo outside its range, a kit or voice the genre and its family do not use, chords far from its progressions). Style warnings never make ok false.";

const CID_PAT: &str = "^z[1-9A-HJ-NP-Za-km-z]{20,100}$";
const NAME_PAT: &str = "^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$";
const ID_PAT: &str = "^[A-Za-z0-9_-]{6,64}$";

fn tool(name: &str, title: &str, desc: String, hints: [bool; 4], props: Vec<(&str, Value)>, required: &[&str]) -> Value {
    let mut p = Map::new();
    for (k, v) in props {
        p.insert(k.into(), v);
    }
    let schema = json!({"type": "object", "properties": Value::Object(p), "required": required, "additionalProperties": false});
    let ann = gojson::obj([
        ("title", json!(title)),
        ("readOnlyHint", json!(hints[0])),
        ("destructiveHint", json!(hints[1])),
        ("idempotentHint", json!(hints[2])),
        ("openWorldHint", json!(hints[3])),
    ]);
    gojson::obj([("annotations", ann), ("description", json!(desc)), ("inputSchema", gojson::sorted_deep(schema)), ("name", json!(name))])
}

impl Tools {
    pub fn new(app: &AppState) -> Tools {
        let c = &app.cfg;
        let public = c.public_url.trim_end_matches('/').to_string();
        let (check_per_min, render_per_hour, share_per_hour) = (60usize, 30usize, 60usize);
        let qn = |k: &str, d: i64| {
            let v = c.flag_num(k, d);
            if v <= 0 {
                d as usize
            } else {
                v as usize
            }
        };
        let queue = (qn("queue-max-items", 50), qn("queue-max-per-add", 20), qn("queue-idle-minutes", 30), qn("queue-max-auto", 8), qn("queue-writes-min", 30));
        let (min, hour) = (Duration::from_secs(60), Duration::from_secs(3600));
        let mut t = Tools {
            app: app.clone(),
            public,
            max_prog: c.max_program_bytes,
            check_per_min,
            render_per_hour,
            queue,
            cpu: Limiter::new(check_per_min, min),
            cpu_ip: Limiter::new(IP_CEILING * check_per_min, min),
            render: Limiter::new(render_per_hour, hour),
            render_ip: Limiter::new(IP_CEILING * render_per_hour, hour),
            share: Limiter::new(share_per_hour, hour),
            share_ip: Limiter::new(IP_CEILING * share_per_hour, hour),
            queue_lim: Limiter::new(queue.4, min),
            queue_ip: Limiter::new(IP_CEILING * queue.4, min),
            inst: Mutex::new(None),
            eng: Engine::over(app.checker.get().cloned()),
            // P6's hub (hub::mcp_api implements HubApi); EmptyHub only if the hub did not start
            hub: app.hub.get().map(|h| h.clone() as Arc<dyn HubApi>).unwrap_or_else(|| Arc::new(EmptyHub)),
            list: Vec::new(),
            listen: listen::Listen::new(render_per_hour),
            harvest: Arc::new(match btwserve::diagnostics::Harvest::open(&c.data_dir().join("diagnostics")) {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("btwserve: lexicon harvest: {e}; keeping it in memory");
                    btwserve::diagnostics::Harvest::memory()
                }
            }),
        };
        t.list = t.build_list();
        t
    }

    /// Replace the hub (P6 calls this with its `hub::Hub`, or main does).
    pub fn set_hub(&mut self, h: Arc<dyn HubApi>) {
        self.hub = h;
    }

    pub fn instructions(&self) -> String {
        fill(text::INSTRUCTIONS, &[&self.public])
    }

    fn build_list(&self) -> Vec<Value> {
        let mut v = tool_list(&self.public, self.max_prog, self.check_per_min, self.render_per_hour, self.queue);
        if self.beta_tool() {
            v.push(tool(
                "beta_signups",
                "Android beta requests",
                "The people who asked to join the BeatScript Android beta (a Google account email, an optional name, a status of pending, added or removed). action list (default) returns them, with status to filter; added marks one email as added to the Google Group (the page then shows them the join link); remove takes one off; purge erases the removed ones for good. For the site's owner only.".into(),
                [false, true, true, false],
                vec![
                    ("action", sprop("list, added, remove or purge.", &[("enum", json!(["list", "added", "remove", "purge"]))])),
                    ("email", sprop("The email address, for added and remove.", &[("maxLength", json!(254))])),
                    ("status", sprop("list: only this status.", &[("enum", json!(["pending", "added", "removed"]))])),
                ],
                &[],
            ));
            v.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        }
        v
    }
}

/// What tools/list answers, sorted by name (mcp-go's order).
pub fn tool_list(public: &str, max_prog: usize, check_per_min: usize, render_per_hour: usize, queue: (usize, usize, usize, usize, usize)) -> Vec<Value> {
    {
        let p = public;
        let mp = max_prog.to_string();
        let mpv = json!(max_prog);
        let session = sprop(
            "Session id of one browser tab, from the MCP panel of the page. Leave out to use your own signed-in tabs (every live one for eval/load/stop, the latest for get).",
            &[("maxLength", json!(64)), ("pattern", json!(ID_PAT))],
        );
        let device = sprop("Id of one of your paired phones (djrust on Android), from the devices tool. Targets that phone alone; do not combine with session.", &[("maxLength", json!(64)), ("pattern", json!(ID_PAT))]);
        let play = json!({"type": "boolean", "description": "Start playing too (default false: load it and leave it stopped, or leave a playing tab as it is)."});
        let cid_stored = sprop("CID of a program already stored (for instance the cid render or a signed-in share returned).", &[("maxLength", json!(100)), ("pattern", json!(CID_PAT))]);
        let example = sprop("Name of an example, from list_examples.", &[("maxLength", json!(64)), ("pattern", json!(NAME_PAT))]);
        let stored_text = sprop("The program text. It must pass check; it is stored publicly (CC0-1.0).", &[("maxLength", mpv.clone())]);
        let (qi, qa, qid, qauto, qw) = queue;
        let queue_items = json!({
            "type": "array", "maxItems": qa,
            "description": "add: the programs to append, each an object with exactly one of cid, example, url, and optionally bars, minutes, title.",
            "items": {
                "type": "object", "additionalProperties": false,
                "properties": {
                    "cid": {"type": "string", "maxLength": 100, "pattern": CID_PAT, "description": "CID of a stored program or an example."},
                    "example": {"type": "string", "maxLength": 64, "pattern": NAME_PAT, "description": "Name of an example, from list_examples."},
                    "url": {"type": "string", "maxLength": 32768, "description": "A share link, verified as load verifies it."},
                    "bars": {"type": "integer", "minimum": 1, "maximum": 1024, "description": "Play this many bars, then advance (else until the program ends; a loop: one arrangement pass)."},
                    "minutes": {"type": "number", "minimum": 0.1, "maximum": 120, "description": "Play this long, then advance at the next bar line. Not with bars."},
                    "title": {"type": "string", "maxLength": 80, "description": "What the page shows for it (else the program's name)."},
                },
            },
        });
        let mut v = vec![
            tool(
                "check",
                "Check a BeatScript program",
                format!("{}\n\n{}", fill(text::CHECK_DESC, &[p, &mp, &check_per_min.to_string()]), text::references()),
                [true, false, true, false],
                vec![
                    ("text", sprop(&format!("The program text (at most {mp} bytes). Comments and blank lines are fine."), &[("maxLength", mpv.clone())])),
                    ("genre", sprop(GENRE_ARG, &[("maxLength", json!(200))])),
                    ("expand", json!({"type": "boolean", "description": text::EXPAND_ARG})),
                    ("same_as", sprop(&format!("Optional: another program to compare with, as its CID (stored here, or an example's), a short {p}/p/<CID> link, or its text. The answer gains same_as: whether the two are the same song by their compiled nets and song-level settings (tempo, key, swing, length, end, loop), not their text: a let/def/repeat refactor or a renamed section is the same song; a changed note is not, and diff says what changed in music (song settings, sections moved or inserted, a part transposed or rotated, notes at bar:step)."), &[("maxLength", mpv.clone())])),
                ],
                &["text"],
            ),
            tool("fmt", "Format a BeatScript program", text::FMT_DESC.into(), [true, false, true, false], vec![("text", sprop("The program text.", &[("maxLength", mpv.clone())]))], &["text"]),
            tool(
                "expand",
                "Write out a program's seeds and templates",
                text::EXPAND_DESC.into(),
                [true, false, true, false],
                vec![
                    ("text", sprop("The program text, with gen@N calls, seed(...) or template(...) references in it.", &[("maxLength", mpv.clone())])),
                    ("lineage", json!({"type": "boolean", "description": "Open the written-out text with `#! forked-from: <CID of the program as given>` (default false), so that once it is shared it says what it came from."})),
                    ("eject", sprop(text::EXPAND_EJECT, &[("maxLength", json!(16)), ("pattern", json!("^[0-9]+:[0-9]+$"))])),
                ],
                &["text"],
            ),
            tool(
                "generate",
                "Generate a track from a seed (no model)",
                text::GENERATE_DESC.into(),
                [false, false, true, false],
                vec![
                    ("genre", json!({"type": "string", "enum": btwapi::generate::genres(), "description": "The genre whose rhythms, chords, instruments and song structure it uses (default techno)."})),
                    ("bpm", json!({"type": "number", "minimum": 40, "maximum": 300, "description": "Tempo; default the genre's."})),
                    ("key", sprop("Key: am, c, f#m, d:dorian ...; default the genre's.", &[("maxLength", json!(16))])),
                    ("length", json!({"type": "string", "enum": ["short", "standard", "long"], "description": "Song length (default standard)."})),
                    ("seed", json!({"type": ["string", "integer"], "description": "A number or a word (default 1). Same inputs, same program text and CID."})),
                    ("generator", json!({"type": "integer", "enum": btwapi::generate::VERSIONS, "description": format!("The version of the seeded generator, as gen@N names it in a program: 1 (seeded@1, frozen) or 2 (seeded@2, the chords and arps as Roman numerals); default {}. A version always prints the same program for the same inputs.", btwapi::generate::GENERATOR_VERSION)})),
                    ("part", json!({"type": "string", "enum": djlive::parse::ROLES, "description": "One part (a role): the answer gains part (that role's entry of parts) and let, the let its gen call writes (let drop_bass_1 = ... let drop_bass = <drop_bass_1 ...>), to paste into a program of your own."})),
                    ("section", sprop("With part: the section the part is taken from (default the drop or chorus when it plays there, else the first section it plays in).", &[("maxLength", json!(32))])),
                    ("seeds", json!({
                        "description": format!("Instead of seed: many seeds, as a list ([1, 2, \"sunrise\"]) or as {{count, start}} (count seeds from start, default 1), at most {MAX_SEEDS}. The answer is programs, one compact summary a seed in order: seed, generator, cid, name, key, tempo, bars, gen, parts (role, section, sections, voice or kit, text, gen) and url and render_url; each program is stored as one generate stores it, and nothing else is. The same seeds give the same summaries."),
                        "anyOf": [
                            {"type": "array", "minItems": 1, "maxItems": MAX_SEEDS, "items": {"type": ["string", "integer"]}},
                            {"type": "object", "additionalProperties": false, "required": ["count"], "properties": {
                                "count": {"type": "integer", "minimum": 1, "maximum": MAX_SEEDS},
                                "start": {"type": "integer", "minimum": 0},
                            }},
                        ],
                    })),
                ],
                &[],
            ),
            tool(
                "eval",
                "Play a program in a browser tab",
                fill(text::EVAL_DESC, &[p, &mp]),
                [false, true, true, false],
                vec![("text", sprop("The program text. It must pass check.", &[("maxLength", mpv.clone())])), ("play", play.clone()), ("session", session.clone()), ("device", device.clone())],
                &["text"],
            ),
            tool(
                "load",
                "Load a stored program into a browser tab",
                fill(text::LOAD_DESC, &[p]),
                [false, true, true, false],
                vec![
                    ("cid", sprop("CID of a program the site has stored (render and a signed-in share store theirs) or of an example.", &[("maxLength", json!(100)), ("pattern", json!(CID_PAT))])),
                    ("example", sprop("Name of an example, from list_examples.", &[("maxLength", json!(64)), ("pattern", json!(NAME_PAT))])),
                    ("url", sprop(&format!("A share link: {p}/#<CID>.<packed>, a /c/ link, a beats.bitwrap.io/p# link, or a short /p/<CID> link (the program must be stored here)."), &[("maxLength", json!(32768))])),
                    ("play", play.clone()),
                    ("session", session.clone()),
                    ("device", device.clone()),
                ],
                &[],
            ),
            tool("get", "What a browser tab is playing", fill(text::GET_DESC, &[p]), [true, false, true, false], vec![("session", session.clone()), ("device", device.clone())], &[]),
            tool(
                "share",
                "Make share links",
                fill(text::SHARE_DESC, &[p]),
                [false, false, true, false],
                vec![
                    ("text", sprop("The program text. It must pass check.", &[("maxLength", mpv.clone())])),
                    ("license", sprop("Rights kept with the stored program: a Creative Commons SPDX id (default CC0-1.0; CC-BY-4.0, CC-BY-SA-4.0 and the like). The site stores no other licence: keep such a program local, or send it to your phone with push_private.", &[("maxLength", json!(40))])),
                    ("forked_from", sprop("CID of the program this one was forked from. Stored with the rights.", &[("maxLength", json!(100)), ("pattern", json!(CID_PAT))])),
                    ("credits", json!({"type": "array", "items": {"type": "string", "maxLength": MAX_CREDIT_CHARS}, "maxItems": MAX_CREDITS, "description": "Who is credited, one short line each. Stored with the rights."})),
                    ("name", sprop("A name for the program (else its `#! name:` line).", &[("maxLength", json!(80))])),
                ],
                &["text"],
            ),
            tool(
                "open",
                "Open a share link",
                fill(text::OPEN_DESC, &[p]),
                [true, false, true, false],
                vec![("url", sprop("A share link (self-sealing #<CID>.<packed>, or a short <site>/p/<CID> or /c/<CID> link), a #fragment, <CID>.<packed>, or a bare CID.", &[("maxLength", json!(32768))]))],
                &["url"],
            ),
            tool(
                "render",
                "Render a program to streaming audio",
                fill(text::RENDER_DESC, &[&render_per_hour.to_string()]),
                [false, false, true, false],
                vec![
                    ("text", stored_text.clone()),
                    ("cid", cid_stored.clone()),
                    ("example", example.clone()),
                    ("format", sprop("opus (Ogg Opus, smaller, for download or <audio>) or wav (16-bit stereo, plays chunk by chunk in a web page) to listen; for a DAW: midi (a Standard MIDI File of the notes, a track per part), stems (a zip: a 32-bit float WAV per part and per deck that add up to the master, the master, the .mid and manifest.json with the CID, tempo, key, license and credits) or stems-opus (the same zip with Ogg Opus stems).", &[("enum", json!(["opus", "wav", "midi", "stems", "stems-opus"])), ("default", json!("opus"))])),
                ],
                &[],
            ),
            tool("record", "Set up a demo recording", fill(text::RECORD_DESC, &[&render_per_hour.to_string()]), [false, false, true, false], vec![("text", stored_text), ("cid", cid_stored), ("example", example)], &[]),
            tool(
                "list_examples",
                "The example programs",
                fill(text::EXAMPLES_DESC, &[p]),
                [true, false, true, false],
                vec![
                    ("name", sprop("Optional: also return this example's full text.", &[("maxLength", json!(64)), ("pattern", json!(NAME_PAT))])),
                    ("phrasebook", json!({"type": "boolean", "description": "Also return the public-domain phrasebook: lines first published before 1929, each with its text, source, year, meter, stress pattern, syllables and phoneme escapes (default false)."})),
                    ("meter", sprop("Only the phrasebook lines of this meter: a family (iambic, trochaic, anapestic, dactylic, common-meter, sprung, nursery, parallelism, oratory, call-and-response) or a word of a meter's name (pentameter). Implies phrasebook.", &[("maxLength", json!(64))])),
                    ("syllables", sprop("Only the phrasebook lines with this many syllables: a number (8) or a range (8-12). Implies phrasebook.", &[("maxLength", json!(16))])),
                    ("themes", json!({"type": "boolean", "description": "Also return the public-domain theme library: melodies, basses and figures composed and published before 1929, each as BeatScript bars in a stated key (let-ready), with its role, meter, work, composer, year and pairings (default false)."})),
                    ("key", sprop("Only the themes in this key, matched on its tonic: a key word (d, bm, db). Implies themes.", &[("maxLength", json!(16))])),
                    ("role", sprop("Only the themes of this role: melody, bass, harmony or arp. Implies themes.", &[("maxLength", json!(16))])),
                    ("composer", sprop("Only the themes by this composer (any part of the name: beethoven). Implies themes.", &[("maxLength", json!(64))])),
                    ("lexicon", json!({"type": "boolean", "description": format!("Also return the lexicon harvest (default false): the words the checking tools collected from programs' lyrics, ranked for the CC0 lexicon, at most {LEXICON_TOP}: those with an annotated escape (word=/phonemes/) first, then by how many programs gave them. Each is {{word, programs, conflict, in_lexicon, variants[{{phones, source (annotated or rule), count, first, last, cids}}]}}; conflict: two annotated pronunciations disagree. Read-only.")})),
                ],
                &[],
            ),
            tool(
                "lineage",
                "A program's family tree",
                crate::music_tools::description(p),
                [true, false, true, false],
                vec![("cid", sprop(&format!("The program: its CID, or a short {p}/p/<CID> link. It must be stored here (or be an example) and public."), &[("maxLength", json!(200))]))],
                &["cid"],
            ),
            tool(
                "genre_template",
                "A starting program for a genre",
                format!("{} {} {}", crate::music_tools::genre_template_description(), text::TEMPLATE_REFS, text::TEMPLATE_ROLES),
                [true, false, true, false],
                vec![
                    ("genre", sprop("A genre of the bitwrap ontology: its name (techno, house, jazz, dnb, ...), genre:<name>, or its https://ontology.bitwrap.io/genre/ IRI.", &[("maxLength", json!(200))])),
                    ("phrase", sprop("Optional, for vocals (found phrases, see docs/btw/found-phrases.md): a phrasebook id (tyger_burning_bright, from list_examples with phrasebook true) or any words. It is sung on the melody part of each section that has one, and the answer gains phrase: its source, year, meter, stress, syllables and phoneme escapes, and whether its meter fits the genre.", &[("maxLength", json!(400))])),
                    ("form", sprop("Optional (docs/btw/forms.md): a classical form, one of quodlibet, passacaglia, canon, rondo, fugue, theme-and-variations, or cross (a quodlibet of two sources, roles a and b). The program is then that form's template at the genre's tempo, arrange from template(<form>, <phrase>), and the answer gains form (shape, mutation plan, themes, reference track) and expanded (the program written out as plain BeatScript). Not with theme.", &[("maxLength", json!(64))])),
                    ("roles", json!({"type": "object", "additionalProperties": {"type": "string", "maxLength": 400}, "description": text::GENRE_TEMPLATE_ROLES})),
                    ("theme", sprop("Optional (docs/btw/forms.md): a theme of the public-domain theme library (ode_to_joy, pachelbel_ground, from list_examples with themes true). Its lets go into the program, moved to the genre's key, and the part of its role plays it in every section that has one; the answer gains theme. Not with form.", &[("maxLength", json!(64))])),
                ],
                &["genre"],
            ),
            tool("instruments", "Voices, kits and palettes", text::INSTRUMENTS_DESC.into(), [true, false, true, false], vec![], &[]),
            tool("stop", "Stop a browser tab", fill(text::STOP_DESC, &[p]), [false, false, true, false], vec![("session", session.clone()), ("device", device.clone())], &[]),
            tool("devices", "Your paired phones", fill(text::DEVICES_DESC, &[p]), [true, false, true, false], vec![], &[]),
            tool(
                "push_private",
                "Send a private program to your phone",
                fill(text::PUSH_PRIVATE_DESC, &[p]),
                [false, false, false, false],
                vec![
                    ("device", sprop("Id of one of your paired phones, from the devices tool.", &[("maxLength", json!(64)), ("pattern", json!(ID_PAT))])),
                    ("text", sprop("The program text, sealed here for the phone. Give text or sealed.", &[("maxLength", mpv.clone())])),
                    ("sealed", sprop("An envelope you sealed for the phone's seal_key (btwseal v1). Give text or sealed.", &[("maxLength", json!(65536))])),
                    ("load", json!({"type": "boolean", "description": "Load it on the phone and leave it stopped unless play (default false: play it as eval does)."})),
                    ("play", play.clone()),
                ],
                &["device"],
            ),
            tool(
                "queue",
                "Up next: queue programs on a browser tab",
                fill(text::QUEUE_DESC, &[p, &qa.to_string(), &qi.to_string(), &qid.to_string(), &qauto.to_string(), &qw.to_string()]),
                [false, true, false, false],
                vec![
                    ("action", sprop("list, add, remove, clear or skip.", &[("enum", json!(["list", "add", "remove", "clear", "skip"]))])),
                    ("items", queue_items),
                    ("item", sprop("remove: an item id from list (q1, q2, ...) or a CID (the first queued item with it).", &[("maxLength", json!(100))])),
                    ("session", session),
                    ("device", device),
                ],
                &["action"],
            ),
        ];
        v.push(listen::tool_def(max_prog, render_per_hour));
        // the lexicon harvest, said once (text::LEXICON_DESC, also docs/btw/lexicon-harvest.md's)
        for t in v.iter_mut() {
            if HARVEST_TOOLS.contains(&t["name"].as_str().unwrap_or("")) {
                let d = format!("{}\n\n{}", t["description"].as_str().unwrap_or(""), text::LEXICON_DESC);
                t["description"] = json!(d);
            }
        }
        v.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        v
    }
}

impl Tools {
    pub fn has(&self, name: &str) -> bool {
        TOOL_NAMES.contains(&name) || (name == "beta_signups" && self.beta_tool())
    }

    /// The owner's beta list tool exists only when the beta flag is set and an owner is named (`-beta-owners`).
    fn beta_tool(&self) -> bool {
        let b = crate::beta::get(&self.app);
        b.enabled() && b.has_owners()
    }

    /// `beta_signups`: list the Android beta requests, mark one added, remove one, purge the removed. Owners only.
    fn beta_signups(&self, c: &Caller, a: Args) -> R {
        a.only(&["action", "email", "status"])?;
        let b = crate::beta::get(&self.app);
        if !c.uid.as_deref().is_some_and(|u| b.is_owner(u)) {
            return Err(Fail::new(403, "forbidden", "beta_signups is for the site's owner"));
        }
        let action = a.str("action", 20)?.unwrap_or_default();
        let email = a.str("email", 254)?.unwrap_or_default();
        let status = a.str("status", 20)?.unwrap_or_default();
        b.act(&action, &email, &status).map(sorted).map_err(Fail::from)
    }

    /// tools/list with mcp-go's cursor: base64 of the last name seen; the tools after it.
    pub fn list(&self, cursor: &str) -> Result<Value, String> {
        let mut from = 0;
        if !cursor.is_empty() {
            let c = go_base64(cursor.as_bytes()).map_err(|i| format!("illegal base64 data at input byte {i}"))?;
            let c = String::from_utf8_lossy(&c).to_string();
            from = self.list.partition_point(|t| t["name"].as_str().unwrap_or("") <= c.as_str());
        }
        Ok(Value::Array(self.list[from..].to_vec()))
    }

    /// The landing page (a browser's GET /mcp).
    pub fn landing(&self) -> Response<Body> {
        Response::builder()
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CACHE_CONTROL, "no-cache")
            .body(crate::go_body(landing_html(&self.public, &self.list)))
            .unwrap()
    }
}

/// The landing page's HTML for `public` (the site's origin) and the tools of tools/list.
pub(crate) fn landing_html(public: &str, list: &[Value]) -> String {
    let e = crate::site::html_escape;
    let p = e(public);
    let endpoint = e(&format!("{public}/mcp"));
    let mut b = format!(
        r#"<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>BeatScript MCP server</title>
<meta name="author" content="Matt York">
<link rel="me" href="https://github.com/stackdump">
<link rel="me" href="https://blog.stackdump.com/">
<link rel="stylesheet" href="/mcp.css">
</head><body>
<h1>BeatScript MCP server</h1>
<p>BeatScript: AI-assisted algorithmic composition. BeatScript is also the name of its small language for writing whole music tracks as text. This endpoint is the
<a href="https://modelcontextprotocol.io">Model Context Protocol</a> over Streamable HTTP at <code>{endpoint}</code>:
check, format, share and render programs you write, and drive a browser tab at <a href="{p}">{p}</a>.
Programs are CC0. The reading list for a model is <a href="{p}/llms.txt">{p}/llms.txt</a>.</p>
<h2>Add to Claude Code</h2>
<pre><code>claude mcp add --transport http beatscript {endpoint} --header 'Authorization: Bearer btw_...'</code></pre>
<p>The name is yours to choose; older installs may show <code>btw</code>, the same server. The token keeps its <code>btw_</code> prefix.</p>
<p>Without the header the read tools still work (check, fmt, share, open, render, list_examples, instruments);
eval, load, get, stop, devices and queue need a token. There is no compose tool here: you write the program
(read the cheat-sheet, check it, fix every line:col error); composing from a prompt is a feature of the web app. Sign in on <a href="{p}">{p}</a> with a wallet or a browser key and press
<b>MCP token</b>. Other clients take the same URL and header:</p>
<pre><code>{{ "mcpServers": {{ "beatscript": {{ "type": "http", "url": "{endpoint}",
"headers": {{ "Authorization": "Bearer btw_..." }} }} }} }}</code></pre>
<h2>Tools ({})</h2>
"#,
        list.len()
    );
    for t in list {
        let name = t["name"].as_str().unwrap_or("");
        b.push_str(&format!(r#"<div class="tool"><span class="name">{}</span>"#, e(name)));
        if auth_tool(name) {
            b.push_str(r#"<span class="auth">needs a token</span>"#);
        }
        let d = t["description"].as_str().unwrap_or("");
        if !d.is_empty() {
            b.push_str(&format!(r#"<div class="desc">{}</div>"#, e(d)));
        }
        let props = t["inputSchema"]["properties"].as_object().cloned().unwrap_or_default();
        if !props.is_empty() {
            let req: Vec<&str> = t["inputSchema"]["required"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
            let mut names: Vec<&String> = props.keys().collect();
            names.sort();
            b.push_str(r#"<div class="params">"#);
            for pn in names {
                let tag = if req.contains(&pn.as_str()) { r#"<span class="req">required</span>"# } else { "" };
                let desc = props[pn]["description"].as_str().filter(|d| !d.is_empty()).map(|d| format!(" - {}", e(d))).unwrap_or_default();
                b.push_str(&format!(r#"<span class="param">{}</span>{tag}{desc}<br>"#, e(pn)));
            }
            b.push_str("</div>");
        }
        b.push_str("</div>");
    }
    b.push_str(btwserve::rights::FOOTER);
    b.push_str("</body></html>");
    b
}

impl Tools {

    // ---------- the call ----------

    fn caller(&self, ci: &CallInfo) -> Caller {
        let uid = if ci.token.is_empty() { None } else { token_user(&self.app, &ci.token) };
        let key = match (&uid, ci.ip.as_str()) {
            (Some(u), _) => format!("u:{u}"),
            (None, "") => "ip:unknown".into(),
            (None, ip) => format!("ip:{}", compose::limits::ip_key(ip)),
        };
        Caller { uid, ip: ci.ip.clone(), key }
    }

    fn auth_help(&self) -> String {
        compose::auth_help(&self.public)
    }

    /// One tools/call: the `CallToolResult`. The checker tools run on a blocking thread (the engine blocks).
    pub async fn call(self: &Arc<Self>, name: &str, args: Value, ci: &CallInfo) -> Value {
        let c = self.caller(ci);
        let r = if auth_tool(name) && c.uid.is_none() {
            Err(Fail::new(401, "auth_required", format!("{name} needs a signed-in caller. {}", self.auth_help())))
        } else {
            let a = match args {
                Value::Null => Ok(Args(Map::new())),
                Value::Object(m) => Ok(Args(m)),
                _ => Err(bad(format!("the arguments of {name} must be a JSON object"))),
            };
            match a {
                Err(e) => Err(e),
                Ok(a) if name == "get" => self.get(&c, a).await.map(Text::Json),
                Ok(a) => {
                    let (me, n) = (self.clone(), name.to_string());
                    tokio::task::spawn_blocking(move || me.dispatch(&n, &c, a)).await.unwrap_or_else(|_| Err(Fail::new(500, "internal", "the server could not complete that call; try again")))
                }
            }
        };
        match r {
            Ok(v) => ok_result(v),
            Err(e) => err_result(e),
        }
    }

    fn dispatch(&self, name: &str, c: &Caller, a: Args) -> Result<Text, Fail> {
        match name {
            "check" => self.check(c, a).map(Text::Json),
            "fmt" => self.fmt_tool(c, a).map(Text::Json),
            "expand" => self.expand_tool(c, a).map(Text::Json),
            "generate" => self.generate_tool(c, a).map(Text::Json),
            "eval" => self.eval(c, a).map(Text::Json),
            "load" => self.load(c, a).map(Text::Json),
            "share" => self.share_tool(c, a).map(Text::Json),
            "open" => self.open(c, a).map(Text::Json),
            "render" => self.render_tool(c, a).map(Text::Json),
            "listen" => self.listen_tool(c, a).map(Text::Json),
            "record" => self.record_tool(c, a).map(Text::Json),
            "list_examples" => self.list_examples(a).map(Text::Json),
            "instruments" => self.instruments(a).map(Text::Raw),
            "stop" => self.stop(c, a).map(Text::Json),
            "devices" => self.devices(c, a).map(Text::Json),
            "queue" => self.queue_tool(c, a).map(Text::Json),
            "genre_template" => {
                a.only(&["genre", "phrase", "form", "theme", "roles"])?;
                let g = a.str("genre", 200)?.unwrap_or_default();
                if g.trim().is_empty() {
                    return Err(bad("argument \"genre\" is empty: name a genre (techno, house, jazz ...)"));
                }
                let phrase = a.str("phrase", 400)?;
                let form = a.str("form", 64)?;
                let theme = a.str("theme", 64)?;
                // a form's roles, each bound to a source: {"a": "gen@1 dubstep seed=54876", "b": "ode_to_joy"}
                let mut roles: Vec<(String, String)> = Vec::new();
                if let Some(v) = a.present("roles") {
                    let Some(o) = v.as_object() else { return Err(bad("roles is an object: {\"ROLE\": \"SOURCE\", ...}")) };
                    if o.len() > 8 {
                        return Err(bad("at most 8 roles"));
                    }
                    for (k, v) in o {
                        let Some(src) = v.as_str().filter(|s| s.len() <= 400 && !s.contains('\n')) else { return Err(bad(format!("roles.{k} must be a source on one line, at most 400 characters"))) };
                        if k.is_empty() || !k.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                            return Err(bad(format!("{k:?} is no role name")));
                        }
                        roles.push((k.clone(), src.to_string()));
                    }
                }
                self.limit_cpu(c)?;
                let mut v = btwserve::genres::installed_template_roles(&g, phrase.as_deref(), form.as_deref(), theme.as_deref(), &roles, &self.public).map(with_references).map_err(|m| Fail::new(404, "not_found", m))?;
                // the lexicon harvest of the program it answers (written out, for a form)
                let text = v["expanded"].as_str().or(v["text"].as_str()).unwrap_or("").to_string();
                v["lexicon"] = self.harvest(&text, None);
                Ok(Text::Json(v))
            }
            "lineage" => {
                a.only(&["cid"])?;
                let cid = a.str("cid", 200)?.unwrap_or_default();
                self.limit_cpu(c)?;
                crate::music_tools::call(&crate::lineage(&self.app), &self.public, &cid).map(Text::Json)
            }
            "push_private" => self.push_private(c, a).map(Text::Json),
            "beta_signups" if self.beta_tool() => self.beta_signups(c, a).map(Text::Json),
            _ => Err(Fail::new(404, "not_found", "no such tool")),
        }
    }

    // ---------- shared steps ----------

    fn ip_ceiling(&self, l: &Limiter, c: &Caller) -> Result<(), u64> {
        if c.uid.is_none() {
            return Ok(());
        }
        let ip = if c.ip.is_empty() { "unknown" } else { c.ip.as_str() };
        l.allow(&format!("ip:{}", compose::limits::ip_key(ip)))
    }

    fn limit_cpu(&self, c: &Caller) -> Result<(), Fail> {
        if let Err(r) = self.cpu.allow(&c.key) {
            return Err(Fail::from(crate::ApiError::new(429, "rate_limited", "too many calls to the checker; slow down").retry(r)));
        }
        if let Err(r) = self.ip_ceiling(&self.cpu_ip, c) {
            return Err(Fail::from(crate::ApiError::new(429, "rate_limited", "too many calls to the checker from your address; slow down").retry(r)));
        }
        Ok(())
    }

    fn check_program(&self, c: &Caller, text: &str) -> R {
        self.limit_cpu(c)?;
        Ok(self.eng.check(&c.ip, text)?)
    }

    fn program(&self, a: &Args, key: &str) -> Result<String, Fail> {
        let v = match a.str(key, self.max_prog) {
            Err(mut e) if e.err.code == "too_large" => {
                let n = a.0[key].as_str().map_or(0, str::len);
                e.err.message = format!("the program is {n} bytes; programs are limited to {} bytes (about {} lines). Shorten it: use let and def to say a pattern once.", self.max_prog, self.max_prog / 40);
                return Err(e);
            }
            r => r?,
        };
        match v {
            Some(v) if !v.trim().is_empty() => Ok(v),
            _ => Err(bad(format!("argument {} is required: the text of a BeatScript program", q(key)))),
        }
    }

    fn session(&self, a: &Args) -> Result<String, Fail> {
        let v = a.str("session", 64)?;
        match v {
            Some(s) if !id_ok(&s) => Err(bad("argument \"session\" is not a session id; copy the id the page's MCP panel shows (letters, digits, - and _)")),
            Some(s) => Ok(s),
            None => Ok(String::new()),
        }
    }

    fn target(&self, c: &Caller, a: &Args) -> Result<String, Fail> {
        let session = self.session(a)?;
        let dev = a.str("device", 64)?.unwrap_or_default();
        if dev.is_empty() {
            return Ok(session);
        }
        if !id_ok(&dev) {
            return Err(bad("argument \"device\" is not a device id; call devices for the ids of your paired phones"));
        }
        if !session.is_empty() {
            return Err(bad("give session or device, not both"));
        }
        self.hub.device_session(c.uid.as_deref().unwrap_or(""), &dev)
    }

    fn no_session(&self, session: &str) -> Fail {
        let msg = if !session.is_empty() {
            format!("No connected browser has session {} (it expired, was closed, or belongs to another account). Open {} in a browser tab, press MCP in the top bar to see its session id, and pass that id as `session`.", q(session), self.public)
        } else {
            format!("No browser is connected to drive. Open {} in a browser tab signed in as the account this token belongs to, then call again. For an anonymous tab, press MCP in the top bar of the page: it shows a session id; pass it as `session`.", self.public)
        };
        Fail::new(404, "no_session", msg)
    }

    fn push(&self, c: &Caller, session: &str, ev: SessionEvent) -> Result<usize, Fail> {
        match self.hub.push(c.uid.as_deref().unwrap_or(""), session, ev) {
            Err(e) if e.err.code == "no_session" => Err(self.no_session(session)),
            Err(e) => Err(e),
            Ok(0) => Err(self.no_session(session)),
            Ok(n) => Ok(n),
        }
    }

    fn example_cid(&self, name: &str) -> Option<String> {
        self.app.examples.list.iter().find(|e| e.name == name).map(|e| e.cid.clone())
    }

    fn example_text(&self, name: &str) -> Option<String> {
        self.app.examples.text.get(name).cloned()
    }

    /// `storedProgram`: (text, "store", meta).
    fn stored(&self, cid: &str) -> Option<(String, &'static str, Value)> {
        compose::stored(&self.app, cid).map(|(t, m)| (t, "store", m))
    }

    fn open_report(&self, c: &Caller, url: &str) -> R {
        self.limit_cpu(c)?;
        let rep = self.eng.open(&c.ip, url)?;
        if rep["ok"] != true || rep["verified"] != true {
            let msg = rep["error"].as_str().filter(|s| !s.is_empty()).unwrap_or("the link did not open").to_string();
            let code = if !rep["canonical"].as_str().unwrap_or("").is_empty() && rep["verified"] != true { "cid_mismatch" } else { "bad_request" };
            return Err(Fail::new(422, code, msg));
        }
        Ok(rep)
    }

    /// The lexicon harvest's side effect: the lyric pairs of `text` (`djengine::vocal::lexicon::lyric_pairs`), recorded
    /// when the program checks and may be kept: not one that uses samples or loops, says `use restricted` or is not public
    /// (push_private never comes here). `rep` is its check report; None checks it here, unbilled, and only when there is a
    /// pair to record. Only the word, its phonemes, the source and the CID are kept.
    fn harvest_raw(&self, text: &str, rep: Option<&Value>) -> btwserve::diagnostics::Recorded {
        let mut pairs: Vec<djengine::vocal::lexicon::LyricPair> = Vec::new();
        for l in djlive::sing::lyrics(text) {
            for p in djengine::vocal::lexicon::lyric_pairs(&l) {
                if !pairs.contains(&p) {
                    pairs.push(p);
                }
            }
        }
        if pairs.is_empty() || crate::store::declares_not_public(text) {
            return Default::default();
        }
        let owned;
        let rep = match rep {
            Some(r) => r,
            None => match self.eng.check("", text) {
                Ok(v) => {
                    owned = v;
                    &owned
                }
                Err(_) => return Default::default(),
            },
        };
        let r = Rep(rep);
        if !r.ok() || r.restricted() || r.uses_assets() {
            return Default::default();
        }
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        self.harvest.record(r.cid(), &pairs, now)
    }

    /// [`Tools::harvest_raw`] as the answer's `lexicon` field.
    fn harvest(&self, text: &str, rep: Option<&Value>) -> Value {
        self.harvest_raw(text, rep).json()
    }

    // ---------- check, fmt ----------

    fn check(&self, c: &Caller, a: Args) -> R {
        a.only(&["text", "same_as", "genre", "expand"])?;
        let text = self.program(&a, "text")?;
        let genre = a.str("genre", 200)?;
        let expand = a.boolean("expand")?;
        let other = match a.str("same_as", self.max_prog)? {
            None => None,
            Some(v) if v.trim().is_empty() => return Err(bad("argument \"same_as\" is empty: give a CID or a program's text")),
            Some(v) => match bare_cid(&v) {
                Some(cid) => match self.stored(&cid) {
                    Some((t, ..)) => Some(t),
                    None => return Err(Fail::new(404, "not_found", format!("same_as: no program with CID {cid} is stored here or among the examples; pass its text instead"))),
                },
                None => Some(v),
            },
        };
        self.limit_cpu(c)?;
        let mut rep = self.eng.check_with(&c.ip, &text, expand)?;
        // the genre's conventions (the argument, else the `#! genre:` line): style warnings, never errors
        if let Some(g) = btwserve::genres::wanted(&text, genre.as_deref()) {
            rep.as_object_mut().unwrap().insert("style".into(), btwserve::genres::installed_style(&text, &g));
        }
        let lexicon = self.harvest(&text, Some(&rep));
        rep.as_object_mut().unwrap().insert("lexicon".into(), lexicon);
        let Some(other) = other else { return Ok(rep) };
        self.limit_cpu(c)?;
        let cmp: Value = serde_json::from_str(&self.eng.same_as(&c.ip, &text, &other)?).unwrap_or(Value::Null);
        rep.as_object_mut().unwrap().insert("same_as".into(), cmp);
        Ok(rep)
    }

    fn expand_tool(&self, c: &Caller, a: Args) -> R {
        a.only(&["text", "lineage", "eject"])?;
        let text = self.program(&a, "text")?;
        let lineage = a.boolean("lineage")?;
        let eject = a.str("eject", 16)?;
        self.limit_cpu(c)?;
        match eject {
            None => Ok(self.eng.expand(&text, lineage)?),
            Some(at) => Ok(eject_at(&text, &at)?),
        }
    }

    fn fmt_tool(&self, c: &Caller, a: Args) -> R {
        a.only(&["text"])?;
        let text = self.program(&a, "text")?;
        self.limit_cpu(c)?;
        Ok(self.eng.fmt(&c.ip, &text)?)
    }

    /// The seeded generator (btwapi::generate): no model, no sign-in. The program is stored like compose's (public,
    /// CC0, source "generate", its seed in the meta) and comes back with its render and share links.
    fn generate_tool(&self, c: &Caller, a: Args) -> R {
        a.only(&["genre", "bpm", "key", "length", "seed", "seeds", "generator", "part", "section"])?;
        let one_seed = |v: &Value, what: &str| match v {
            Value::String(s) if s.len() <= 64 => Ok(s.clone()),
            Value::Number(n) if n.as_u64().is_some() => Ok(n.to_string()),
            _ => Err(bad(format!("{what} must be a whole number or a word (at most 64 characters)"))),
        };
        let seed = match a.present("seed") {
            None => String::new(),
            Some(v) => one_seed(v, "argument \"seed\"")?,
        };
        let ask = GenAsk {
            req: btwapi::GenRequest {
                genre: a.str("genre", 32)?.unwrap_or_default(),
                bpm: a.num("bpm", 40.0, 300.0)?,
                key: a.str("key", 16)?.unwrap_or_default(),
                length: a.str("length", 16)?.unwrap_or_default(),
                seed,
            },
            version: gen_version(a.present("generator"))?,
            part: a.str("part", 16)?.filter(|p| !p.trim().is_empty()),
            section: a.str("section", 32)?.filter(|s| !s.trim().is_empty()),
        };
        if let Some(p) = &ask.part {
            if !djlive::parse::ROLES.contains(&p.as_str()) {
                return Err(bad(format!("argument \"part\" is a role: one of {}", djlive::parse::ROLES.join(", "))));
            }
        }
        let Some(seeds) = a.present("seeds") else {
            self.limit_cpu(c)?;
            let mut v = generate_and_store(&self.app, &self.eng, &self.public, &c.ip, &ask)?;
            v["lexicon"] = self.harvest(v["text"].as_str().unwrap_or(""), None);
            return Ok(v);
        };
        // many seeds: a summary each, every program stored as one generate stores it, nothing else
        if a.present("seed").is_some() {
            return Err(bad("give seed or seeds, not both"));
        }
        let list: Vec<String> = match seeds {
            Value::Array(v) if !v.is_empty() && v.len() <= MAX_SEEDS => v.iter().enumerate().map(|(i, s)| one_seed(s, &format!("seeds[{i}]"))).collect::<Result<_, _>>()?,
            Value::Array(v) => return Err(bad(format!("argument \"seeds\" is a list of 1 to {MAX_SEEDS} seeds (it has {})", v.len()))),
            Value::Object(o) => {
                if let Some(k) = o.keys().find(|k| !matches!(k.as_str(), "count" | "start")) {
                    return Err(bad(format!("argument \"seeds\" takes count and start, not {}", q(k))));
                }
                let count = o.get("count").and_then(Value::as_u64).filter(|n| (1..=MAX_SEEDS as u64).contains(n)).ok_or_else(|| bad(format!("seeds.count is a whole number from 1 to {MAX_SEEDS}")))?;
                let start = match o.get("start") {
                    None | Some(Value::Null) => 1,
                    Some(v) => v.as_u64().filter(|s| *s <= u64::from(u32::MAX)).ok_or_else(|| bad("seeds.start is a whole number (the first seed)"))?,
                };
                (start..start + count).map(|s| s.to_string()).collect()
            }
            _ => return Err(bad(format!("argument \"seeds\" is a list of seeds, or {{count, start}} (at most {MAX_SEEDS})"))),
        };
        // each seed is a generate: charged as one
        for _ in &list {
            self.limit_cpu(c)?;
        }
        let mut out = Vec::new();
        for s in list {
            let one = GenAsk { req: btwapi::GenRequest { seed: s, ..ask.req.clone() }, ..ask.clone() };
            out.push(generate_summary(&self.app, &self.eng, &self.public, &c.ip, &one));
        }
        let gen = format!("{}@{}", btwapi::generate::GENERATOR, ask.version);
        Ok(json!({"generator": gen, "count": out.len(), "programs": out, "license": "CC0-1.0", "note": "One summary a seed, in order; each program is stored as generate stores it (url, render_url). Call generate with one seed for its text and each part's let."}))
    }

    // ---------- eval, load, stop, get ----------

    fn eval(&self, c: &Caller, a: Args) -> R {
        a.only(&["text", "play", "session", "device"])?;
        let text = self.program(&a, "text")?;
        let play = a.boolean("play")?;
        let session = self.target(c, &a)?;
        let rep = self.check_program(c, &text)?;
        let r = Rep(&rep);
        if !r.ok() {
            return Err(needs_fixing(&rep, "nothing was sent to the browser"));
        }
        if let Some(e) = private(&rep, "it cannot be played in the browser") {
            return Err(e);
        }
        // (checked as it was sent, so an error is at the line and column the sender wrote; it arrives formatted)
        let n = self.push(c, &session, SessionEvent { kind: "eval", text: btwserve::autoformat::autoformat(&text, self.max_prog), cid: r.cid().into(), play })?;
        Ok(sorted(json!({
            "sent": true, "delivered": n, "cid": r.cid(), "play": play, "warnings": r.warnings(), "lexicon": self.harvest(&text, Some(&rep)),
            "warning_diagnostics": warnings_of(&rep),
            "note": "Sent. The tab evaluates it like Cmd+Enter: the change lands on the next bar. Call get to see what is playing and any diagnostics the page reports.",
        })))
    }

    /// One of cid, example, url (a CID-only url is the cid path).
    fn one_program(&self, a: &Args) -> Result<(String, String, String), Fail> {
        let mut cid = a.str("cid", 100)?.unwrap_or_default();
        let example = a.str("example", 64)?.unwrap_or_default();
        let mut url = a.str("url", 32768)?.unwrap_or_default();
        if !url.is_empty() && a.present("cid").is_none() {
            if let Some(sc) = bare_cid(&url) {
                cid = sc;
                url.clear();
            }
        }
        let n = [!cid.is_empty(), !example.is_empty(), !url.is_empty()].iter().filter(|b| **b).count();
        if n != 1 {
            return Err(bad("give exactly one of cid, example, url"));
        }
        Ok((cid, example, url))
    }

    fn load(&self, c: &Caller, a: Args) -> R {
        a.only(&["cid", "example", "url", "play", "session", "device"])?;
        let (mut cid, example, url) = self.one_program(&a)?;
        let play = a.boolean("play")?;
        let session = self.target(c, &a)?;
        let (text, source) = if !cid.is_empty() {
            if !is_cid(&cid) {
                return Err(bad("argument \"cid\" is not a CID (it starts with z)"));
            }
            match self.stored(&cid) {
                Some((t, src, _)) => (t, src),
                None => return Err(Fail::new(404, "not_found", format!("no stored program has CID {cid}. Programs are stored by render and by share when signed in; for any other program pass its text to eval, or a share link as url."))),
            }
        } else if !example.is_empty() {
            if !name_ok(&example) {
                return Err(bad("argument \"example\" is an example name, as list_examples gives"));
            }
            let Some(t) = self.example_text(&example) else { return Err(Fail::new(404, "not_found", format!("no example is named {}; call list_examples", q(&example)))) };
            cid = self.example_cid(&example).unwrap_or_default();
            (t, "example")
        } else {
            let rep = self.open_report(c, &url)?;
            cid = rep["cid"].as_str().unwrap_or("").into();
            (rep["canonical"].as_str().unwrap_or("").to_string(), "link")
        };
        let n = self.push(c, &session, SessionEvent { kind: "load", text: btwserve::autoformat::autoformat(&text, self.max_prog), cid: cid.clone(), play })?;
        Ok(sorted(json!({"sent": true, "delivered": n, "cid": cid, "source": source, "play": play, "lexicon": self.harvest(&text, None), "note": "Loaded into the tab. Call get to see what is playing."})))
    }

    fn stop(&self, c: &Caller, a: Args) -> R {
        a.only(&["session", "device"])?;
        let session = self.target(c, &a)?;
        let n = self.push(c, &session, SessionEvent { kind: "stop", ..Default::default() })?;
        Ok(sorted(json!({"sent": true, "delivered": n, "note": "Stopped: held filters and mutes are put back."})))
    }

    async fn get(&self, c: &Caller, a: Args) -> R {
        a.only(&["session", "device"])?;
        let mut session = self.target(c, &a)?;
        let uid = c.uid.clone().unwrap_or_default();
        let mut connected = 0;
        if session.is_empty() {
            let list = self.hub.sessions(&uid);
            if list.is_empty() {
                return Err(self.no_session(""));
            }
            let mut best = (list[0].id.clone(), list[0].last_seen.clone());
            for s in &list[1..] {
                if s.last_seen > best.1 {
                    best = (s.id.clone(), s.last_seen.clone());
                }
            }
            session = best.0;
            connected = list.len();
        }
        let st = match tokio::time::timeout(Duration::from_secs(8), self.hub.state(&uid, &session)).await {
            Err(_) => return Err(Fail::new(504, "timeout", "the server took too long; try again")),
            Ok(Err(e)) if e.err.code == "no_session" => {
                if connected > 0 {
                    return Err(Fail::new(409, "no_session", format!("{} If the tab was closed, open {} again signed in as this account, or press MCP in its top bar and pass the session id it shows as `session`.", e.err.message, self.public)));
                }
                return Err(self.no_session(&session));
            }
            Ok(Err(e)) => return Err(e),
            Ok(Ok(st)) => st,
        };
        let Value::Object(mut out) = st else { return Err(self.no_session(&session)) };
        listen::mixer_report(&mut out);
        let st_cid = out.get("cid").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let st_id = out.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if !st_cid.is_empty() {
            out.insert("jsonld_url".into(), json!(format!("{}/c/{st_cid}.jsonld", self.public)));
            out.insert("context".into(), json!(format!("{}/context/v1.jsonld", self.public)));
        }
        // the program as its `<beatscript-view>` block's JSON-LD (djlive progld): canonical text, @id its CID; the same
        // document the pages embed, so a client reads one shape everywhere
        // the doc comments of the tab's text (the `#` lines above a section, let, def or arrange): `docs`, and each
        // documented section's `description` in the block
        let docs = out.get("text").and_then(Value::as_str).map(djlive::doc::doc_comments).unwrap_or_default();
        if !docs.is_empty() {
            out.insert("docs".into(), serde_json::from_str(&btwapi::docs_json(&docs).js_stringified()).unwrap_or(Value::Null));
        }
        if let Some(c) = out.get("text").and_then(Value::as_str).and_then(|t| djlive::compile::check(t).compiled) {
            let onto: &dyn djlive::linked::Ontology = djhost::ontology::vocabulary();
            let f = djlive::progld::Facts { ontology: Some(onto), docs: &docs, ..Default::default() };
            out.insert("program_ld".into(), serde_json::from_str(&djlive::progld::document(&c, &f).js_stringified()).unwrap_or(Value::Null));
        }
        if connected > 0 {
            out.insert("sessions_connected".into(), json!(connected));
        }
        if let Ok((v, _)) = self.hub.queue(&uid, &st_id, QueueOp { action: "list".into(), items: Vec::new(), remove: String::new() }) {
            if let Some(u) = v["up_next"].as_array().filter(|u| !u.is_empty()) {
                out.insert("up_next".into(), Value::Array(u.clone()));
            }
            let mut qi = Map::new();
            if v["now_playing"].is_object() {
                qi.insert("now_playing".into(), v["now_playing"].clone());
            }
            qi.insert("position".into(), json!(v["position"].as_i64().unwrap_or(0)));
            qi.insert("length".into(), json!(v["up_next"].as_array().map_or(0, Vec::len)));
            if !v["paused_reason"].as_str().unwrap_or("").is_empty() {
                qi.insert("paused_reason".into(), v["paused_reason"].clone());
            }
            qi.insert("listener".into(), json!(v["listener"].as_str().unwrap_or("")));
            qi.insert("auto_advanced".into(), json!(v["auto_advanced"].as_i64().unwrap_or(0)));
            out.insert("queue".into(), Value::Object(qi));
        }
        Ok(Value::Object(out))
    }

    /// `push_private`: sealed for the phone and relayed in memory; nothing here is stored or checked against the
    /// storage gate, because nothing is stored (hub::private).
    fn push_private(&self, c: &Caller, a: Args) -> R {
        a.only(&["device", "text", "sealed", "load", "play"])?;
        let Some(uid) = c.uid.as_deref() else {
            return Err(Fail::new(401, "auth_required", format!("push_private needs a signed-in caller: it goes only to your own paired phones. {}", self.auth_help())));
        };
        let dev = a.str("device", 64)?.unwrap_or_default();
        if !id_ok(&dev) {
            return Err(bad("argument \"device\" is a device id; call devices for the ids of your paired phones"));
        }
        let text = a.str("text", self.max_prog)?.unwrap_or_default();
        let sealed = a.str("sealed", btwseal::MAX_ENVELOPE)?.unwrap_or_default();
        let what = match (text.is_empty(), sealed.is_empty()) {
            // (a `text` the server seals is formatted first, as eval's is; a `sealed` envelope the server cannot read)
            (false, true) => crate::mcp::hubapi::PrivatePush::Text { kind: if a.boolean("load")? { "load" } else { "eval" }, text: btwserve::autoformat::autoformat(&text, self.max_prog), play: a.boolean("play")? || !a.boolean("load")? },
            (true, false) => crate::mcp::hubapi::PrivatePush::Sealed(sealed),
            _ => return Err(bad("give exactly one of text, sealed")),
        };
        self.limit_cpu(c)?;
        let mut v = self.hub.push_private(uid, &dev, what, self.max_prog)?;
        v["note"] = json!("Sent to the phone only, sealed for its key; nothing was stored. The phone checks and plays it: call get with this device for what it reports (missing samples are listed in its diagnostics).");
        Ok(sorted(v))
    }

    fn devices(&self, c: &Caller, a: Args) -> R {
        a.only(&[])?;
        let mut list = self.hub.devices(c.uid.as_deref().unwrap_or(""));
        for d in list.iter_mut() {
            if let Some(k) = self.hub.device_key(c.uid.as_deref().unwrap_or(""), d["id"].as_str().unwrap_or("")) {
                // what a client seals its own private pushes for (btwseal: the X25519 form of this key)
                d["seal_key"] = json!(k);
            }
        }
        let n = list.iter().filter(|d| d["connected"] == true).count();
        let mut note = "Pass a phone's id as device to eval, load, get or stop to drive that phone alone.".to_string();
        if list.is_empty() {
            note = format!("No phone is paired with this account. Open {} signed in, press Mobile, and scan the code from the BeatScript app (menu, then Scan to connect).", self.public);
        } else if n == 0 {
            note = format!("No paired phone is connected now: open djrust on the phone (it connects on its own). {note}");
        }
        Ok(sorted(json!({"devices": list, "connected": n, "note": note})))
    }

    // ---------- share, open ----------

    fn site_links(&self, url: &str, social: &str) -> (String, String) {
        let Some(i) = url.find('#') else { return (url.into(), social.into()) };
        if self.public.is_empty() {
            return (url.into(), social.into());
        }
        let frag = &url[i + 1..];
        let sealed = frag.split('~').next().unwrap_or("");
        (format!("{}/#{frag}", self.public), format!("{}/c/{sealed}", self.public))
    }

    fn rights(&self, a: &Args) -> R {
        let mut m = json!({"license": "CC0-1.0", "visibility": "public", "source": "share"});
        if let Some(v) = a.str("license", 40)? {
            if !v.trim().is_empty() {
                if !license_ok(v.trim()) {
                    return Err(bad("argument \"license\" is a short SPDX-style id such as CC0-1.0 or CC-BY-4.0"));
                }
                m["license"] = json!(v.trim());
            }
        }
        if let Some(v) = a.str("forked_from", 100)? {
            if !v.trim().is_empty() {
                if !is_cid(v.trim()) {
                    return Err(bad("argument \"forked_from\" is a CID (it starts with z)"));
                }
                m["forked_from"] = json!(v.trim());
            }
        }
        if let Some(v) = a.str("name", 80)? {
            match one_line(&v, 80) {
                Some(n) => m["name"] = json!(n),
                None if !v.trim().is_empty() => return Err(bad("argument \"name\" is one line of at most 80 characters")),
                None => {}
            }
        }
        if let Some(raw) = a.present("credits") {
            let list = raw.as_array().filter(|l| l.len() <= MAX_CREDITS).ok_or_else(|| bad(format!("argument \"credits\" is a list of at most {MAX_CREDITS} short lines")))?;
            let mut cr = Vec::new();
            for it in list {
                let line = it.as_str().and_then(|s| one_line(s, MAX_CREDIT_CHARS)).filter(|l| !l.is_empty());
                let Some(line) = line else { return Err(bad(format!("each credit is one line of 1 to {MAX_CREDIT_CHARS} characters"))) };
                cr.push(line);
            }
            m["credits"] = json!(cr);
        }
        Ok(m)
    }

    fn share_tool(&self, c: &Caller, a: Args) -> R {
        a.only(&["text", "license", "forked_from", "credits", "name"])?;
        let text = self.program(&a, "text")?;
        let meta = self.rights(&a)?;
        self.limit_cpu(c)?;
        let chk = self.eng.check(&c.ip, &text)?;
        if !Rep(&chk).ok() {
            return Err(needs_fixing(&chk, "it cannot be shared"));
        }
        if let Some(e) = private(&chk, "it cannot be shared") {
            return Err(e);
        }
        if crate::store::declares_not_public(&text) {
            return Err(Fail::new(422, "restricted", "the program says `#! visibility:` something but public, so it cannot be shared or stored: private programs never leave the machine they were written on."));
        }
        let rep = self.eng.share(&c.ip, &text)?;
        if rep["ok"] != true {
            return Err(Fail::new(422, "bad_request", "the program cannot be shared"));
        }
        let (url, social) = self.site_links(rep["url"].as_str().unwrap_or(""), rep["social_url"].as_str().unwrap_or(""));
        let mut out = json!({
            "ok": true, "url": url, "social_url": social, "beats_url": rep["beats_url"], "cid": rep["cid"], "chars": rep["chars"],
            "canonical": rep["canonical"], "warnings": rep.get("warnings").cloned().unwrap_or(json!([])),
            "jsonld_url": format!("{social}.jsonld"), "context": format!("{}/context/v1.jsonld", self.public),
            "note": "jsonld_url is the program as linked data (its @context is the context URL). url is the self-sealing link (the program is in the link; opening it checks the CID). Post social_url where a link preview is wanted.",
            "lexicon": self.harvest(&text, Some(&chk)),
        });
        let none = |out: &mut Value, note: String| {
            out["short_url"] = Value::Null;
            out["short_social_url"] = Value::Null;
            out["short_beats_url"] = Value::Null;
            out["embed_url"] = Value::Null;
            out["stored"] = json!(false);
            out["short_note"] = json!(note);
        };
        if c.uid.is_none() {
            none(&mut out, format!("No short (CID-only) link: that needs a signed-in call, because the program is stored on the site. {}", self.auth_help()));
            return Ok(sorted(out));
        }
        let lim = self.share.allow(&c.key).and_then(|_| self.ip_ceiling(&self.share_ip, c));
        if let Err(r) = lim {
            return Err(Fail::from(crate::ApiError::new(429, "rate_limited", "too many programs stored by share; the self-sealing link above still works").retry(r)));
        }
        // what is stored is the canonical text (its CID is of that text); it is handed over with the formatted one, whose
        // `#! name:` header names it
        let formatted = btwserve::autoformat::autoformat(&text, self.max_prog);
        match compose::put(&self.app, &self.eng, &c.ip, &formatted, meta) {
            Ok((prog, _)) if prog["cid"] == rep["cid"] => {
                let cid = prog["cid"].as_str().unwrap_or("");
                out["short_url"] = json!(format!("{}/p/{cid}", self.public));
                out["short_social_url"] = json!(format!("{}/c/{cid}", self.public));
                out["short_beats_url"] = json!(format!("https://beats.bitwrap.io/p/{cid}"));
                // a stored program is a public Creative Commons one (the storage gate), so it embeds (6664)
                out["embed_url"] = json!(format!("{}{}{cid}", self.public, crate::cards::EMBED_PREFIX));
                out["stored"] = json!(true);
                out["meta"] = prog["meta"].clone();
                out["fetch_url"] = json!(format!("{}/api/programs/{cid}", self.public));
                out["note"] = json!(format!("url is the self-sealing link (the program is in the link; opening it checks the CID). short_url is the CID-only link: the program is stored on the site (public, {}; the first share's rights stay), and opening it fetches the text and checks it against the CID. Post social_url or short_social_url where a link preview is wanted.", prog["meta"]["license"].as_str().unwrap_or("")));
            }
            Ok(_) => none(&mut out, "the program could not be stored. The self-sealing link above still works.".into()),
            Err(e) => none(&mut out, format!("{}. The self-sealing link above still works.", e.err.message)),
        }
        Ok(sorted(out))
    }

    fn open(&self, c: &Caller, a: Args) -> R {
        a.only(&["url"])?;
        let u = a.str("url", 32768)?.unwrap_or_default();
        let u = u.trim();
        if u.is_empty() {
            return Err(bad("argument \"url\" is required: a share link, a #fragment or a CID"));
        }
        if let Some(cid) = bare_cid(u) {
            return self.open_stored(c, &cid);
        }
        self.limit_cpu(c)?;
        let rep = self.eng.open(&c.ip, u)?;
        if rep["ok"] != true {
            let msg = rep["error"].as_str().filter(|s| !s.is_empty()).unwrap_or("the link did not open").to_string();
            let code = if !rep["canonical"].as_str().unwrap_or("").is_empty() && rep["verified"] != true { "cid_mismatch" } else { "bad_request" };
            return Err(Fail::new(422, code, msg));
        }
        Ok(rep)
    }

    fn open_stored(&self, c: &Caller, cid: &str) -> R {
        let Some((text, src, meta)) = self.stored(cid) else {
            return Err(Fail::new(404, "not_found", format!("no program {cid} is stored here. A CID alone names a program but does not carry it: open the self-sealing link (#<CID>.<packed>) instead, or share the program signed in to store it.")));
        };
        self.limit_cpu(c)?;
        let chk = self.eng.check(&c.ip, &text)?;
        let r = Rep(&chk);
        if !r.ok() || r.cid().is_empty() {
            return Err(Fail::new(422, "bad_request", format!("the text stored under {cid} does not check; it is not loaded")));
        }
        if r.cid() != cid {
            return Err(Fail::new(422, "cid_mismatch", format!("refused: the text stored under {cid} is program {}. Do not trust or play it.", r.cid())));
        }
        let warnings: Vec<String> = r.diagnostics().iter().filter(|d| d["severity"] == "warning").map(|d| format!("{}:{} {}", d["line"], d["col"], d["message"].as_str().unwrap_or(""))).collect();
        Ok(sorted(json!({"ok": true, "cid": cid, "canonical": r.canonical(), "verified": true, "source": src, "meta": compose::go_meta(&meta), "warnings": warnings})))
    }

    // ---------- render, record ----------

    fn render_tool(&self, c: &Caller, a: Args) -> R {
        a.only(&["text", "cid", "example", "format"])?;
        let mut text = a.str("text", self.max_prog)?.unwrap_or_default();
        let mut cid = a.str("cid", 100)?.unwrap_or_default();
        let example = a.str("example", 64)?.unwrap_or_default();
        let n = [!text.trim().is_empty(), !cid.is_empty(), !example.is_empty()].iter().filter(|b| **b).count();
        if n != 1 {
            return Err(bad("give exactly one of text, cid, example"));
        }
        let format = a.str("format", 8)?.filter(|f| !f.is_empty()).unwrap_or_else(|| "opus".into());
        let export = crate::export_route::FORMATS.contains(&format.as_str());
        if format != "opus" && format != "wav" && !export {
            return Err(bad("argument \"format\" is opus, wav, midi, stems or stems-opus"));
        }
        let mut seconds = None;
        let mut ends = true;
        let lexicon;
        if !cid.is_empty() {
            if !is_cid(&cid) {
                return Err(bad("argument \"cid\" is not a CID (it starts with z)"));
            }
            let Some((t, ..)) = self.stored(&cid) else {
                return Err(Fail::new(404, "not_found", format!("no stored program has CID {cid}. Pass its text as `text` and render stores it.")));
            };
            lexicon = self.harvest(&t, None);
        } else {
            let mut source = "put";
            if !example.is_empty() {
                if !name_ok(&example) {
                    return Err(bad("argument \"example\" is an example name, as list_examples gives"));
                }
                let Some(t) = self.example_text(&example) else { return Err(Fail::new(404, "not_found", format!("no example is named {}; call list_examples", q(&example)))) };
                text = t;
                source = "example";
            }
            let rep = self.check_program(c, &text)?;
            if !Rep(&rep).ok() {
                return Err(needs_fixing(&rep, "it cannot be rendered"));
            }
            if let Some(e) = private(&rep, "it cannot be rendered or stored") {
                return Err(e);
            }
            let lim = self.render.allow(&c.key).and_then(|_| self.ip_ceiling(&self.render_ip, c));
            if let Err(r) = lim {
                return Err(Fail::from(crate::ApiError::new(429, "rate_limited", "too many programs submitted for rendering").retry(r)));
            }
            let (prog, _) = compose::put(&self.app, &self.eng, &c.ip, &text, json!({"license": "CC0-1.0", "visibility": "public", "source": source}))?;
            cid = prog["cid"].as_str().unwrap_or("").to_string();
            if cid.is_empty() {
                return Err(Fail::new(500, "internal", "the store answered nothing"));
            }
            seconds = Rep(&rep).seconds();
            ends = Rep(&rep).ends();
            lexicon = self.harvest(&text, Some(&rep));
        }
        // RenderInfo: btwserve renders Opus in-process, so the stream is always the format asked for
        let v = if export { crate::export_route::url_version() } else { compose::render_version(&format).to_string() };
        let stream = format!("{}/api/render/{cid}?format={format}&v={v}", self.public);
        let status = format!("{}/api/render/{cid}/status", self.public);
        // RenderInfo checks unbilled (Go: programReport, cached by CID)
        let secs = self.stored(&cid).and_then(|(t, ..)| self.eng.check("", &t).ok()).and_then(|r| Rep(&r).seconds()).unwrap_or(0.0);
        let sv = if secs > 0.0 {
            gojson::num(secs)
        } else {
            seconds.map_or(Value::Null, gojson::num)
        };
        let mut note = "Open stream_url to listen: the audio streams as it renders (first sound within about a second). status_url says how far the render is.".to_string();
        if !ends {
            note += " This program does not end by itself (it loops, or has no timeline), so the render is cut at the server's bar limit; add `end at bar N` for a length of your own.";
        }
        note += " The offline render plays the notes and sounds, not control lines (filters, reverb, do ...), so it can sound plainer than the browser.";
        if export {
            note = match format.as_str() {
                "midi" => "Download stream_url: a Standard MIDI File (type 1), one track per part (drums on channel 10, General MIDI), the tempo map, the key and a marker per section. The notes are the ones the render plays.".into(),
                _ => "GET stream_url: while the stems are being made it answers 202 with {state, file, done} and Retry-After; ask again until it is the zip (a minute or two for a long song). Each stem is its part as it sits in the mix, master effects applied per stem; manifest.json says how closely they add up to the master.".into(),
            };
            if !ends {
                note += " This program does not end by itself, so it is cut at the server's bar limit; add `end at bar N` for a length of your own.";
            }
        }
        Ok(sorted(json!({"cid": cid, "format": format, "stream_url": stream, "status_url": status, "license": "CC0-1.0", "seconds": sv, "lexicon": lexicon, "note": note})))
    }

    fn record_tool(&self, c: &Caller, a: Args) -> R {
        a.only(&["text", "cid", "example"])?;
        let mut ra = Map::new();
        ra.insert("format".into(), json!("wav"));
        for k in ["text", "cid", "example"] {
            if let Some(v) = a.0.get(k) {
                ra.insert(k.into(), v.clone());
            }
        }
        let m = self.render_tool(c, Args(ra))?;
        let cid = m["cid"].as_str().unwrap_or("").to_string();
        if cid.is_empty() {
            return Err(Fail::new(500, "internal", "no CID to record"));
        }
        let p = &self.public;
        Ok(sorted(json!({
            "cid": cid, "seconds": m["seconds"], "license": "CC0-1.0",
            "record_url": format!("{p}/?record=1&cid={cid}"),
            "record_url_manual_clock": format!("{p}/?record=1&clock=manual&cid={cid}"),
            "audio_url": m["stream_url"],
            "recipe": sorted(json!({
                "deterministic": format!("node btwsite/tools/record-demo.mjs --server {p} --btwd <path to btwd> --cid {cid} --out <dir>"),
                "needs": "Node 22+, ffmpeg and Google Chrome on your machine; the script steps the page's fake clock frame by frame, renders the audio with btwd and muxes them",
                "outputs": ["btw-demo.mp4 (45-60 s, H.264 CRF 20, AAC 192k, 1920x1080, faststart)", "btw-hero-loop.mp4 and .webm (10-15 s, silent, the drop)", "btw-poster.png"],
            })),
            "note": "record_url_manual_clock has no audio and no wall clock: it only moves when window.__btw.setTime is called. audio_url is the offline render of the same program (notes and sounds, not control lines such as filter sweeps).",
        })))
    }

    // ---------- list_examples, instruments ----------

    fn list_examples(&self, a: Args) -> R {
        a.only(&["name", "phrasebook", "meter", "syllables", "themes", "key", "role", "composer", "lexicon"])?;
        let name = a.str("name", 64)?;
        let meter = a.str("meter", 64)?;
        let syllables = a.str("syllables", 16)?;
        let want_book = a.boolean("phrasebook")? || meter.is_some() || syllables.is_some();
        let (key, role, composer) = (a.str("key", 16)?, a.str("role", 16)?, a.str("composer", 64)?);
        let want_themes = a.boolean("themes")? || key.is_some() || role.is_some() || composer.is_some();
        let items: Vec<Value> = self
            .app
            .examples
            .list
            .iter()
            .map(|e| {
                let mut v = e.json();
                v["url"] = json!(format!("{}/examples/{}.btw", self.public, e.name));
                v
            })
            .collect();
        let mut out = json!({"examples": if items.is_empty() { Value::Null } else { Value::Array(items) }, "license": "CC0-1.0"});
        if let Some(n) = name.filter(|n| !n.is_empty()) {
            if !name_ok(&n) {
                return Err(bad("argument \"name\" is an example name"));
            }
            let Some(t) = self.example_text(&n) else { return Err(Fail::new(404, "not_found", format!("no example is named {}; the names are in the list", q(&n)))) };
            out["name"] = json!(n);
            out["text"] = json!(t);
        }
        if want_book {
            out["phrasebook"] = btwserve::found::lines(meter.as_deref(), syllables.as_deref()).map_err(|m| bad(&m))?;
        }
        if want_themes {
            out["themes"] = btwserve::themes::list(key.as_deref(), role.as_deref(), composer.as_deref()).map_err(|m| bad(&m))?;
        }
        if a.boolean("lexicon")? {
            out["lexicon"] = json!({
                "words": self.harvest.top(LEXICON_TOP),
                "total": self.harvest.len(),
                "note": "The lexicon harvest (docs/btw/lexicon-harvest.md): pairs collected as a side effect of check, eval, load, listen, render, share, queue, genre_template and generate. Annotated pairs (word=/phonemes/) are strong, rule pairs weak. A person adds words to the CC0 lexicon from the weekly proposal; nothing here changes it.",
            });
        }
        Ok(sorted(out))
    }

    fn instruments(&self, a: Args) -> Result<String, Fail> {
        a.only(&[])?;
        let mut c = self.inst.lock().unwrap();
        if let Some((at, raw)) = c.as_ref() {
            if at.elapsed() < Duration::from_secs(300) {
                return Ok(raw.clone());
            }
        }
        let raw = self.eng.instruments();
        *c = Some((Instant::now(), raw.clone()));
        Ok(raw)
    }

    // ---------- queue ----------

    fn limit_queue(&self, c: &Caller) -> Result<(), Fail> {
        if let Err(r) = self.queue_lim.allow(&c.key) {
            return Err(Fail::from(crate::ApiError::new(429, "rate_limited", "too many queue writes; slow down").retry(r)));
        }
        if let Err(r) = self.ip_ceiling(&self.queue_ip, c) {
            return Err(Fail::from(crate::ApiError::new(429, "rate_limited", "too many queue writes from your address; slow down").retry(r)));
        }
        Ok(())
    }

    fn queue_tool(&self, c: &Caller, a: Args) -> R {
        a.only(&["action", "items", "item", "session", "device"])?;
        let action = a.str("action", 10)?.unwrap_or_default();
        if !matches!(action.as_str(), "list" | "add" | "remove" | "clear" | "skip") {
            return Err(bad("argument \"action\" must be one of list, add, remove, clear, skip"));
        }
        if a.0.contains_key("items") && action != "add" {
            return Err(bad("items is for action add"));
        }
        let item = a.str("item", 100)?;
        if item.is_some() && action != "remove" {
            return Err(bad("item is for action remove"));
        }
        let item = item.unwrap_or_default();
        if action == "remove" && item.trim().is_empty() {
            return Err(bad("remove needs item: an item id from list, or a CID"));
        }
        let session = self.target(c, &a)?;
        if action != "list" {
            self.limit_queue(c)?;
        }
        let mut op = QueueOp { action: action.clone(), items: Vec::new(), remove: item };
        let mut lexicon = btwserve::diagnostics::Recorded::default();
        if action == "add" {
            op.items = self.queue_items(c, &a)?;
            // the lexicon harvest, item by item (a link's text, else the stored program's or the example's)
            for it in &op.items {
                let text = if !it.text.is_empty() {
                    Some(it.text.clone())
                } else if !it.example.is_empty() {
                    self.example_text(&it.example)
                } else {
                    self.stored(&it.cid).map(|(t, ..)| t)
                };
                if let Some(t) = text {
                    let r = self.harvest_raw(&t, None);
                    lexicon.harvested += r.harvested;
                    lexicon.new += r.new;
                }
            }
        }
        let (v, sent) = match self.hub.queue(c.uid.as_deref().unwrap_or(""), &session, op) {
            Err(e) if e.err.code == "no_session" => return Err(self.no_session(&session)),
            Err(e) => return Err(e),
            Ok(x) => x,
        };
        let mut out = json!({"action": action, "queue": v, "lexicon": lexicon.json()});
        match action.as_str() {
            "skip" => {
                out["skipped"] = json!(sent);
                let paused = v["paused_reason"].as_str().unwrap_or("");
                let listener = v["listener"].as_str().unwrap_or("");
                let note = if sent {
                    "The tab was asked to play the next item now. Call get to see it.".to_string()
                } else if v["up_next"].as_array().is_none_or(|u| u.is_empty()) {
                    "The queue is empty: nothing to skip to.".into()
                } else if !paused.is_empty() {
                    format!("Not skipped: the queue is paused ({paused}) until someone interacts with the page. Only a person on the page can resume it.")
                } else if listener != "present" {
                    format!("Not skipped: no listener is present on the tab ({listener}). The queue advances only while someone is listening.")
                } else {
                    "Not skipped: the tab has no live connection.".into()
                };
                out["note"] = json!(note);
            }
            "add" => out["note"] = json!("Queued. The tab plays them in order after the current program ends (call get to see what is playing). Nothing is composed or rendered for a queue."),
            _ => {}
        }
        Ok(sorted(out))
    }

    fn queue_items(&self, c: &Caller, a: &Args) -> Result<Vec<QueueItem>, Fail> {
        let raw = a.0.get("items").and_then(|v| v.as_array()).filter(|r| !r.is_empty()).ok_or_else(|| bad("add needs items: a list of {cid}, {example} or {url} objects"))?;
        let max = self.queue.1;
        if raw.len() > max {
            return Err(Fail::new(413, "too_large", format!("one add takes at most {max} items; this one has {}", raw.len())));
        }
        let mut out = Vec::new();
        for (i, r) in raw.iter().enumerate() {
            let Some(m) = r.as_object() else { return Err(bad(format!("items[{i}] must be an object"))) };
            match self.queue_item(c, &Args(m.clone())) {
                Ok(it) => out.push(it),
                Err(mut e) => {
                    e.err.message = format!("items[{i}]: {}", e.err.message);
                    return Err(e);
                }
            }
        }
        Ok(out)
    }

    fn queue_item(&self, c: &Caller, a: &Args) -> Result<QueueItem, Fail> {
        a.only(&["cid", "example", "url", "bars", "minutes", "title"])?;
        let (cid, example, url) = self.one_program(a)?;
        let bars = a.num("bars", 1.0, 1024.0)?;
        if bars.is_some_and(|b| b != b.trunc()) {
            return Err(bad("argument \"bars\" must be a whole number"));
        }
        let minutes = a.num("minutes", 0.1, 120.0)?;
        if bars.is_some() && minutes.is_some() {
            return Err(bad("give bars or minutes, not both"));
        }
        let mut title = match a.str("title", 320)? {
            Some(t) => one_line(&t, 80).ok_or_else(|| bad("argument \"title\" must be one line of at most 80 characters"))?,
            None => String::new(),
        };
        let mut it = QueueItem { bars: bars.unwrap_or(0.0) as i64, minutes: minutes.unwrap_or(0.0), ..Default::default() };
        if !cid.is_empty() {
            if !is_cid(&cid) {
                return Err(bad("argument \"cid\" is not a CID (it starts with z)"));
            }
            let Some((text, src, meta)) = self.stored(&cid) else {
                return Err(Fail::new(404, "not_found", format!("no stored program has CID {cid}. The queue plays only programs that exist: store one first (render, or share signed in), then queue its CID.")));
            };
            queue_gate(&text, &meta)?;
            it.cid = cid.clone();
            it.source = src.into();
            if title.is_empty() {
                title = meta["name"].as_str().unwrap_or("").into();
            }
        } else if !example.is_empty() {
            if !name_ok(&example) {
                return Err(bad("argument \"example\" is an example name, as list_examples gives"));
            }
            if self.example_text(&example).is_none() {
                return Err(Fail::new(404, "not_found", format!("no example is named {}; call list_examples", q(&example))));
            }
            it.cid = self.example_cid(&example).unwrap_or_default();
            it.source = "example".into();
            it.example = example.clone();
            if title.is_empty() {
                title = self.app.examples.list.iter().find(|e| e.name == example).map(|e| e.title.clone()).unwrap_or_default();
            }
        } else {
            let rep = self.open_report(c, &url)?;
            it.cid = rep["cid"].as_str().unwrap_or("").into();
            it.source = "link".into();
            match self.stored(&it.cid) {
                Some((text, src, meta)) => {
                    queue_gate(&text, &meta)?;
                    it.source = src.into();
                }
                None => {
                    // held in memory for the queue, never written: gated all the same (a link carries no rights,
                    // so it is taken as CC0-1.0, as a stored one would be)
                    queue_gate(rep["canonical"].as_str().unwrap_or(""), &Value::Null)?;
                    it.text = btwserve::autoformat::autoformat(rep["canonical"].as_str().unwrap_or(""), self.max_prog);
                }
            }
        }
        it.title = title;
        Ok(it)
    }
}

enum Text {
    Json(Value),
    Raw(String),
}

fn ok_result(t: Text) -> Value {
    let s = match t {
        Text::Json(v) => gojson::to_string(&v),
        Text::Raw(s) => s,
    };
    json!({"content": [{"type": "text", "text": s}]})
}

/// `errResult`: isError, the site's error body as the text (with "Retry after N seconds." when the words lack it).
fn err_result(mut e: Fail) -> Value {
    if e.err.retry_after > 0 && !e.err.message.to_lowercase().contains("retry") {
        e.err.message += &format!(" Retry after {} seconds.", e.err.retry_after);
    }
    let s = gojson::to_string(&json!({"error": e.json()}));
    json!({"content": [{"type": "text", "text": s}], "isError": true})
}

/// `needsFixing`.
fn needs_fixing(rep: &Value, what: &str) -> Fail {
    let r = Rep(rep);
    let mut e = Fail::new(422, "bad_request", format!("the program has {} error(s), so {what}. Fix each at its line:col (the help line says how; render is the caret form) and call check again.", r.errors()));
    e.details = Some(json!({"diagnostics": r.diagnostics(), "errors": r.errors(), "warnings": r.warnings()}));
    e
}

/// `private`.
/// The storage gate for what the server holds in a queue: an example, or a program the gate admits.
fn queue_gate(text: &str, meta: &Value) -> Result<(), Fail> {
    if meta["source"] == "example" {
        return Ok(());
    }
    btwserve::gate::admit(text, meta["license"].as_str().unwrap_or(""), meta["visibility"].as_str().unwrap_or("")).map(|_| ()).map_err(|r| Fail::new(r.status, r.code, r.message))
}

fn private(rep: &Value, what: &str) -> Option<Fail> {
    let r = Rep(rep);
    if r.restricted() {
        return Some(Fail::new(422, "restricted", format!("the program says `use restricted` or uses a restricted asset, so {what}: private programs never leave the machine they were written on.")));
    }
    if r.uses_assets() {
        return Some(Fail::new(422, "restricted", format!("the program uses `sample` or `loop` assets, which this site has no files for, so {what}. Use the instruments tool's voices instead.")));
    }
    None
}

/// An answer that carries a program (`text`, its `cid`) gains what the recipe makes, as check says it (btwapi::resolution):
/// `expanded_cid`, and `refs` and `resolution` when the program has references. genre_template and generate add these.
/// `expand` with `eject` ("line:col", 1-based): the program with the one reference there written out in place under its
/// `# from: <call> -> <cid>` line (djlive::refs::eject), the rest as they are. Answers {ok, text, cid, ejected: {call, kind,
/// half, place, name, resolved, from}}; the text plays as the program did.
fn eject_at(text: &str, at: &str) -> Result<Value, Fail> {
    let (l, c) = at.split_once(':').and_then(|(l, c)| Some((l.parse::<usize>().ok()?, c.parse::<usize>().ok()?))).ok_or_else(|| bad("eject is line:col, 1-based: 5:19"))?;
    let norm = djlive::lex::normalise_input(text).to_string();
    let line_start = norm.split_inclusive('\n').take(l.saturating_sub(1)).map(str::len).sum::<usize>();
    let byte = (line_start + c.saturating_sub(1)).min(norm.len());
    let env = btwapi::web_env().env();
    let site = match djlive::refs::site_at(&norm, &env, byte) {
        None => return Err(bad(format!("nothing at {at} is a reference (a gen@N call, a seed(...), a <cid>.drop or a template(...))"))),
        Some(Err(ds)) => return Err(bad(ds.first().map_or("the references do not check".to_string(), |d| d.message.clone()))),
        Some(Ok(s)) => s,
    };
    let out = djlive::refs::eject_site(&norm, &site);
    let rep: Value = serde_json::from_str(&btwapi::check_json(&out)).unwrap_or(Value::Null);
    Ok(json!({
        "ok": rep["ok"],
        "text": out,
        "cid": rep["cid"],
        "ejected": {"call": site.call, "kind": site.kind, "half": site.half, "place": site.place, "name": site.name, "resolved": site.resolved, "from": djlive::refs::from_line(&site)},
        "diagnostics": rep["diagnostics"],
    }))
}

pub fn with_references(mut v: Value) -> Value {
    let (Some(t), Some(cid)) = (v["text"].as_str().map(str::to_string), v["cid"].as_str().filter(|c| !c.is_empty()).map(str::to_string)) else { return v };
    for (k, x) in btwapi::resolution::fields(&t, &cid, false) {
        v[k] = serde_json::from_str(&x.js_stringified()).unwrap_or(Value::Null);
    }
    v
}

/// `warningsOf`: null when there are none (a nil slice).
fn warnings_of(rep: &Value) -> Value {
    let w: Vec<Value> = Rep(rep).diagnostics().into_iter().filter(|d| d["severity"] == "warning").collect();
    if w.is_empty() {
        Value::Null
    } else {
        Value::Array(w)
    }
}

#[cfg(test)]
mod exposure;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_cids() {
        let c = "z4EBG9jAWvWyiVNF43gMjU5wYinuLmnAfWLhAVK41gqupdYGvVo";
        assert_eq!(bare_cid(c).as_deref(), Some(c));
        assert_eq!(bare_cid(&format!("#{c}")).as_deref(), Some(c));
        assert_eq!(bare_cid(&format!("https://beatscript.app/p/{c}")).as_deref(), Some(c));
        assert_eq!(bare_cid(&format!("https://beatscript.app/c/{c}/")).as_deref(), Some(c));
        assert_eq!(bare_cid(&format!("https://beatscript.app/#{c}?x=1")).as_deref(), Some(c));
        assert_eq!(bare_cid(&format!("https://beatscript.app/#{c}.abc")), None);
        assert_eq!(bare_cid(&format!("https://beatscript.app/c/{c}.abc")), None);
    }

    /// btwsite/testdata/mcp/tools-list.json (Go's mcpsrv test fixture, public origin https://btw.bitwrap.io): the
    /// same tools, descriptions and schemas. The fixture is re-indented with sorted keys; the wire order is checked
    /// by the contract test.
    #[test]
    fn tools_list_matches_go_fixture() {
        let got = json!({"tools": tool_list("https://btw.bitwrap.io", 16384, 60, 30, (50, 20, 30, 8, 30))});
        // BTW_UPDATE_TOOLS_LIST=1: rewrite the fixture from the list (a tool added or reworded on purpose: listen, get's mixer)
        if std::env::var_os("BTW_UPDATE_TOOLS_LIST").is_some() {
            let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/mcp/testdata/tools-list.json");
            std::fs::write(path, serde_json::to_string_pretty(&gojson::sorted_deep(got.clone())).unwrap() + "\n").unwrap();
        }
        let want: Value = serde_json::from_str(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/mcp/testdata/tools-list.json")).unwrap()).unwrap();
        let (w, g) = (gojson::sorted_deep(want), gojson::sorted_deep(got));
        for (a, b) in w["tools"].as_array().unwrap().iter().zip(g["tools"].as_array().unwrap()) {
            assert_eq!(a, b, "tool {}", a["name"]);
        }
        assert_eq!(w, g);
        let names: Vec<&str> = g["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        let mut want_names = TOOL_NAMES.to_vec();
        want_names.sort();
        assert_eq!(names, want_names);
    }

    /// A hub that takes every push (a tab is always there), for the tools that need one.
    pub(super) struct TakesAll;

    impl HubApi for TakesAll {
        fn push(&self, _: &str, _: &str, _: SessionEvent) -> Result<usize, Fail> {
            Ok(1)
        }
        fn state<'a>(&'a self, _: &'a str, _: &'a str) -> super::super::hubapi::BoxFut<'a, Result<Value, Fail>> {
            Box::pin(async { Ok(json!({})) })
        }
        fn sessions(&self, _: &str) -> Vec<super::super::hubapi::SessionInfo> {
            Vec::new()
        }
        fn devices(&self, _: &str) -> Vec<Value> {
            Vec::new()
        }
        fn device_session(&self, _: &str, _: &str) -> Result<String, Fail> {
            Ok(String::new())
        }
        fn queue(&self, _: &str, _: &str, _: QueueOp) -> Result<(Value, bool), Fail> {
            Ok((json!({"up_next": []}), false))
        }
        fn push_private(&self, _: &str, _: &str, _: super::super::hubapi::PrivatePush, _: usize) -> Result<Value, Fail> {
            Ok(json!({"sent": true}))
        }
    }

    fn answer(t: &Tools, name: &str, args: Value) -> Value {
        let c = Caller { uid: Some("u-test".into()), ip: "127.0.0.1".into(), key: "u:u-test".into() };
        let Value::Object(m) = args else { panic!("args") };
        match t.dispatch(name, &c, Args(m)) {
            Ok(Text::Json(v)) => v,
            Ok(Text::Raw(s)) => serde_json::from_str(&s).unwrap(),
            Err(e) => panic!("{name}: {}", e.err.message),
        }
    }

    /// 6666: every tool that checks a program records its lyric pairs as a side effect, with no flag, and says so in its
    /// answer's `lexicon`; push_private records nothing, nor does a program that may not be kept.
    #[test]
    fn each_checking_tool_harvests_the_lyric_and_says_so_and_push_private_records_nothing() {
        let dir = std::env::temp_dir().join(format!("btw-tools-harvest-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let app = crate::test_app(&dir);
        crate::ontology::install_genres();
        let mut t = Tools::new(&app);
        t.set_hub(Arc::new(TakesAll));
        let prog = "tempo 120\nkey am\ndeck B: melody c4 e4 g4 e4 | harmony chord(am)\ndeck B: sing \"zorblax=/z oh r b l ae k s/ on the road\" on melody\n";
        let lex = |v: &Value| (v["lexicon"]["harvested"].as_u64(), v["lexicon"]["new"].as_u64());
        // the first sight of the annotated pair is new; the same program again is not
        let v = answer(&t, "check", json!({"text": prog}));
        assert_eq!(v["ok"], true, "{v}");
        assert_eq!(lex(&v), (Some(1), Some(1)), "{v}");
        assert!(!v["diagnostics"].as_array().unwrap().iter().any(|d| d["message"].as_str().unwrap_or("").contains("zorblax")), "an annotated word is not a lexicon miss: {v}");
        let cid = v["cid"].as_str().unwrap().to_string();
        let render = answer(&t, "render", json!({"text": prog}));
        assert_eq!(render["cid"], json!(cid));
        for (name, args) in [
            ("eval", json!({"text": prog})),
            ("load", json!({"cid": cid})),
            ("listen", json!({"text": prog, "bars": 2})),
            ("render", json!({"cid": cid})),
            ("share", json!({"text": prog})),
            ("queue", json!({"action": "add", "items": [{"cid": cid}]})),
        ] {
            let v = answer(&t, name, args);
            assert_eq!(lex(&v), (Some(1), Some(0)), "{name}: {v}");
        }
        assert_eq!(lex(&render), (Some(1), Some(0)));
        // a queue action that adds nothing harvests nothing, and says so
        assert_eq!(lex(&answer(&t, "queue", json!({"action": "list"}))), (Some(0), Some(0)));
        // generate's tracks have no lyrics; genre_template with words sings them (a word no lexicon knows: weak)
        assert_eq!(lex(&answer(&t, "generate", json!({"genre": "techno", "seed": 3}))), (Some(0), Some(0)));
        let g = answer(&t, "genre_template", json!({"genre": "techno", "phrase": "quexlo night"}));
        assert_eq!(lex(&g), (Some(1), Some(1)), "{g}");
        // what was harvested: the pair and the programs' CIDs, never their text
        let file = std::fs::read_to_string(dir.join("diagnostics").join(btwserve::diagnostics::FILE)).unwrap();
        assert!(file.contains("\"word\":\"zorblax\"") && file.contains(&cid), "{file}");
        assert!(!file.contains("on the road") && !file.contains("melody"), "no program text: {file}");
        // push_private, and programs that may not be kept, record nothing
        let before = t.harvest.top(0);
        let private = prog.replace("zorblax", "privword");
        let v = answer(&t, "push_private", json!({"device": "phone-1", "text": private}));
        assert!(v.get("lexicon").is_none(), "{v}");
        for p in [format!("#! visibility: private\n{private}"), format!("use restricted\n{private}")] {
            let v = answer(&t, "check", json!({"text": p}));
            assert_eq!(lex(&v), (Some(0), Some(0)), "{p}: {v}");
        }
        assert_eq!(t.harvest.top(0), before);
        // the review, folded into list_examples: annotated words first
        let l = answer(&t, "list_examples", json!({"lexicon": true}));
        assert_eq!(l["lexicon"]["words"][0]["word"], "zorblax", "{l}");
        assert_eq!(l["lexicon"]["words"][0]["variants"][0]["source"], "annotated");
        assert_eq!(l["lexicon"]["words"][0]["variants"][0]["cids"][0], json!(cid));
        assert!(l["lexicon"]["words"].as_array().unwrap().iter().any(|w| w["word"] == "quexlo"));
        assert!(answer(&t, "list_examples", json!({})).get("lexicon").is_none());
        assert!(!std::fs::read_to_string(dir.join("diagnostics").join(btwserve::diagnostics::FILE)).unwrap().contains("privword"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The lexicon harvest is described once (text::LEXICON_DESC), on exactly the tools that do it, and the docs say it in
    /// the same words.
    #[test]
    fn the_harvest_is_described_once_on_the_tools_that_do_it_and_in_the_docs() {
        for t in tool_list("https://beatscript.app", 16384, 60, 30, (50, 20, 30, 8, 30)) {
            let n = t["name"].as_str().unwrap();
            let d = t["description"].as_str().unwrap();
            assert_eq!(d.ends_with(text::LEXICON_DESC), HARVEST_TOOLS.contains(&n), "{n}");
            assert_eq!(d.matches(text::LEXICON_DESC).count(), usize::from(HARVEST_TOOLS.contains(&n)), "{n}");
        }
        let doc = include_str!("../../../../docs/btw/lexicon-harvest.md");
        assert!(doc.contains(text::LEXICON_DESC), "docs/btw/lexicon-harvest.md does not quote text::LEXICON_DESC");
    }

    #[test]
    fn base64_offsets_as_go() {
        for (s, want) in [("abc", Err(0)), ("YQ", Err(0)), ("abcdYQ", Err(4)), ("YQ=x", Err(2)), ("Y===", Err(1)), ("abcd!", Err(4)), ("!!", Err(0))] {
            assert_eq!(go_base64(s.as_bytes()).map(|_| ()), want.map(|_: ()| ()), "{s}");
        }
        assert_eq!(go_base64(b"Y2hlY2s=").unwrap(), b"check");
    }
}

/// At most this many seeds in one `generate` call (`seeds`).
pub const MAX_SEEDS: usize = 16;

/// What a `generate` call asks for: the request, the generator's version (`gen@N`), and optionally one part (and the section
/// it is taken from).
#[derive(Clone, Debug)]
pub struct GenAsk {
    pub req: btwapi::GenRequest,
    pub version: u32,
    pub part: Option<String>,
    pub section: Option<String>,
}

impl GenAsk {
    pub fn new(req: btwapi::GenRequest) -> GenAsk {
        GenAsk { req, version: btwapi::generate::GENERATOR_VERSION, part: None, section: None }
    }
}

/// The `generator` argument: a version of the seeded generator (1 or 2, as `gen@1` and `gen@2` name them; `seeded@2` and
/// `gen@2` are read too); absent, the current one.
pub fn gen_version(v: Option<&Value>) -> Result<u32, Fail> {
    let versions = btwapi::generate::VERSIONS;
    let n = match v {
        None => return Ok(btwapi::generate::GENERATOR_VERSION),
        Some(Value::Number(n)) => n.as_u64(),
        Some(Value::String(s)) => s.trim().trim_start_matches("seeded@").trim_start_matches("gen@").parse::<u64>().ok(),
        Some(_) => None,
    };
    match n.and_then(|n| u32::try_from(n).ok()).filter(|n| versions.contains(n)) {
        Some(n) => Ok(n),
        None => Err(bad(format!("argument \"generator\" is a version of the seeded generator: {}", versions.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(" or ")))),
    }
}

/// A part of the program as the answer gives it: role, section, sections, voice or kit, text, gen, and with its let, `let`
/// (or `let_error`).
fn part_json(p: &btwapi::generate::GenPart, with_let: bool) -> Value {
    let mut o = json!({"role": p.role, "section": p.section, "sections": p.sections, "text": p.text, "gen": p.gen});
    if let Some(v) = &p.voice {
        o["voice"] = json!(v);
    }
    if let Some(k) = &p.kit {
        o["kit"] = json!(k);
    }
    if with_let {
        o["let"] = p.let_text.as_ref().map_or(Value::Null, |t| json!(t));
        if let Some(e) = &p.let_error {
            o["let_error"] = json!(e);
        }
    }
    o
}

/// Store a generated program as generate always has (public, CC0, source generate, its generator and seed in the meta): its
/// url and render_url, or why not.
fn store_generated(app: &AppState, eng: &Engine, public: &str, ip: &str, g: &btwapi::Generated, gen: &str) -> Result<(String, String, Value), String> {
    let meta = json!({"license": "CC0-1.0", "visibility": "public", "source": "generate", "generator": gen, "seed": g.seed, "genre": g.genre, "name": g.name});
    match compose::put(app, eng, ip, &g.text, meta) {
        Ok((p, rep)) => {
            let cid = p["cid"].as_str().unwrap_or("").to_string();
            Ok((format!("{public}/p/{cid}"), format!("{public}/api/render/{cid}?format=opus&v={}", compose::render_version("opus")), rep))
        }
        Err(e) => Err(e.err.message),
    }
}

/// Generate, check and store one seeded program: the shared body of the MCP tool and `POST /api/generate`.
/// A program that does not check (a generator bug) is answered with its diagnostics and `stored: false`. The answer gains `gen`
/// (the call that is this program), `parts` (each part's role, section, voice or kit, text, its own `gen` call and the let
/// that call writes; with `part`, that part alone, and its let as `let`) and `expanded_cid` (with_references).
pub fn generate_and_store(app: &AppState, eng: &Engine, public: &str, ip: &str, ask: &GenAsk) -> R {
    let req = &ask.req;
    let g = btwapi::generate::generate_version(ask.version, req).map_err(bad)?;
    let rep = eng.check(ip, &g.text)?;
    let gen = format!("{}@{}", btwapi::generate::GENERATOR, ask.version);
    let mut out = json!({
        "ok": Rep(&rep).ok(), "text": g.text, "cid": Rep(&rep).cid(), "name": g.name, "generator": gen,
        "genre": g.genre, "seed": g.seed, "bpm": g.bpm, "key": g.key, "length": g.length,
        "license": "CC0-1.0", "stored": false,
    });
    if !Rep(&rep).ok() {
        out["check"] = rep;
        return Ok(out);
    }
    let call = btwapi::generate::gen_call(ask.version, req, &g, Rep(&rep).cid());
    out["gen"] = call.as_ref().map_or(Value::Null, |c| json!(c));
    match &call {
        Some(call) => match btwapi::generate::parts_of(&g.text, call, true, ask.part.as_deref(), ask.section.as_deref()) {
            Ok(parts) => {
                if ask.part.is_some() {
                    if let Some(p) = parts.first() {
                        out["part"] = part_json(p, true);
                        out["let"] = p.let_text.as_ref().map_or(Value::Null, |t| json!(t));
                    }
                }
                out["parts"] = Value::Array(parts.iter().map(|p| part_json(p, true)).collect());
            }
            Err(e) if ask.part.is_some() || ask.section.is_some() => return Err(bad(e)),
            Err(e) => out["parts_error"] = json!(e),
        },
        None => out["parts_error"] = json!("this seed cannot be written as a gen call (a seed is a whole number or one word): the parts have no call"),
    }
    match store_generated(app, eng, public, ip, &g, &gen) {
        Ok((url, render_url, _)) => {
            out["stored"] = json!(true);
            out["render_url"] = json!(render_url);
            out["url"] = json!(url);
        }
        Err(e) => out["store_error"] = json!(e),
    }
    out["note"] = json!("A whole program from the seed: edit it freely (lets, defs, sections, arrange). The same inputs always give this text and CID. Play it with eval or load. Each part's gen is the call that is just that part: put it in a let (its let is what that writes) or on a part line of your own program, where it is fitted to your key and tempo unless it says bpm= and key=.");
    Ok(with_references(out))
}

/// One seed of a `generate` with `seeds`: a compact summary (seed, generator, cid, name, key, tempo, bars, the parts and their
/// voices, url, render_url), the program stored as generate stores it. A failure is the seed's own entry (`error`), so one bad
/// seed does not lose the others.
pub fn generate_summary(app: &AppState, eng: &Engine, public: &str, ip: &str, ask: &GenAsk) -> Value {
    let gen = format!("{}@{}", btwapi::generate::GENERATOR, ask.version);
    let g = match btwapi::generate::generate_version(ask.version, &ask.req) {
        Ok(g) => g,
        Err(e) => return json!({"seed": ask.req.seed, "generator": gen, "ok": false, "error": e}),
    };
    let mut out = json!({"seed": g.seed, "generator": gen, "name": g.name, "key": g.key, "tempo": g.bpm, "stored": false});
    // the check is the store's (one check a seed); a program the store refuses is checked here to say why
    let rep = match store_generated(app, eng, public, ip, &g, &gen) {
        Ok((url, render_url, rep)) => {
            out["url"] = json!(url);
            out["render_url"] = json!(render_url);
            out["stored"] = json!(true);
            rep
        }
        Err(e) => {
            out["store_error"] = json!(e);
            match eng.check(ip, &g.text) {
                Ok(rep) => rep,
                Err(e) => {
                    out["ok"] = json!(false);
                    out["error"] = json!(e.message);
                    return out;
                }
            }
        }
    };
    if !Rep(&rep).ok() {
        out["ok"] = json!(false);
        out["error"] = json!("the generated program does not check");
        return out;
    }
    let cid = Rep(&rep).cid().to_string();
    out["ok"] = json!(true);
    out["cid"] = json!(cid);
    out["bars"] = rep["song_bars"].clone();
    let call = btwapi::generate::gen_call(ask.version, &ask.req, &g, &cid);
    out["gen"] = call.as_ref().map_or(Value::Null, |c| json!(c));
    if let Some(call) = &call {
        if let Ok(parts) = btwapi::generate::parts_of(&g.text, call, false, ask.part.as_deref(), ask.section.as_deref()) {
            out["parts"] = Value::Array(parts.iter().map(|p| part_json(p, false)).collect());
        }
    }
    out
}
