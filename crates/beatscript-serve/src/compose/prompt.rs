//! Go `compose/prompt.go` and `extract.go`: the system prompt (the role statement, the engine's cheat-sheet, the
//! instruments, up to three vetted examples), the user turn, the repair turn, and the extraction of a program from a
//! model's reply. The prompt must be byte-identical with Go's: it is what the model sees, and the fake model's usage
//! (and so the cost) is counted from its length.

use serde_json::Value;

use super::engine::Rep;

pub const ROLE_STATEMENT: &str = r##"You are a composer who writes music as programs in btw, a small text language for whole tracks. Your reply is read by a program, not a person.

OUTPUT CONTRACT
- Reply with ONLY the BeatScript program. No prose before or after it, no explanation, no markdown, no code fences.
- Start with the "tempo" line and the "key" line.
- Use only instruments, kits and sounds named in the INSTRUMENTS list below. Never invent a name.
- A bar is 16 steps: write patterns that divide 16 evenly (16, 8, 4, 2 terms, or x*N with N dividing 16).
- For a song, write "section NAME N bars { ... }" blocks and an "arrange" line that puts them in order; a short sketch or loop may be a few lines with no sections. Name common parts once with "let" and reuse them.
- Keep within the requested length. If no length is given, make something of 32 to 64 bars. A smaller correct program is better than a larger broken one.
- Write comments only as "# ..." lines. Do not write lines that start with "#!".
- Everything you write is public domain (CC0). Never use "use restricted", "sample" or "loop" asset statements: only the sounds in the instruments list.
- If the checker sends back errors, fix every one of them and reply with the complete corrected program, again with no prose."##;

/// RE2's `\s`.
fn ws(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\x0c' | b'\r')
}

fn word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// The starts of `(?m)^\s*` matches: every line start, after its leading whitespace (which may run over newlines).
fn line_starts(t: &[u8]) -> Vec<usize> {
    let mut out = Vec::new();
    for i in 0..=t.len() {
        if i == 0 || t[i - 1] == b'\n' {
            let mut j = i;
            while j < t.len() && ws(t[j]) {
                j += 1;
            }
            out.push(j);
        }
    }
    out
}

/// `(?m)^\s*#!\s*<key>:\s*(\S+)`: the first value.
fn header_value(text: &str, key: &str) -> Option<String> {
    let t = text.as_bytes();
    for st in line_starts(t) {
        // \s* is greedy but may give back: try every split of the leading whitespace run, from the longest
        let mut j = st;
        if !t[j..].starts_with(b"#!") {
            continue;
        }
        j += 2;
        while j < t.len() && ws(t[j]) {
            j += 1;
        }
        if !t[j..].starts_with(key.as_bytes()) || t.get(j + key.len()) != Some(&b':') {
            continue;
        }
        j += key.len() + 1;
        while j < t.len() && ws(t[j]) {
            j += 1;
        }
        let k = (j..t.len()).find(|&k| ws(t[k])).unwrap_or(t.len());
        if k > j {
            return Some(text[j..k].to_string());
        }
    }
    None
}

/// `(?m)^\s*(use\s+restricted\b|sample\b|loop\s+\S)`
fn restricted_line(text: &str) -> bool {
    let t = text.as_bytes();
    let bnd = |k: usize| k >= t.len() || !word(t[k]);
    line_starts(t).into_iter().any(|j| {
        if t[j..].starts_with(b"use") && t.get(j + 3).is_some_and(|c| ws(*c)) {
            let mut k = j + 3;
            while k < t.len() && ws(t[k]) {
                k += 1;
            }
            if t[k..].starts_with(b"restricted") && bnd(k + 10) {
                return true;
            }
        }
        if t[j..].starts_with(b"sample") && bnd(j + 6) {
            return true;
        }
        if t[j..].starts_with(b"loop") && t.get(j + 4).is_some_and(|c| ws(*c)) {
            let mut k = j + 4;
            while k < t.len() && ws(t[k]) {
                k += 1;
            }
            if k < t.len() {
                return true;
            }
        }
        false
    })
}

/// `->.*\bover\s+\d+\s+bars\b` on one line.
fn has_ramp(text: &str) -> bool {
    text.split('\n').any(|l| {
        let Some(a) = l.find("->") else { return false };
        let t = l.as_bytes();
        let mut i = a + 2;
        while let Some(p) = l[i..].find("over") {
            let o = i + p;
            i = o + 1;
            if o > 0 && word(t[o - 1]) {
                continue;
            }
            let mut k = o + 4;
            let s0 = k;
            while k < t.len() && ws(t[k]) {
                k += 1;
            }
            if k == s0 {
                continue;
            }
            let d0 = k;
            while k < t.len() && t[k].is_ascii_digit() {
                k += 1;
            }
            if k == d0 {
                continue;
            }
            let s1 = k;
            while k < t.len() && ws(t[k]) {
                k += 1;
            }
            if k == s1 {
                continue;
            }
            if t[k..].starts_with(b"bars") && (k + 4 >= t.len() || !word(t[k + 4])) {
                return true;
            }
        }
        false
    })
}

/// `VetPublicProgram`: "" when the program may be put in front of the model, else why not.
pub fn vet_public_program(text: &str, rep: Option<&Value>, strict: bool) -> String {
    if let Some(l) = header_value(text, "license") {
        if !l.eq_ignore_ascii_case("CC0-1.0") {
            return format!("licence is {l}, not CC0-1.0");
        }
    }
    if let Some(v) = header_value(text, "visibility") {
        if !v.eq_ignore_ascii_case("public") {
            return format!("visibility is {v}");
        }
    }
    if restricted_line(text) {
        return "uses restricted or asset statements".into();
    }
    let Some(r) = rep.filter(|r| Rep(r).ok()) else { return "does not check".into() };
    let r = Rep(r);
    if r.restricted() {
        return "restricted".into();
    }
    if r.uses_assets() {
        return "uses assets".into();
    }
    if strict && r.warnings() > 0 {
        return format!("has {} warnings", r.warnings());
    }
    String::new()
}

/// `headerClaim`.
pub fn header_claim(text: &str) -> String {
    if let Some(l) = header_value(text, "license") {
        if !l.eq_ignore_ascii_case("CC0-1.0") {
            return format!("licensed {l}");
        }
    }
    if let Some(v) = header_value(text, "visibility") {
        if !v.eq_ignore_ascii_case("public") {
            return v;
        }
    }
    String::new()
}

/// `stripHeader`: the `#!` lines out, trimmed, one newline at the end.
pub fn strip_header(text: &str) -> String {
    let t = text.replace("\r\n", "\n");
    let out: Vec<&str> = t.split('\n').filter(|l| !l.trim_start_matches([' ', '\t', '\n', '\x0c', '\r']).starts_with("#!")).collect();
    format!("{}\n", out.join("\n").trim())
}

#[derive(Clone, Debug)]
pub struct ExampleCand {
    pub name: String,
    pub text: String,
    pub bytes: usize,
    pub sections: usize,
    pub ramp: bool,
}

impl ExampleCand {
    pub fn new(name: &str, text: String, sections: usize) -> ExampleCand {
        ExampleCand { name: name.into(), bytes: text.len(), ramp: has_ramp(&text), text, sections }
    }
}

/// `pickExamples`.
pub fn pick_examples(mut c: Vec<ExampleCand>, max_total: usize) -> Vec<ExampleCand> {
    c.sort_by(|a, b| a.bytes.cmp(&b.bytes).then_with(|| a.name.cmp(&b.name)));
    let mut out: Vec<ExampleCand> = Vec::new();
    let mut total = 0;
    let take = |out: &mut Vec<ExampleCand>, total: &mut usize, ok: &dyn Fn(&ExampleCand, usize) -> bool| {
        for e in &c {
            if out.iter().any(|o| o.name == e.name) || !ok(e, out.len()) || *total + e.bytes > max_total {
                continue;
            }
            *total += e.bytes;
            out.push(e.clone());
            return;
        }
    };
    take(&mut out, &mut total, &|e, _| e.sections == 0);
    let n = out.len();
    take(&mut out, &mut total, &|e, _| e.sections >= 4);
    if out.len() == n {
        take(&mut out, &mut total, &|e, _| e.sections >= 2);
    }
    take(&mut out, &mut total, &|e, _| e.ramp);
    take(&mut out, &mut total, &|_, len| len < 2);
    out
}

/// `compactInstruments`: `key: a, b, c` lines, voices / families / kits / palettes first.
pub fn compact_instruments(raw: &str) -> String {
    let Ok(Value::Object(m)) = serde_json::from_str::<Value>(raw) else { return raw.trim().to_string() };
    let order = |k: &str| ["voices", "families", "kits", "palettes"].iter().position(|x| *x == k);
    let mut keys: Vec<&String> = m.keys().collect();
    keys.sort_by(|a, b| match (order(a), order(b)) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        _ => a.cmp(b),
    });
    let mut out = Vec::new();
    for k in keys {
        let names: Vec<String> = match &m[k] {
            Value::Array(a) => a
                .iter()
                .map(|el| match el {
                    Value::String(s) => s.clone(),
                    Value::Object(o) => match (o.get("name"), o.get("id")) {
                        (Some(Value::String(n)), _) => n.clone(),
                        (_, Some(Value::String(n))) => n.clone(),
                        _ => super::gojson::to_string(el),
                    },
                    _ => super::gojson::to_string(el),
                })
                .collect(),
            other => vec![super::gojson::to_string(other)],
        };
        out.push(format!("{k}: {}", names.join(", ")));
    }
    out.join("\n")
}

/// The system prompt from its parts (the examples already chosen).
pub fn system_prompt(sheet: &str, instruments: &str, chosen: &[ExampleCand]) -> String {
    let mut b = String::from(ROLE_STATEMENT);
    b.push_str("\n\nGRAMMAR AND CHEAT-SHEET\n");
    b.push_str(sheet.trim());
    b.push_str("\n\nINSTRUMENTS (the only sound names you may use)\n");
    b.push_str(&compact_instruments(instruments));
    if !chosen.is_empty() {
        b.push_str("\n\nEXAMPLES (public-domain programs that pass the checker; study their shape, do not copy them)\n");
        for e in chosen {
            b.push_str(&format!("\n--- example: {} ---\n{}", e.name, e.text));
        }
        b.push_str("--- end of examples ---\n");
    }
    b
}

// ---------- the user turn ----------

pub fn length_words(l: &str) -> Option<&'static str> {
    match l {
        "short" => Some("short: about 16 to 24 bars in total"),
        "medium" => Some("medium: about 48 to 64 bars in total"),
        "long" => Some("long: about 96 to 128 bars in total"),
        _ => None,
    }
}

/// `fmtNum`.
pub fn fmt_num(f: f64) -> String {
    if f == (f as i64) as f64 {
        return format!("{}", f as i64);
    }
    let s = format!("{f:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[derive(Clone, Debug, Default)]
pub struct Request {
    pub prompt: String,
    pub genre: String,
    pub bpm: f64,
    pub key: String,
    pub length: String,
    pub base_cid: String,
    pub current: String,
}

fn constraint_lines(r: &Request) -> String {
    let mut b = String::new();
    if !r.genre.is_empty() {
        b.push_str(&format!("- genre: {}\n", r.genre));
    }
    if r.bpm > 0.0 {
        b.push_str(&format!("- tempo: {} BPM (write \"tempo {}\")\n", fmt_num(r.bpm), fmt_num(r.bpm)));
    }
    if !r.key.is_empty() {
        b.push_str(&format!("- key: {} (write \"key {}\")\n", r.key, r.key));
    }
    if !r.length.is_empty() {
        match length_words(&r.length) {
            Some(w) => b.push_str(&format!("- length: {w}\n")),
            None => b.push_str(&format!("- length: about {} bars in total\n", r.length)),
        }
    }
    b
}

/// `userMessage`.
pub fn user_message(r: &Request, base: &str, max_bytes: usize) -> String {
    let mut b = String::new();
    if !base.is_empty() {
        b.push_str("Here is a BeatScript program. Modify it as the request below says, keeping what the request does not mention, and reply with the COMPLETE modified program (not a diff).\n\n--- the program to modify ---\n");
        b.push_str(base.trim());
        b.push_str("\n--- end of the program ---\n\nRequested change: ");
    } else {
        b.push_str("Write a BeatScript program for this request: ");
    }
    b.push_str(&r.prompt);
    b.push('\n');
    let c = constraint_lines(r);
    if !c.is_empty() {
        b.push_str("\nConstraints:\n");
        b.push_str(&c);
    }
    b.push_str(&format!("\nThe program must be under {max_bytes} bytes. Reply with the program only."));
    b
}

fn is_error(d: &Value) -> bool {
    matches!(d["severity"].as_str().unwrap_or(""), "" | "error")
}

/// `repairMessage`.
/// The next user turn after a program that passed with warnings: the warnings, in the caret form the checker renders,
/// and the ask to clear them while keeping the music.
pub fn refine_message(diags: &[Value]) -> String {
    let ws: Vec<&Value> = diags.iter().filter(|d| d["severity"].as_str() == Some("warning")).collect();
    let n = ws.len();
    let mut b = if n == 1 { "Your program passes, but the checker has 1 warning. ".to_string() } else { format!("Your program passes, but the checker has {n} warnings. ") };
    b.push_str(
        "Fix every warning while keeping the music the same: the same tempo, key, sections, length and parts. \
         A pattern must divide the bar into sixteenths (16, 8, 4 or 2 steps per bar). \
         Reply with the complete corrected program only: no prose, no code fences, no \"#!\" lines.\n",
    );
    for (i, d) in ws.iter().enumerate() {
        if i == 10 {
            b.push_str(&format!("\n({} more warnings not shown)\n", n - 10));
            break;
        }
        b.push('\n');
        let line = d["line"].as_i64().unwrap_or(0);
        if line > 0 {
            b.push_str(&format!("[{}] line {}, col {}:\n", i + 1, line, d["col"].as_i64().unwrap_or(0)));
        } else {
            b.push_str(&format!("[{}]\n", i + 1));
        }
        let render = d["render"].as_str().unwrap_or("");
        if !render.is_empty() {
            b.push_str(render.trim_end_matches('\n'));
            b.push('\n');
        } else {
            b.push_str(&format!("warning: {}\n", d["message"].as_str().unwrap_or("")));
        }
        let help = d["help"].as_str().unwrap_or("");
        if !help.is_empty() && !render.contains(help) {
            b.push_str(&format!("help: {help}\n"));
        }
    }
    b
}

pub fn repair_message(diags: &[Value]) -> String {
    let mut errs: Vec<&Value> = diags.iter().filter(|d| is_error(d)).collect();
    if errs.is_empty() {
        errs = diags.iter().collect();
    }
    let n = errs.len();
    let mut b = if n == 1 { "The checker refused your program with 1 error. ".to_string() } else { format!("The checker refused your program with {n} errors. ") };
    b.push_str("Fix every one and reply with the complete corrected program only: no prose, no code fences, no \"#!\" lines.\n");
    for (i, d) in errs.iter().enumerate() {
        if i == 10 {
            b.push_str(&format!("\n({} more errors not shown)\n", n - 10));
            break;
        }
        b.push('\n');
        let line = d["line"].as_i64().unwrap_or(0);
        if line > 0 {
            b.push_str(&format!("[{}] line {}, col {}:\n", i + 1, line, d["col"].as_i64().unwrap_or(0)));
        } else {
            b.push_str(&format!("[{}]\n", i + 1));
        }
        let render = d["render"].as_str().unwrap_or("");
        if !render.is_empty() {
            b.push_str(render.trim_end_matches('\n'));
            b.push('\n');
        } else {
            b.push_str(&format!("error: {}\n", d["message"].as_str().unwrap_or("")));
        }
        let help = d["help"].as_str().unwrap_or("");
        if !help.is_empty() && !render.contains(help) {
            b.push_str(&format!("help: {help}\n"));
        }
    }
    b
}

/// `syntheticDiag`, as `api.Diagnostic` encodes it (start/end 0, omitempty fields).
pub fn synthetic_diag(msg: &str, help: &str) -> Value {
    serde_json::json!({"severity": "error", "line": 1, "col": 1, "end_col": 1, "start": 0, "end": 0, "message": msg, "help": help, "render": format!("error: {msg}")})
}

pub fn estimate_tokens(system: &str, msgs: &[super::model::Message]) -> i64 {
    let n = system.len() + msgs.iter().map(|m| m.text.len()).sum::<usize>();
    (n / 3 + 1) as i64
}

// ---------- extraction ----------

const STATEMENT_WORDS: &[&str] = &[
    "tempo", "swing", "key", "let", "def", "repeat", "section", "arrange", "import", "end", "loop", "evolve", "deck", "master", "control", "every", "on", "at", "after", "before", "for", "do", "swap", "move", "load", "queue", "play", "stop", "sync", "mute",
    "unmute", "filter", "delay", "reverb", "crush", "level", "low", "mid", "high", "crossfader", "resonance", "sample", "use", "kick", "snare", "hihat", "clap", "bass", "melody", "harmony", "arp", "rotate", "transpose", "octave", "invert", "reverse",
    "thin",
];

fn go_trim(s: &str) -> &str {
    s.trim()
}

/// `looksBeatscript`.
fn looks_beatscript(l: &str) -> bool {
    let t = go_trim(l);
    if t.is_empty() {
        return false;
    }
    if matches!(t.as_bytes()[0], b'#' | b'|' | b'}' | b'{') || t.ends_with('{') || t.ends_with('}') {
        return true;
    }
    let first = match t.find([' ', '\t', ':', '(', '=']) {
        Some(i) => &t[..i],
        None => t,
    };
    if STATEMENT_WORDS.contains(&first) {
        return true;
    }
    // ^[a-z_][a-z0-9_-]*\(
    let b = t.as_bytes();
    if !(b[0].is_ascii_lowercase() || b[0] == b'_') {
        return false;
    }
    let mut i = 1;
    while i < b.len() && (b[i].is_ascii_lowercase() || b[i].is_ascii_digit() || b[i] == b'_' || b[i] == b'-') {
        i += 1;
    }
    b.get(i) == Some(&b'(')
}

/// `isProseLine`.
fn is_prose_line(l: &str) -> bool {
    let t = go_trim(l);
    if t.is_empty() || looks_beatscript(t) {
        return false;
    }
    if t.starts_with("**") || t.starts_with("##") {
        return true;
    }
    // ^(\*|-|>|\d+[.)])\s
    let b = t.as_bytes();
    let bullet = match b[0] {
        b'*' | b'-' | b'>' => b.get(1).is_some_and(|c| ws(*c)),
        c if c.is_ascii_digit() => {
            let mut i = 0;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            matches!(b.get(i), Some(b'.') | Some(b')')) && b.get(i + 1).is_some_and(|c| ws(*c))
        }
        _ => false,
    };
    if bullet {
        return true;
    }
    matches!(t.chars().last(), Some('.' | '!' | '?' | ':'))
}

fn fenced_block(s: &str) -> Option<String> {
    let mut blocks: Vec<String> = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    let mut inb = false;
    for l in s.split('\n') {
        let t = go_trim(l);
        if t.starts_with("```") || t.starts_with("~~~") {
            if inb {
                blocks.push(cur.join("\n"));
                cur.clear();
                inb = false;
            } else {
                inb = true;
            }
            continue;
        }
        if inb {
            cur.push(l);
        }
    }
    if inb {
        blocks.push(cur.join("\n"));
    }
    if blocks.is_empty() {
        return None;
    }
    for b in &blocks {
        if b.split('\n').any(looks_beatscript) {
            return Some(b.clone());
        }
    }
    Some(blocks[0].clone())
}

fn drop_prose(lines: Vec<String>) -> Vec<String> {
    let mut ps: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if go_trim(&lines[i]).is_empty() {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < lines.len() && !go_trim(&lines[j]).is_empty() {
            j += 1;
        }
        ps.push((i, j));
        i = j;
    }
    if ps.is_empty() {
        return lines;
    }
    let prose_para = |p: (usize, usize)| {
        let mut prose = false;
        for l in &lines[p.0..p.1] {
            if looks_beatscript(l) {
                return false;
            }
            if is_prose_line(l) {
                prose = true;
            }
        }
        prose
    };
    if !lines.iter().any(|l| looks_beatscript(l)) {
        return lines;
    }
    let (mut lo, mut hi) = (0, ps.len());
    while lo < hi && prose_para(ps[lo]) {
        lo += 1;
    }
    while hi > lo && prose_para(ps[hi - 1]) {
        hi -= 1;
    }
    if lo >= hi {
        return lines;
    }
    let out = &lines[ps[lo].0..ps[hi - 1].1];
    let mut k = 0;
    while k < out.len() && is_prose_line(&out[k]) {
        k += 1;
    }
    let out = &out[k..];
    let mut m = out.len();
    while m > 0 && is_prose_line(&out[m - 1]) {
        m -= 1;
    }
    out[..m].to_vec()
}

/// `ExtractProgram`.
pub fn extract_program(raw: &str) -> String {
    let mut s = raw.replace("\r\n", "\n").replace('\r', "\n");
    if let Some(x) = s.strip_prefix('\u{feff}') {
        s = x.to_string();
    }
    if let Some(b) = fenced_block(&s) {
        s = b;
    }
    let mut lines = drop_prose(s.split('\n').map(str::to_string).collect());
    while lines.first().is_some_and(|l| go_trim(l).is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| go_trim(l).is_empty()) {
        lines.pop();
    }
    if lines.is_empty() {
        return String::new();
    }
    let lines: Vec<&str> = lines.iter().map(|l| l.trim_end_matches([' ', '\t'])).collect();
    lines.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_cases() {
        // compose/extract_test.go
        let prog = "tempo 120\nkey am\ndeck A: kick x ~ x ~\n";
        let blk = "tempo 120\nkey am\nsection a 8 bars {\n  deck A: kick x ~ x ~\n\n  master: reverb 0.2\n}\narrange a\n";
        let cases: Vec<(String, String)> = vec![
            (prog.into(), prog.into()),
            ("tempo 120\nkey am\ndeck A: kick x ~ x ~".into(), prog.into()),
            ("tempo 120\r\nkey am\r\ndeck A: kick x ~ x ~\r\n".into(), prog.into()),
            (format!("\u{feff}{prog}"), prog.into()),
            ("tempo 120  \nkey am\t\ndeck A: kick x ~ x ~   \n\n\n".into(), prog.into()),
            (format!("```\n{prog}```"), prog.into()),
            (format!("```beatscript\n{prog}```\n"), prog.into()),
            (format!("Sure! Here is a track:\n\n```beatscript\n{prog}```\n\nThis uses a four-on-the-floor kick.\n"), prog.into()),
            (format!("```beatscript\n{prog}"), prog.into()),
            (format!("Here is your program:\n{prog}"), prog.into()),
            (format!("Here is a dark techno track for you.\n\n{prog}"), prog.into()),
            (format!("{prog}\nThis program has a kick and a snare.\nEnjoy the music!\n"), prog.into()),
            (format!("{prog}\n**Notes**\n* the kick is steady\n* the snare is on 2 and 4\n"), prog.into()),
            (blk.into(), blk.into()),
            (format!("# a comment\n{prog}"), format!("# a comment\n{prog}")),
            (format!("```\nnot code at all\n```\n```beatscript\n{prog}```\n"), prog.into()),
            ("tempo 120\nkey am\nkik x ~ x ~\n".into(), "tempo 120\nkey am\nkik x ~ x ~\n".into()),
            (format!("{prog}\nBAD\n"), format!("{prog}\nBAD\n")),
            ("Tempo 120\nkey am\n".into(), "Tempo 120\nkey am\n".into()),
            ("I cannot do that.".into(), "I cannot do that.\n".into()),
            ("  \n\n".into(), "".into()),
            ("```\n```".into(), "".into()),
        ];
        for (i, (inp, want)) in cases.iter().enumerate() {
            assert_eq!(&extract_program(inp), want, "case {i}: {inp:?}");
        }
    }

    #[test]
    fn vets() {
        assert_eq!(header_value("tempo 1\n  #! license:  CC-BY-4.0\n", "license").as_deref(), Some("CC-BY-4.0"));
        assert!(restricted_line("tempo 1\n  sample x = \"a\"\n"));
        assert!(!restricted_line("tempo 1\nsamples\n"));
        assert!(has_ramp("master: filter lp 400 -> 6000 over 8 bars"));
        assert!(!has_ramp("master: filter lp 400 -> 6000 hover 8 bars"));
        assert_eq!(strip_header("#! license: CC0-1.0\ntempo 1\n\n"), "tempo 1\n");
    }
}
