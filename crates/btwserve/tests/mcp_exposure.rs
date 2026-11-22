//! Every feature is reachable and described through MCP (project-datum 6679): a table, one row a feature. Each row names the
//! tool, the words its description must say (one source with the docs: the same words are in docs/BTW.md, and the row says
//! which), the input it needs, and one real `tools/call` against the real `btwserve` binary whose answer must carry the
//! feature's fields. The table is held to the docs: every engine call docs/BTW.md "Two halves" says is coming, and every field
//! "What a reference resolves to" names, must be some row's (a feature documented with no row fails the test).
//!
//! A row marked `pending` is a feature another branch is building: it is expected to fail until it lands, and the test fails
//! when it passes (drop the mark then, so the row guards it from there on).
//!
//! The same assertions, live, for a deploy script (one line; `jq -e` exits non-zero when one is missing):
//!
//! ```text
//! curl -s https://beatscript.app/mcp -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"check","arguments":{"text":"tempo 124\nkey am\nsection drop 4 bars {\n  deck A: kick x ~ x ~\n  deck B: bass gen@2 techno seed=42 section=drop\n}\narrange drop\n","expand":true}}}' | jq -e '.result.content[0].text | fromjson | .ok and (.refs[0].kind == "gen") and (.refs[0].resolved | startswith("z")) and (.refs[0].fitted == ["key","bpm"]) and (.refs[0].expansion | length > 0) and (.expanded_cid != .cid) and (.resolution[0].generator == "seeded@2")'
//! ```
//!
//! `cargo test -p btwserve --test mcp_exposure`.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// A program with a gen call on a part line, in a key and at a tempo of its own (so the call is fitted to both).
const GEN_PROG: &str = "tempo 124\nkey am\nsection drop 4 bars {\n  deck A: kick x ~ x ~\n  deck B: bass gen@2 techno seed=42 section=drop\n}\narrange drop\n";
/// A program with no reference.
const PLAIN: &str = "tempo 120\ndeck A: kick x ~ x ~\n";
/// The docs the descriptions say the same thing as.
const BTW_MD: &str = include_str!("../../../docs/BTW.md");

struct Server {
    child: Child,
    addr: String,
    _dir: tempfile::TempDir,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start() -> Server {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    std::fs::write(d.join("session.secret"), hex::encode([42u8; 32]) + "\n").unwrap();
    std::fs::write(d.join("site.json"), json!({"site_id": "siteSITEsiteSITEsite01", "created": "2026-11-19T00:00:00Z"}).to_string()).unwrap();
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let addr = format!("127.0.0.1:{port}");
    let child = Command::new(env!("CARGO_BIN_EXE_btwserve"))
        .args(["-addr", &addr, "-data", &d.to_string_lossy(), "-log-dir", &d.join("log-rs").to_string_lossy(), "-public-url", "https://beatscript.app", "-web", "", "-docs", ""])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let t0 = Instant::now();
    while TcpStream::connect(&addr).is_err() {
        assert!(t0.elapsed() < Duration::from_secs(60), "btwserve did not start");
        std::thread::sleep(Duration::from_millis(50));
    }
    Server { child, addr, _dir: dir }
}

fn dechunk(b: &str) -> String {
    let mut out = String::new();
    let mut rest = b;
    while let Some((len, tail)) = rest.split_once("\r\n") {
        let Ok(n) = usize::from_str_radix(len.trim(), 16) else { return b.to_string() };
        if n == 0 || tail.len() < n {
            break;
        }
        out.push_str(&tail[..n]);
        rest = tail[n..].trim_start_matches("\r\n");
    }
    out
}

impl Server {
    /// One JSON-RPC request to `/mcp`: its `result`.
    fn rpc(&self, method: &str, params: Value) -> Value {
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string();
        let mut s = TcpStream::connect(&self.addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(300))).unwrap();
        s.write_all(format!("POST /mcp HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", self.addr, body.len()).as_bytes()).unwrap();
        let mut raw = Vec::new();
        let _ = s.read_to_end(&mut raw);
        let raw = String::from_utf8_lossy(&raw).to_string();
        let (head, b) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
        let b = if head.to_ascii_lowercase().contains("transfer-encoding: chunked") { dechunk(b) } else { b.to_string() };
        let v: Value = serde_json::from_str(b.trim()).unwrap_or_else(|e| panic!("{method}: not JSON ({e}): {head}\n{b}"));
        v["result"].clone()
    }

    /// `tools/call`: the tool's answer, as the JSON its text is (an error answer is `{"error": ...}`).
    fn call(&self, tool: &str, args: Value) -> Value {
        let r = self.rpc("tools/call", json!({"name": tool, "arguments": args}));
        let text = r["content"][0]["text"].as_str().unwrap_or("null");
        serde_json::from_str(text).unwrap_or(Value::Null)
    }
}

/// Whitespace runs as one space: the docs wrap their lines, a description does not.
fn flat(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

type Check = fn(&Server) -> Result<(), String>;

struct Row {
    /// The feature, and where it was asked for (project-datum).
    feature: &'static str,
    /// The words of the docs it stands for (see [`documented`]).
    covers: &'static [&'static str],
    tool: &'static str,
    /// What the tool's description must say, word for word.
    desc: &'static [&'static str],
    /// What docs/BTW.md must say, word for word (whitespace aside): the same words, where a row has them.
    doc: &'static [&'static str],
    /// The inputs the tool's schema must take.
    inputs: &'static [&'static str],
    /// The real call and what its answer must carry.
    call: Check,
    /// Another branch's feature, expected to fail until it lands: why.
    pending: Option<&'static str>,
}

fn ensure(cond: bool, what: impl Into<String>) -> Result<(), String> {
    if cond {
        Ok(())
    } else {
        Err(what.into())
    }
}

fn is_cid(v: &Value) -> bool {
    v.as_str().is_some_and(|s| s.starts_with('z') && s.len() > 20)
}

// ---------------------------------------------------------------- the calls

fn check_gen_refs(s: &Server) -> Result<(), String> {
    let r = s.call("check", json!({"text": GEN_PROG}));
    ensure(r["ok"] == true, format!("the gen program does not check: {r}"))?;
    let refs = r["refs"].as_array().ok_or(format!("no refs: {r}"))?;
    let g = refs.iter().find(|x| x["kind"] == "gen").ok_or("no gen ref")?;
    ensure(g["call"] == "gen@2 techno seed=42 section=drop", format!("call: {g}"))?;
    ensure(g["half"] == "engine" && g["place"] == "part" && g["name"] == "bass", format!("half, place, name: {g}"))?;
    ensure(is_cid(&g["resolved"]) && g["generator"] == "seeded@2", format!("resolved, generator: {g}"))?;
    ensure(g["fitted"] == json!(["key", "bpm"]), format!("fitted: {g}"))?;
    ensure(g["line"] == 5 && g["col"].as_i64().is_some(), format!("line, col: {g}"))?;
    ensure(g.get("expansion").is_none(), "an expansion without expand")?;
    // the provenance: an info at the call with the CID it resolved to and what it was fitted to
    let note = r["diagnostics"].as_array().into_iter().flatten().find(|d| d["severity"] == "info" && d["message"].as_str().is_some_and(|m| m.contains("seeded@2") && m.contains(g["resolved"].as_str().unwrap_or("?"))));
    ensure(note.is_some_and(|d| d["message"].as_str().unwrap_or("").contains("generated in the program's key am and at its tempo 124")), format!("no fitted note: {}", r["diagnostics"]))
}

fn check_expand(s: &Server) -> Result<(), String> {
    let r = s.call("check", json!({"text": GEN_PROG, "expand": true}));
    let g = r["refs"].as_array().and_then(|a| a.iter().find(|x| x["kind"] == "gen").cloned()).ok_or(format!("no gen ref: {r}"))?;
    ensure(g["expansion"].as_str().is_some_and(|e| !e.is_empty() && !e.contains("gen@")), format!("expansion: {g}"))
}

fn check_expanded_cid(s: &Server) -> Result<(), String> {
    let plain = s.call("check", json!({"text": PLAIN}));
    ensure(is_cid(&plain["expanded_cid"]) && plain["expanded_cid"] == plain["cid"], format!("a program with no references: expanded_cid is cid: {plain}"))?;
    ensure(plain.get("refs").is_none() && plain.get("resolution").is_none(), "refs or resolution on a program with no references")?;
    let r = s.call("check", json!({"text": GEN_PROG}));
    ensure(is_cid(&r["expanded_cid"]) && r["expanded_cid"] != r["cid"], format!("expanded_cid: {r}"))?;
    // the same recipe, the same result
    let again = s.call("check", json!({"text": GEN_PROG}));
    ensure(again["expanded_cid"] == r["expanded_cid"], "the same recipe gave another expanded_cid")?;
    // and it is the CID of the program expand writes out
    let e = s.call("expand", json!({"text": GEN_PROG}));
    ensure(e["cid"] == r["expanded_cid"], format!("expand's cid {} is not check's expanded_cid {}", e["cid"], r["expanded_cid"]))
}

fn check_resolution(s: &Server) -> Result<(), String> {
    let r = s.call("check", json!({"text": GEN_PROG}));
    let res = r["resolution"].as_array().ok_or(format!("no resolution: {r}"))?;
    let g = res.iter().find(|x| x["kind"] == "gen").ok_or("no gen entry")?;
    ensure(g["cid"] == r["refs"][0]["resolved"] && g["generator"] == "seeded@2" && g["nested"] == false, format!("{g}"))?;
    ensure(g["key"] == "am" && g["bpm"] == 124.0, format!("key and bpm it was generated in: {g}"))
}

fn listen_from(s: &Server) -> Result<(), String> {
    let r = s.call("listen", json!({"text": GEN_PROG, "bars": 4}));
    ensure(r.get("error").is_none(), format!("listen failed: {r}"))?;
    let parts = r["parts"].as_array().ok_or(format!("no parts: {r}"))?;
    let bass = parts.iter().find(|p| p["deck"] == "B" && p["part"] == "bass").ok_or(format!("no bass part: {parts:?}"))?;
    ensure(bass["from"]["call"] == "gen@2 techno seed=42 section=drop" && bass["from"]["kind"] == "gen" && is_cid(&bass["from"]["resolved"]), format!("from: {bass}"))?;
    let kick = parts.iter().find(|p| p["deck"] == "A").ok_or("no kick")?;
    ensure(kick.get("from").is_none(), format!("a part typed by hand says it came from a reference: {kick}"))?;
    ensure(r["notes"].as_array().is_some_and(|n| n.iter().any(|x| x.as_str().is_some_and(|t| t.contains("generated in the program's key am")))), format!("no fitted note: {}", r["notes"]))?;
    ensure(is_cid(&r["expanded_cid"]) && r["resolution"].as_array().is_some_and(|a| !a.is_empty()), format!("expanded_cid, resolution: {r}"))
}

fn generate_generator(s: &Server) -> Result<(), String> {
    let r = s.call("generate", json!({"genre": "techno", "seed": 42, "generator": 1}));
    ensure(r["generator"] == "seeded@1" && r["gen"] == "gen@1 techno seed=42", format!("{}", json!({"generator": r["generator"], "gen": r["gen"]})))?;
    let now = s.call("generate", json!({"genre": "techno", "seed": 42}));
    ensure(now["generator"] == "seeded@2" && now["gen"] == "gen@2 techno seed=42" && now["cid"] != r["cid"], "the default is seeded@2, another program")?;
    // the call is the program: checking `arrange from <gen>` resolves to its CID
    let c = s.call("check", json!({"text": format!("arrange from {}\n", now["gen"].as_str().unwrap_or(""))}));
    ensure(c["refs"][0]["resolved"] == now["cid"], format!("arrange from {} resolves to {}, not {}", now["gen"], c["refs"][0]["resolved"], now["cid"]))
}

fn generate_part(s: &Server) -> Result<(), String> {
    let r = s.call("generate", json!({"genre": "techno", "seed": 42, "part": "bass"}));
    let p = &r["part"];
    ensure(p["role"] == "bass" && p["gen"] == "gen@2 techno seed=42 section=drop part=bass", format!("part: {p}"))?;
    let l = r["let"].as_str().ok_or(format!("no let: {r}"))?;
    ensure(l.contains("let drop_bass_1 = ") && l.contains("let drop_bass = <"), format!("let: {l}"))?;
    ensure(r["parts"].as_array().is_some_and(|a| a.len() == 1), "with part, parts is that part alone")?;
    // the let is what the call writes: a program with the call and one with the let play the same
    let a = s.call("check", json!({"text": format!("let drop_bass = {}\ndeck B: bass drop_bass\n", p["gen"].as_str().unwrap_or(""))}));
    let b = s.call("check", json!({"text": format!("{l}\ndeck B: bass drop_bass\n")}));
    ensure(a["ok"] == true && a["expanded_cid"] == b["cid"], format!("the let {} is not what the call writes ({})", b["cid"], a["expanded_cid"]))
}

fn generate_parts(s: &Server) -> Result<(), String> {
    let r = s.call("generate", json!({"genre": "techno", "seed": 42}));
    let parts = r["parts"].as_array().ok_or(format!("no parts: {r}"))?;
    ensure(parts.len() >= 3, format!("parts: {parts:?}"))?;
    for p in parts {
        let role = p["role"].as_str().unwrap_or("?");
        ensure(p["gen"].as_str().is_some_and(|g| g.starts_with("gen@2 techno seed=42 section=") && g.ends_with(&format!(" part={role}"))), format!("gen: {p}"))?;
        ensure(p.get("voice").is_some() || p.get("kit").is_some() || matches!(role, "snare" | "hihat" | "clap"), format!("voice or kit: {p}"))?;
        ensure(p["let"].is_string() || p["let_error"].is_string(), format!("let: {p}"))?;
        ensure(p["text"].as_str().is_some_and(|t| !t.is_empty()), format!("text: {p}"))?;
    }
    ensure(r["expanded_cid"] == r["cid"], "a generated program has no references: expanded_cid is cid")
}

fn generate_seeds(s: &Server) -> Result<(), String> {
    let r = s.call("generate", json!({"genre": "house", "seeds": {"count": 2, "start": 7}}));
    let ps = r["programs"].as_array().ok_or(format!("no programs: {r}"))?;
    ensure(ps.len() == 2 && ps[0]["seed"] == "7" && ps[1]["seed"] == "8", format!("seeds: {r}"))?;
    for p in ps {
        for k in ["generator", "cid", "name", "key", "tempo", "bars", "gen", "parts", "url", "render_url"] {
            ensure(!p[k].is_null(), format!("summary without {k}: {p}"))?;
        }
        ensure(p["parts"].as_array().is_some_and(|a| a.iter().all(|x| x["gen"].is_string() && x.get("let").is_none())), "a summary's parts carry their gen, not their let")?;
    }
    // the same seed, the same program, as one generate gives it
    let one = s.call("generate", json!({"genre": "house", "seed": 7}));
    ensure(one["cid"] == ps[0]["cid"], "seeds[7] is not generate seed=7")?;
    let list = s.call("generate", json!({"genre": "house", "seeds": [8]}));
    ensure(list["programs"][0]["cid"] == ps[1]["cid"], "a list of seeds is not the count from start")?;
    let over = s.call("generate", json!({"genre": "house", "seeds": {"count": 17}}));
    ensure(over.get("error").is_some(), "more than 16 seeds is refused")
}

fn expand_gen(s: &Server) -> Result<(), String> {
    let r = s.call("expand", json!({"text": GEN_PROG}));
    ensure(r["ok"] == true, format!("{r}"))?;
    let t = r["text"].as_str().unwrap_or("");
    ensure(!t.contains("gen@") && t.contains("deck B: bass "), format!("text: {t}"))?;
    ensure(r["refs"].as_array().is_some_and(|a| a.iter().any(|x| x.as_str().is_some_and(|c| c.starts_with("gen@2 techno seed=42")))), format!("refs: {}", r["refs"]))
}

fn genre_template_refs(s: &Server) -> Result<(), String> {
    let r = s.call("genre_template", json!({"genre": "techno"}));
    ensure(is_cid(&r["cid"]) && r["expanded_cid"] == r["cid"], format!("a template with no references: expanded_cid is cid: {}", json!({"cid": r["cid"], "expanded_cid": r["expanded_cid"], "error": r["error"]})))?;
    let f = s.call("genre_template", json!({"genre": "techno", "form": "rondo"}));
    ensure(is_cid(&f["expanded_cid"]) && f["expanded_cid"] != f["cid"] && f["resolution"].as_array().is_some_and(|a| a.iter().any(|x| x["kind"] == "template")), format!("a form: {}", json!({"cid": f["cid"], "expanded_cid": f["expanded_cid"], "resolution": f["resolution"], "error": f["error"]})))
}

// placeholders: other branches' features

fn control_deck(s: &Server) -> Result<(), String> {
    let r = s.call("check", json!({"text": "tempo 124\nsection groove 4 bars {\n  deck A: kick x ~ x ~\n  at bar 2: control: crossfader 0.5\n  control: move echo_out on B\n}\narrange groove\n"}));
    ensure(r["ok"] == true && r["canonical"].as_str().is_some_and(|t| t.contains("at bar 2: control: crossfader 0.5")), format!("a control: line does not check: {}", r["diagnostics"][0]["message"]))
}

fn structural_use(s: &Server) -> Result<(), String> {
    let r = s.call("check", json!({"text": "tempo 124\nuse ode_to_joy\ndeck B: melody ode_to_joy\n"}));
    ensure(r["ok"] == true && r["refs"].as_array().is_some_and(|a| a.iter().any(|x| x["kind"] == "use")), format!("use: {}", r["diagnostics"][0]["message"]))
}

fn dj_macro(s: &Server) -> Result<(), String> {
    let r = s.call("check", json!({"text": "tempo 124\ndeck A: kick x ~ x ~\non phrase: macro riser over 8 bars on A\n"}));
    ensure(r["ok"] == true, format!("macro: {}", r["diagnostics"][0]["message"]))
}

fn form_cross(s: &Server) -> Result<(), String> {
    let r = s.call("genre_template", json!({"genre": "techno", "form": "cross", "roles": {"a": "gen@1 dubstep seed=54876", "b": "gen@2 trap seed=54877"}}));
    ensure(r["form"].is_object() && r["resolution"].as_array().is_some_and(|a| a.iter().filter(|x| x["kind"] == "gen").count() >= 2), format!("cross: {r}"))
}

fn nested_templates(s: &Server) -> Result<(), String> {
    let r = s.call("genre_template", json!({"genre": "techno", "form": "rondo"}));
    ensure(r["resolution"].as_array().is_some_and(|a| a.iter().any(|x| x["nested"] == true && is_cid(&x["cid"]))), "a nested template's node has no CID of its own")
}

fn eject(s: &Server) -> Result<(), String> {
    let r = s.call("expand", json!({"text": GEN_PROG, "eject": "5:19"}));
    ensure(r["text"].as_str().is_some_and(|t| t.contains("# from: gen@2 techno seed=42 section=drop -> z")), format!("eject: {r}"))
}

// btwapi's words for the three fields: the descriptions are built from them, and the docs must say them too
use btwapi::resolution::{EXPANDED_CID_DOC, REFS_DOC, RESOLUTION_DOC};

fn rows() -> Vec<Row> {
    vec![
        Row { feature: "check reports gen references: resolved CID, fitted key/tempo, provenance (6668)", covers: &["gen@", "refs"], tool: "check", desc: &[REFS_DOC, "fitted to the program's key and tempo"], doc: &[REFS_DOC], inputs: &["text"], call: check_gen_refs, pending: None },
        Row { feature: "check's expand: each reference's expansion (6668)", covers: &["expand"], tool: "check", desc: &["With expand true, each entry of refs also carries its expansion"], doc: &["each one also carries its `expansion`"], inputs: &["expand"], call: check_expand, pending: None },
        Row { feature: "expanded_cid: the CID of the written-out program (6680)", covers: &["expanded_cid"], tool: "check", desc: &[EXPANDED_CID_DOC], doc: &[EXPANDED_CID_DOC], inputs: &[], call: check_expanded_cid, pending: None },
        Row { feature: "resolution: each reference's pin and generator version (6680)", covers: &["resolution", "nested"], tool: "check", desc: &[RESOLUTION_DOC], doc: &[RESOLUTION_DOC, "is listed with `nested: true`"], inputs: &[], call: check_resolution, pending: None },
        Row { feature: "listen says which parts a reference made, and what it was fitted to (6668)", covers: &["from"], tool: "listen", desc: &["says so in `from` {call, kind, resolved, place, generator, sections}", "`expanded_cid` and `resolution`"], doc: &["`listen` marks each part a reference made with `from`"], inputs: &["text", "bars"], call: listen_from, pending: None },
        Row { feature: "generate's generator: the version, as gen@N names it (6676)", covers: &[], tool: "generate", desc: &["generator picks the version (1 or 2, as gen@1 and gen@2 name them)", "The answer also has gen, the call that is this program"], doc: &[], inputs: &["generator"], call: generate_generator, pending: None },
        Row { feature: "generate's part: one role's let, as a gen@N let writes it (6676)", covers: &[], tool: "generate", desc: &["With part (a role) the answer gains that part and its let alone"], doc: &[], inputs: &["part", "section"], call: generate_part, pending: None },
        Row { feature: "generate's structured parts: {role, let, text, voice or kit, gen} (6676)", covers: &[], tool: "generate", desc: &["{role, section, sections, voice or kit, text, let, gen}"], doc: &[], inputs: &[], call: generate_parts, pending: None },
        Row { feature: "generate's seeds: a compact summary a seed, at most 16 (6676)", covers: &[], tool: "generate", desc: &["seeds (a list, or {count, start}, at most 16) answers a compact summary a seed"], doc: &[], inputs: &["seeds"], call: generate_seeds, pending: None },
        Row { feature: "expand of a gen program (6668)", covers: &[], tool: "expand", desc: &["A gen@N call (gen@2 techno seed=42 section=drop part=bass"], doc: &[], inputs: &["text"], call: expand_gen, pending: None },
        Row { feature: "genre_template's expanded_cid and resolution (6680)", covers: &[], tool: "genre_template", desc: &["The answer also has expanded_cid"], doc: &[], inputs: &["genre", "form"], call: genre_template_refs, pending: None },
        // ---- placeholders: other branches' features, expected to fail until they land
        Row { feature: "the control deck: `control:` lines, engine calls only (6671)", covers: &["control:"], tool: "check", desc: &["A `control:` line is the control deck"], doc: &["### The control deck"], inputs: &[], call: control_deck, pending: None },
        Row { feature: "structural imports: `use` canned sections and themes as text (6668, 6670)", covers: &["use"], tool: "check", desc: &["`use`"], doc: &[], inputs: &[], call: structural_use, pending: Some("`use` is being built on another branch") },
        Row { feature: "the DJ macro catalogue by name: `macro riser over 8 bars on A` (6670)", covers: &["macro"], tool: "check", desc: &["macro"], doc: &[], inputs: &[], call: dj_macro, pending: Some("macros by name are not built yet (6670)") },
        Row { feature: "role-bound form templates and `cross` two seeds (6673, 6674)", covers: &["cross"], tool: "genre_template", desc: &["cross (a quodlibet of two sources, roles a and b)", "each role is bound to a source"], doc: &[], inputs: &["form", "roles"], call: form_cross, pending: None },
        Row { feature: "nested templates with depth and bar caps, each node its own CID (6678, 6680)", covers: &[], tool: "genre_template", desc: &["depth"], doc: &[], inputs: &["form"], call: nested_templates, pending: Some("nested templates landed (6678), but template::expand returns one text, not its nodes, so a node has no CID of its own yet (6680 item 2)") },
        Row { feature: "eject over MCP: `expand` with `eject` (line:col) writes one reference out under `# from:` (6668)", covers: &["eject"], tool: "expand", desc: &["With eject \"line:col\" it writes out only the reference there"], doc: &[], inputs: &["eject"], call: eject, pending: None },
    ]
}

/// The features docs/BTW.md documents that the table must have a row for: the head of each engine call or reference the
/// "Two halves" table says is coming (`gen@1 ...` -> `gen@`, `control:`, `use`, `macro`), and the fields "What a reference
/// resolves to" names (the first word of each bullet, and the `with ...` options in it).
fn documented() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |k: String| {
        if !k.is_empty() && !out.contains(&k) {
            out.push(k);
        }
    };
    let section = |head: &str| -> String {
        let at = BTW_MD.find(head).unwrap_or_else(|| panic!("docs/BTW.md has no {head:?}"));
        let rest = &BTW_MD[at + head.len()..];
        let end = [rest.find("\n## "), rest.find("\n### ")].into_iter().flatten().min().unwrap_or(rest.len());
        rest[..end].to_string()
    };
    let head_of = |code: &str| -> String {
        let w: String = code.chars().take_while(|c| !c.is_whitespace() && *c != '(').collect();
        match w.find('@') {
            Some(i) => w[..=i].to_string(),
            None => w,
        }
    };
    for line in section("## Two halves: structure and engine").lines().filter(|l| l.starts_with('|')) {
        for cell in line.split(" | ") {
            if let Some(rest) = cell.split("*coming:*").nth(1) {
                if let Some(code) = rest.split('`').nth(1) {
                    add(head_of(code));
                }
            }
        }
    }
    let refs = section("### What a reference resolves to");
    for l in refs.lines() {
        if let Some(b) = l.strip_prefix("- ") {
            add(b.split_whitespace().next().unwrap_or("").to_string());
        }
    }
    for piece in refs.split("with `").skip(1).chain(refs.split("With `").skip(1)) {
        add(piece.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect());
    }
    out
}

#[test]
fn every_feature_is_reachable_and_described_through_mcp() {
    let rows = rows();
    // the table is the docs': a documented feature with no row fails here
    let covered: Vec<&str> = rows.iter().flat_map(|r| r.covers.iter().copied()).collect();
    let docs = documented();
    for k in &docs {
        assert!(covered.contains(&k.as_str()), "docs/BTW.md documents `{k}` and no row of the MCP exposure table covers it: add one (pending if it is still being built)");
    }
    // (the generator and the control deck are current now; `use` and `macro` are still coming)
    assert!(docs.iter().any(|k| k == "use") && docs.iter().any(|k| k == "expanded_cid"), "the docs scan found nothing: {docs:?}");

    let s = start();
    let list = s.rpc("tools/list", json!({}));
    let tools = list["tools"].as_array().expect("tools/list");
    let md = flat(BTW_MD);
    let mut failed: Vec<String> = Vec::new();
    let mut waiting: Vec<String> = Vec::new();
    for row in &rows {
        let verdict = (|| -> Result<(), String> {
            let t = tools.iter().find(|t| t["name"] == row.tool).ok_or(format!("no tool {}", row.tool))?;
            let desc = flat(t["description"].as_str().unwrap_or(""));
            for w in row.desc {
                ensure(desc.contains(&flat(w)), format!("the {} description does not say: {w}", row.tool))?;
            }
            for w in row.doc {
                ensure(md.contains(&flat(w)), format!("docs/BTW.md does not say: {w}"))?;
            }
            for i in row.inputs {
                ensure(t["inputSchema"]["properties"].get(*i).is_some(), format!("{} takes no `{i}`", row.tool))?;
            }
            (row.call)(&s)
        })();
        match (verdict, row.pending) {
            (Ok(()), None) => {}
            (Err(e), None) => failed.push(format!("{}: {e}", row.feature)),
            (Err(e), Some(why)) => waiting.push(format!("{}: {e} ({why})", row.feature)),
            (Ok(()), Some(_)) => failed.push(format!("{}: passes now, so it has landed: take its `pending` off, so the row guards it", row.feature)),
        }
    }
    for w in &waiting {
        eprintln!("pending (expected until it lands): {w}");
    }
    assert!(failed.is_empty(), "features not exposed through MCP as documented:\n  {}", failed.join("\n  "));
}
