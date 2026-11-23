//! Go `card/lex.go`: a tiny lexer (colours a program when the checker gives no spans, finds the hit tokens the card
//! lights) and the label helpers (cleanText, shortCID, keyLabel, clock, fmtBPM, licenseBadge).

/// One highlight span: UTF-8 byte offsets and a class name (Go `api.Span`).
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub class: String,
}

const KEYWORDS: &[&str] = &[
    "deck", "master", "every", "on", "at", "bar", "bars", "phrase", "over", "tempo", "key", "swing", "with", "pairs", "pair", "group", "bpm", "section", "def", "let", "repeat", "arrange", "end",
    "after", "evolve", "before", "stop", "when", "import", "use", "sample", "loop", "now", "for", "in",
];
const ROLES: &[&str] = &["kick", "snare", "hihat", "hat", "clap", "rim", "tom", "perc", "ride", "crash", "bass", "sub", "melody", "harmony", "lead", "pad", "arp", "chords", "stab", "drums", "fx"];
const ACTIONS: &[&str] = &[
    "filter", "reverb", "level", "resonance", "move", "wheel", "swap", "delay", "pan", "volume", "mute", "unmute", "solo", "load", "play", "pause", "rotate", "transpose", "reverse", "octave", "slow",
    "fast", "degrade", "humanize", "lp", "hp", "bp", "off",
];

/// `^[A-Za-z_$][A-Za-z0-9_$#-]*` at the start of `s`: the match length (0 = none).
fn re_word(s: &[u8]) -> usize {
    match s.first() {
        Some(c) if c.is_ascii_alphabetic() || *c == b'_' || *c == b'$' => {}
        _ => return 0,
    }
    1 + s[1..].iter().take_while(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'$' | b'#' | b'-')).count()
}

/// `^[0-9]+(\.[0-9]+)?[k%]?`.
fn re_num(s: &[u8]) -> usize {
    let mut i = s.iter().take_while(|c| c.is_ascii_digit()).count();
    if i == 0 {
        return 0;
    }
    if s.get(i) == Some(&b'.') {
        let f = s[i + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
        if f > 0 {
            i += 1 + f;
        }
    }
    if matches!(s.get(i), Some(b'k') | Some(b'%')) {
        i += 1;
    }
    i
}

/// `^(x|[a-g][#b]?-?[0-9])[!^*.0-9]*$`.
fn re_hit(t: &str) -> bool {
    let b = t.as_bytes();
    let tail = |from: usize| b[from..].iter().all(|c| matches!(c, b'!' | b'^' | b'*' | b'.' | b'0'..=b'9'));
    if b.first() == Some(&b'x') && tail(1) {
        return true;
    }
    if !matches!(b.first(), Some(b'a'..=b'g')) {
        return false;
    }
    // [#b]? -? [0-9], trying each optional part present and absent (the regexp backtracks)
    for acc in [true, false] {
        let mut i = 1;
        if acc {
            if !matches!(b.get(i), Some(b'#') | Some(b'b')) {
                continue;
            }
            i += 1;
        }
        for minus in [true, false] {
            let mut j = i;
            if minus {
                if b.get(j) != Some(&b'-') {
                    continue;
                }
                j += 1;
            }
            if matches!(b.get(j), Some(b'0'..=b'9')) && tail(j + 1) {
                return true;
            }
        }
    }
    false
}

pub fn is_hit_token(tok: &str) -> bool {
    if tok.starts_with("chord(") && tok.ends_with(')') {
        return true;
    }
    re_hit(tok)
}

/// `basicSpans`: coarse colouring, offsets are UTF-8 bytes into text.
pub fn basic_spans(text: &str) -> Vec<Span> {
    let mut out = Vec::new();
    let mut off = 0;
    for line in text.split_inclusive('\n').chain(if text.ends_with('\n') || text.is_empty() { Some("") } else { None }) {
        line_spans(line.trim_end_matches(['\r', '\n']), off, &mut out);
        off += line.len();
    }
    out
}

fn line_spans(l: &str, base: usize, out: &mut Vec<Span>) {
    let b = l.as_bytes();
    let mut add = |a: usize, e: usize, c: &str| out.push(Span { start: base + a, end: base + e, class: c.to_string() });
    let mut i = 0;
    let mut prev = "";
    while i < b.len() {
        let c = b[i];
        if c == b' ' || c == b'\t' {
            i += 1;
        } else if c == b'#' && (i == 0 || b[i - 1] == b' ' || b[i - 1] == b'\t') {
            add(i, b.len(), "comment");
            return;
        } else if c == b'~' {
            add(i, i + 1, "rest");
            i += 1;
        } else if c == b'"' {
            let end = match l[i + 1..].find('"') {
                Some(j) => i + 1 + j + 1,
                None => b.len(),
            };
            add(i, end, "string");
            i = end;
        } else if l[i..].starts_with("->") || l[i..].starts_with("..") {
            add(i, i + 2, "operator");
            i += 2;
        } else if matches!(c, b'(' | b')' | b'[' | b']' | b'<' | b'>' | b'{' | b'}' | b'|' | b',' | b':' | b'=' | b'*') {
            add(i, i + 1, "operator");
            i += 1;
        } else if c.is_ascii_digit() {
            let m = re_num(&b[i..]);
            add(i, i + m, "number");
            i += m;
        } else {
            let wl = re_word(&b[i..]);
            if wl == 0 {
                let n = l[i..].chars().next().map_or(1, char::len_utf8);
                add(i, i + n, "word");
                i += n;
                continue;
            }
            let w = &l[i..i + wl];
            let mut end = i + wl;
            while end < b.len() && (b[end] == b'!' || b[end] == b'^') {
                end += 1;
            }
            let tok = &l[i..end];
            let next = b.get(end).copied().unwrap_or(0);
            let class = if next == b'(' {
                "generator"
            } else if next == b'=' && !l[end..].starts_with("==") {
                "param"
            } else if prev == "deck" && (w == "A" || w == "B") {
                "deck"
            } else if KEYWORDS.contains(&w) {
                "keyword"
            } else if ROLES.contains(&w) {
                "role"
            } else if ACTIONS.contains(&w) {
                "action"
            } else if is_hit_token(tok) {
                "step"
            } else {
                "word"
            };
            add(i, end, class);
            prev = w;
            i = end;
        }
    }
}

/// `classAt`: every byte's class ("" = none), later spans win.
pub fn class_at(text: &str, spans: &[Span]) -> Vec<String> {
    let mut cls = vec![String::new(); text.len()];
    for s in spans {
        if s.end > text.len() || s.start >= s.end {
            continue;
        }
        for c in &mut cls[s.start..s.end] {
            c.clone_from(&s.class);
        }
    }
    cls
}

fn is_control(c: char) -> bool {
    // unicode.IsControl: C0, DEL and C1
    (c as u32) < 0x20 || (0x7f..=0x9f).contains(&(c as u32))
}

/// `cleanLine`: tabs to two spaces, control characters (and U+2028/9, U+FFFD) to a space.
pub fn clean_line(s: &str) -> String {
    let mut b = String::with_capacity(s.len());
    for r in s.chars() {
        if r == '\t' {
            b.push_str("  ");
        } else if r == '\u{FFFD}' || is_control(r) || r == '\u{2028}' || r == '\u{2029}' {
            b.push(' ');
        } else {
            b.push(r);
        }
    }
    b
}

fn is_bidi_control(r: char) -> bool {
    matches!(r as u32, 0x061C | 0x200E | 0x200F | 0x202A..=0x202E | 0x2066..=0x2069)
}

/// `cleanText`: no control characters, spaces collapsed, at most `max` runes.
pub fn clean_text(s: &str, max: usize) -> String {
    let mapped: String = s.chars().map(|r| if is_control(r) || r == '\u{2028}' || r == '\u{2029}' || r == '\u{FFFD}' || is_bidi_control(r) { ' ' } else { r }).collect();
    let joined = mapped.split_whitespace().collect::<Vec<_>>().join(" ");
    trunc_runes(&joined, max)
}

pub fn trunc_runes(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max - 1).collect();
    format!("{}…", head.trim_end_matches(' '))
}

/// `shortCID`: z4EBG9…7v.
pub fn short_cid(cid: &str) -> String {
    let r: Vec<char> = cid.chars().collect();
    if r.len() <= 9 {
        return cid.to_string();
    }
    format!("{}…{}", r[..6].iter().collect::<String>(), r[r.len() - 2..].iter().collect::<String>())
}

/// `keyLabel`: fm -> F minor, c -> C major, d:dorian -> D dorian.
pub fn key_label(k: &str) -> String {
    let k = k.trim();
    if k.is_empty() {
        return String::new();
    }
    let name = |r: &str| -> String {
        let mut cs = r.chars();
        match cs.next() {
            None => String::new(),
            Some(c) => c.to_uppercase().collect::<String>() + &cs.as_str().replace('#', "♯"),
        }
    };
    if let Some((root, mode)) = k.split_once(':') {
        return format!("{} {mode}", name(root));
    }
    let b = k.as_bytes();
    if b.len() >= 2 && k.ends_with('m') && b"abcdefg".contains(&b[0]) {
        return format!("{} minor", name(&k[..k.len() - 1]));
    }
    if (1..=2).contains(&b.len()) && b"abcdefg".contains(&b[0]) {
        return format!("{} major", name(k));
    }
    k.to_string()
}

pub fn clock(seconds: f64) -> String {
    let s = (seconds + 0.5) as i64;
    format!("{}:{:02}", s / 60, s % 60)
}

pub fn fmt_bpm(t: f64) -> String {
    if t == (t as i64) as f64 {
        return format!("{}", t as i64);
    }
    format!("{t:.2}").trim_end_matches('0').trim_end_matches('.').to_string()
}

/// `licenseBadge`: CC0-1.0 -> CC0, CC-BY-4.0 -> CC BY 4.0.
pub fn license_badge(l: &str) -> String {
    let l = l.trim();
    let u = l.to_uppercase();
    if matches!(u.as_str(), "" | "CC0" | "CC0-1.0") {
        return "CC0".into();
    }
    if u.starts_with("CC-") {
        return trunc_runes(&u.replace('-', " "), 12);
    }
    trunc_runes(l, 12)
}

/// The `#! key: value` header (name, license, visibility) of an original text (Go `headerMeta`).
#[derive(Clone, Debug, Default)]
pub struct HeaderMeta {
    pub name: String,
    pub license: String,
    pub visibility: String,
}

pub fn header_meta(text: &str) -> HeaderMeta {
    let mut m = HeaderMeta::default();
    for l in text.split('\n') {
        let t = l.trim();
        if t.is_empty() {
            continue;
        }
        let Some(rest) = t.strip_prefix("#!") else {
            if t.starts_with('#') {
                continue;
            }
            break;
        };
        let Some((k, v)) = rest.trim().split_once(':') else { continue };
        let v = v.trim().to_string();
        match k.trim().to_lowercase().as_str() {
            "name" => m.name = v,
            "license" => m.license = v,
            "visibility" => m.visibility = v,
            _ => {}
        }
    }
    m
}

/// `firstComment`: the first plain comment line of an original text.
pub fn first_comment(text: &str) -> String {
    for l in text.split('\n') {
        let t = l.trim();
        if t.starts_with('#') && !t.starts_with("#!") {
            let c = clean_text(t.trim_start_matches('#').trim(), 200);
            if !c.is_empty() {
                return c;
            }
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hits() {
        for t in ["x", "x*2", "x!", "f1!", "g#1^", "c4", "chord(am)", "bb1", "a-1", "x.5"] {
            assert!(is_hit_token(t), "{t}");
        }
        for t in ["~", "kick", "h1", "a", "ab", "c#", "xx", "inst"] {
            assert!(!is_hit_token(t), "{t}");
        }
    }

    #[test]
    fn helpers() {
        assert_eq!(key_label("fm"), "F minor");
        assert_eq!(key_label("c"), "C major");
        assert_eq!(key_label("d:dorian"), "D dorian");
        assert_eq!(key_label("c#m"), "C♯ minor");
        assert_eq!(fmt_bpm(124.0), "124");
        assert_eq!(fmt_bpm(128.5), "128.5");
        assert_eq!(clock(125.4), "2:05");
        assert_eq!(license_badge("CC-BY-4.0"), "CC BY 4.0");
        assert_eq!(license_badge("cc0-1.0"), "CC0");
        assert_eq!(short_cid("z4EBG9jFzzkVrdxGXZhj7EUYYSYzLhhZJ4rGkQXpuGm7KeDA7Ya"), "z4EBG9…Ya");
        assert_eq!(clean_text("  a\u{202E}b \n c ", 60), "a b c");
        assert_eq!(trunc_runes("abcdef", 4), "abc…");
        assert_eq!(first_comment("#! name: x\n# hello  world\n"), "hello world");
    }

    #[test]
    fn spans() {
        let s = basic_spans("deck A: kick x ~ # c\nlet a = 1\n");
        let cls: Vec<&str> = s.iter().map(|s| s.class.as_str()).collect();
        assert_eq!(cls, ["keyword", "deck", "operator", "role", "step", "rest", "comment", "keyword", "word", "operator", "number"]);
    }
}
