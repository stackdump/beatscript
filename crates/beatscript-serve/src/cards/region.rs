//! Go `card/region.go`: which lines of a program its card shows.

use std::collections::{BTreeSet, HashMap, HashSet};

pub const MAX_LINES: usize = 12;
const MIN_REGION: usize = 8;

#[derive(Clone, Debug)]
struct Block {
    kind: &'static str, // "let" | "section" | "def" | "block" | "line"
    name: String,
    start: usize,
    end: usize,
}

/// One line of the card's code: its index in the canonical text (-1 for an elided marker), the text, and whether
/// a gap comes before it.
#[derive(Clone, Debug, PartialEq)]
pub struct RLine {
    pub idx: i64,
    pub text: String,
    pub gap_top: bool,
}

fn ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}
fn ident_cont(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// `reIdent.FindAllString(s, -1)`.
fn idents(s: &str) -> Vec<&str> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if ident_start(b[i]) {
            let j = i + 1 + b[i + 1..].iter().take_while(|c| ident_cont(**c)).count();
            out.push(&s[i..j]);
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

fn re_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0c)
}

/// `^([A-Za-z_][A-Za-z0-9_]*)\s*\(`: the name.
fn re_call(t: &str) -> Option<&str> {
    let b = t.as_bytes();
    if !b.first().is_some_and(|c| ident_start(*c)) {
        return None;
    }
    let n = 1 + b[1..].iter().take_while(|c| ident_cont(**c)).count();
    let mut j = n;
    while j < b.len() && re_space(b[j]) {
        j += 1;
    }
    (b.get(j) == Some(&b'(')).then(|| &t[..n])
}

/// `^let\s+([A-Za-z_][A-Za-z0-9_]*)\s*=`.
fn re_let_name(t: &str) -> Option<&str> {
    let rest = t.strip_prefix("let")?;
    let b = rest.as_bytes();
    let sp = b.iter().take_while(|c| re_space(**c)).count();
    if sp == 0 {
        return None;
    }
    let r = &rest[sp..];
    let rb = r.as_bytes();
    if !rb.first().is_some_and(|c| ident_start(*c)) {
        return None;
    }
    let n = 1 + rb[1..].iter().take_while(|c| ident_cont(**c)).count();
    let mut j = n;
    while j < rb.len() && re_space(rb[j]) {
        j += 1;
    }
    (rb.get(j) == Some(&b'=')).then(|| &r[..n])
}

fn indent_of(l: &str) -> usize {
    l.len() - l.trim_start_matches(' ').len()
}

fn scan_blocks(lines: &[&str]) -> Vec<Block> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        let t = l.trim();
        if t.is_empty() || indent_of(l) != 0 {
            i += 1;
            continue;
        }
        let mut b = Block { kind: "line", name: String::new(), start: i, end: i };
        let f: Vec<&str> = t.split_whitespace().collect();
        if f[0] == "let" {
            b.kind = "let";
            if let Some(m) = re_let_name(t) {
                b.name = m.to_string();
            }
        } else if t.ends_with('{') {
            b.kind = "block";
            match f[0] {
                "section" => {
                    b.kind = "section";
                    if f.len() > 1 {
                        b.name = f[1].to_string();
                    }
                }
                "def" => {
                    b.kind = "def";
                    if let Some(m) = idents(t.strip_prefix("def").unwrap_or(t).trim()).first() {
                        b.name = m.to_string();
                    }
                }
                _ => {}
            }
            let mut depth = 0i64;
            for (j, lj) in lines.iter().enumerate().skip(i) {
                let tj = lj.trim();
                if tj.ends_with('{') {
                    depth += 1;
                }
                if tj == "}" || tj.starts_with('}') {
                    depth -= 1;
                }
                if depth <= 0 {
                    b.end = j;
                    break;
                }
                if j == lines.len() - 1 {
                    b.end = j;
                }
            }
            i = b.end;
        }
        out.push(b);
        i += 1;
    }
    out
}

fn parts_in(lines: &[&str], from: usize, to: i64, defs: &HashMap<String, Block>, follow: bool) -> usize {
    let mut n = 0;
    let mut i = from as i64;
    while i <= to && (i as usize) < lines.len() {
        let t = lines[i as usize].trim();
        if t.starts_with("deck ") {
            if let Some((_, rest)) = t.split_once(':') {
                n += rest.matches(" | ").count() + 1;
            }
        } else if follow {
            if let Some(m) = re_call(t) {
                if let Some(d) = defs.get(m) {
                    n += parts_in(lines, d.start + 1, d.end as i64 - 1, defs, false);
                }
            }
        }
        i += 1;
    }
    n
}

/// `pickRegion`: the lines (at most MAX_LINES) and the section they came from ("" when not a section).
pub fn pick_region(lines: &[&str]) -> (Vec<RLine>, String) {
    let blocks = scan_blocks(lines);
    let mut defs: HashMap<String, Block> = HashMap::new();
    let mut lets: HashMap<String, Block> = HashMap::new();
    for b in &blocks {
        match b.kind {
            "def" => {
                defs.entry(b.name.clone()).or_insert_with(|| b.clone());
            }
            "let" => {
                lets.entry(b.name.clone()).or_insert_with(|| b.clone());
            }
            _ => {}
        }
    }
    let mut primary: Option<&Block> = None;
    let mut best: i64 = -1;
    for b in &blocks {
        if b.kind != "section" {
            continue;
        }
        if b.name == "drop" {
            primary = Some(b);
            break;
        }
        let p = parts_in(lines, b.start + 1, b.end as i64 - 1, &defs, true) as i64;
        if p > best {
            best = p;
            primary = Some(b);
        }
    }
    if primary.is_none() {
        primary = blocks.iter().find(|b| b.kind == "def");
    }
    let Some(primary) = primary else {
        let mut out = Vec::new();
        for (i, l) in lines.iter().enumerate() {
            if out.len() >= MAX_LINES {
                break;
            }
            if l.trim().is_empty() {
                continue;
            }
            out.push(RLine { idx: i as i64, text: l.to_string(), gap_top: false });
        }
        return (out, String::new());
    };

    let mut idx: BTreeSet<usize> = BTreeSet::new();
    if primary.end - primary.start + 1 > MAX_LINES {
        idx.insert(primary.start);
        idx.insert(primary.end);
        let mut rest: Vec<(usize, i32)> = Vec::new();
        for (i, l) in lines.iter().enumerate().take(primary.end).skip(primary.start + 1) {
            let score = if is_part_line(l) {
                30
            } else if is_ramp_line(l) {
                25
            } else if !hit_tokens(l).is_empty() {
                10
            } else {
                5
            };
            rest.push((i, score));
        }
        rest.sort_by(|a, b| b.1.cmp(&a.1)); // stable
        for (i, _) in rest {
            if idx.len() >= MAX_LINES {
                break;
            }
            idx.insert(i);
        }
    } else {
        idx.extend(primary.start..=primary.end);
    }

    let has_hit = idx.iter().any(|&i| !hit_tokens(lines[i]).is_empty());
    if (idx.len() < MIN_REGION || !has_hit) && idx.len() < MAX_LINES {
        let used = |from: usize, to: usize| -> Vec<String> { idents(&lines[from..=to].join("\n")).into_iter().map(str::to_string).collect() };
        let mut room = MAX_LINES as i64 - idx.len() as i64;
        let mut want_lets: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let note_lets = |names: Vec<String>, seen: &mut HashSet<String>, want: &mut Vec<String>| {
            for w in names {
                if lets.contains_key(&w) && !seen.contains(&w) {
                    seen.insert(w.clone());
                    want.push(w);
                }
            }
        };
        let mut chosen: Vec<Block> = Vec::new();
        let pick_defs = |room: &mut i64, seen: &mut HashSet<String>, chosen: &mut Vec<Block>| {
            for w in used(primary.start, primary.end) {
                let Some(d) = defs.get(&w) else { continue };
                let key = format!("def:{w}");
                if seen.contains(&key) || d.start == primary.start {
                    continue;
                }
                seen.insert(key);
                let size = (d.end - d.start + 1) as i64;
                if size <= *room && size <= 6 {
                    chosen.push(d.clone());
                    *room -= size;
                }
            }
        };
        note_lets(used(primary.start, primary.end), &mut seen, &mut want_lets);
        if has_hit {
            pick_defs(&mut room, &mut seen, &mut chosen);
        } else {
            room -= want_lets.len() as i64;
            pick_defs(&mut room, &mut seen, &mut chosen);
        }
        for d in chosen.clone() {
            note_lets(used(d.start, d.end), &mut seen, &mut want_lets);
        }
        for d in &chosen {
            idx.extend(d.start..=d.end);
        }
        for w in &want_lets {
            if idx.len() >= MAX_LINES {
                break;
            }
            idx.insert(lets[w].start);
        }
    }

    let order: Vec<usize> = idx.into_iter().collect();
    let out = order.iter().enumerate().map(|(k, &i)| RLine { idx: i as i64, text: lines[i].to_string(), gap_top: k > 0 && i != order[k - 1] + 1 }).collect();
    let section = if primary.kind == "section" { primary.name.clone() } else { String::new() };
    (out, section)
}

/// A step token on a line: rune column and length.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitToken {
    pub col: usize,
    pub n: usize,
}

pub fn hit_tokens(line: &str) -> Vec<HitToken> {
    let rs: Vec<char> = line.chars().collect();
    let is_sep = |r: char| matches!(r, ' ' | '[' | ']' | '<' | '>' | '|' | ',' | ':' | '=' | '{' | '}');
    let mut out = Vec::new();
    let mut in_quote = false;
    let mut i = 0;
    while i < rs.len() {
        let r = rs[i];
        if r == '"' {
            in_quote = !in_quote;
            i += 1;
            continue;
        }
        if in_quote || is_sep(r) {
            i += 1;
            continue;
        }
        if r == '#' && (i == 0 || rs[i - 1] == ' ') {
            break;
        }
        let mut j = i;
        let mut depth = 0;
        while j < rs.len() {
            if rs[j] == '(' {
                depth += 1;
            } else if rs[j] == ')' {
                if depth > 0 {
                    depth -= 1;
                }
            } else if depth == 0 && is_sep(rs[j]) {
                break;
            }
            j += 1;
        }
        let tok: String = rs[i..j].iter().collect();
        let prev_eq = i > 0 && rs[i - 1] == '=';
        if !prev_eq && super::lex::is_hit_token(&tok) {
            out.push(HitToken { col: i, n: j - i });
        }
        i = j;
    }
    out
}

pub fn is_ramp_line(l: &str) -> bool {
    l.contains("->") && l.contains("over") && l.contains("bar")
}

pub fn is_part_line(l: &str) -> bool {
    let t = l.trim();
    t.starts_with("deck ") || (re_call(t).is_some() && !t.starts_with("def "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_drop_and_caps() {
        let text = "tempo 128\nlet kick = x ~ ~ ~\nsection intro 4 bars {\n  deck A: kick\n}\nsection drop 8 bars {\n  deck A: kick | kick\n  deck B: bass\n}\narrange intro drop\n";
        let lines: Vec<&str> = text.trim_end_matches('\n').split('\n').collect();
        let (r, sec) = pick_region(&lines);
        assert_eq!(sec, "drop");
        assert!(r.len() <= MAX_LINES);
        assert!(r.iter().any(|l| l.text.starts_with("let kick")));
        assert!(r.iter().any(|l| l.gap_top));
    }

    #[test]
    fn tokens() {
        assert_eq!(hit_tokens("deck A: kick x ~ x inst=x \"x\""), vec![HitToken { col: 13, n: 1 }, HitToken { col: 17, n: 1 }]);
        assert!(is_ramp_line("master: filter lp 300 -> 2800 over 8 bars"));
        assert!(is_part_line("  deck A: kick"));
        assert!(is_part_line("groove()"));
        assert!(!is_part_line("def groove() {"));
    }
}
