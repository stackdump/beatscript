//! The compact grammar, generated from docs/BEATSCRIPT.md (`include_str!`), so it cannot drift from the language
//! reference: `beatscript cheatsheet`, `beatscript_api::cheatsheet()`, `/docs/cheatsheet.md` and the compose prompt are
//! all this text.
//!
//! BEATSCRIPT.md is split into sections by heading, and each section the sheet wants is cut down to the blocks it
//! needs (its code, tables, bullets and named paragraphs): the prose, the benchmarks, the design note on
//! restricted sounds, the test list, "Not done" and the whole-track example are left out. A plan entry that
//! finds nothing in BEATSCRIPT.md (a heading renamed, a paragraph rewritten) is reported by [`build`], and a test
//! fails on it.

/// docs/BEATSCRIPT.md, as compiled in.
pub const BEATSCRIPT_MD: &str = include_str!("../../../docs/BEATSCRIPT.md");

/// What to keep of one block of a section.
#[derive(Clone, Copy)]
enum Keep {
    /// Every block.
    All,
    /// Fenced code blocks.
    Code,
    /// The first fenced code block.
    FirstCode,
    /// The grammar: the EBNF blocks of every section in [`GRAMMAR`] as one (later productions replace earlier ones).
    Ebnf,
    /// Tables.
    Table,
    /// The first column of a table, one cell a line, as a code block (the lines a program can write).
    FirstColumn,
    /// Bullet lists.
    Bullets,
    /// The first paragraph.
    First,
    /// The paragraph that starts with this text.
    Para(&'static str),
}

/// The sections of the sheet, in order: the heading as BEATSCRIPT.md has it, and what to keep.
const PLAN: &[(&str, &[Keep])] = &[
    ("## The rules in one screen", &[Keep::All]),
    ("## Grammar (EBNF)", &[Keep::Ebnf, Keep::Para("`NAME` is")]),
    ("## Patterns: mini-notation", &[Keep::Code, Keep::Bullets]),
    ("### Generators", &[Keep::Table]),
    ("### Parameters", &[Keep::Table, Keep::Para("A **range**")]),
    // (the second paragraph, a tempo that moves, is the grammar's `tempo` and its examples: tempo 100 -> 130 over 8 bars exp)
    ("### Settings", &[Keep::First]),
    ("## Control", &[Keep::FirstColumn]),
    // (the first paragraph, a time word then a target, is the rules' `[when:] [target:] body`)
    ("### Time words", &[Keep::Table]),
    ("## The whole surface: `do`", &[Keep::Code]),
    ("### The wheel", &[Keep::Code, Keep::Para("A wheel line")]),
    // (the code block is the bullets' own examples put together; the bullets keep each one)
    ("## Functions and structure", &[Keep::Bullets]),
    ("### Ending", &[Keep::Table]),
    // (the paragraph on `<a b>` arguments and `gen` guards: the code block shows both, the grammar has the guard)
    ("### Evolving", &[Keep::FirstCode, Keep::Table]),
    // (the opening line, what a transform takes, is the table's own first column)
    ("### Pattern transforms", &[Keep::Table, Keep::Para("Marks touch")]),
    ("### Arrange mutations", &[Keep::FirstCode, Keep::Table, Keep::Para("Try [arrange-in-eight-lines]")]),
    ("### Start from a seed", &[Keep::FirstCode]),
    // (no "Name a source: `use`": the sheet is at its 30 KB cap, and a `use` only names what a program can write in place)
    ("### Sung lyrics", &[Keep::Para("**Found phrases:**")]),
    // (the four trims above made room for this and for the grammar's `station` productions under the 30 KB cap)
    ("## Radio: stations", &[Keep::Para("**Radio:**")]),
    ("## Sounds", &[Keep::Table]),
];

/// The sheet's own words, before and after what BEATSCRIPT.md gives.
const HEAD: &str = "# BeatScript cheat sheet

Generated from docs/BEATSCRIPT.md. BeatScript writes a whole track as text: pattern lines become Petri nets on decks A and B, and every other line is a control call. The grammar and rules are authoritative: run `check` and fix every `line:col` error before playing.
";

const TAIL: &str = "## On beatscript.app

- Everything above checks here; `import` and `<cid>` need the program (a link carries it). `sample` checks (a local file needs `use restricted`), but a program with either is not public: never stored or rendered, it keeps only its inline link.
- `loop` and the booth's lines (`move`, `swap pairs`, `deck B: load`, `deck A: sync`, `queue`, `crossfader`, `deck A: play` / `stop` / `mute`) check, and so do the sugar nouns (`wheel`, `trick`, `freeze`, `vocoder`, `vocal` ...). The browser engine plays `wheel` and `move` (every move but a `phrase_sync` that steps a seed); it does not play `swap pairs`, `load`, `sync`, `queue`, `macro` (`render` and `listen` do) or the other sugar nouns: the page lists each as \"not played here\". The grammar above leaves the booth's lines out; they belong to the desktop app.
- `inst=` and `kit=` take the voices and kits `/api/instruments` lists.
- ✦generate (key G): the seeded generator, no model or sign-in; `[` `]` step the seed as it plays.
";

#[derive(PartialEq)]
enum Block {
    Para(String),
    Code(String),
    Table(String),
    Bullets(String),
}

struct Section {
    heading: String,
    body: Vec<String>,
}

fn sections(md: &str) -> Vec<Section> {
    let mut out: Vec<Section> = Vec::new();
    let mut fence = false;
    for line in md.lines() {
        if line.starts_with("```") {
            fence = !fence;
        }
        if !fence && line.starts_with('#') && line.trim_start_matches('#').starts_with(' ') {
            out.push(Section { heading: line.trim_end().to_string(), body: Vec::new() });
        } else if let Some(s) = out.last_mut() {
            s.body.push(line.to_string());
        }
    }
    out
}

fn blocks(body: &[String]) -> Vec<Block> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < body.len() {
        let l = &body[i];
        if l.trim().is_empty() {
            i += 1;
        } else if l.starts_with("```") {
            let start = i;
            i += 1;
            while i < body.len() && !body[i].starts_with("```") {
                i += 1;
            }
            i = (i + 1).min(body.len());
            out.push(Block::Code(body[start..i].join("\n")));
        } else if l.starts_with('|') {
            let start = i;
            while i < body.len() && body[i].starts_with('|') {
                i += 1;
            }
            out.push(Block::Table(body[start..i].join("\n")));
        } else if l.starts_with("- ") {
            let start = i;
            i += 1;
            // continuation lines are indented; a blank line followed by another bullet keeps the list going
            while i < body.len() && (body[i].starts_with("  ") || body[i].starts_with("- ")) {
                i += 1;
            }
            out.push(Block::Bullets(body[start..i].join("\n")));
        } else {
            let start = i;
            while i < body.len() && !body[i].trim().is_empty() && !body[i].starts_with("```") && !body[i].starts_with('|') {
                i += 1;
            }
            out.push(Block::Para(body[start..i].join("\n")));
        }
    }
    out
}

fn kept<'a>(bs: &'a [Block], k: &Keep) -> Vec<&'a Block> {
    match k {
        Keep::All => bs.iter().collect(),
        Keep::Code => bs.iter().filter(|b| matches!(b, Block::Code(_))).collect(),
        Keep::FirstCode => bs.iter().find(|b| matches!(b, Block::Code(_))).into_iter().collect(),
        Keep::Ebnf => bs.iter().find(|b| matches!(b, Block::Code(t) if t.starts_with("```ebnf"))).into_iter().collect(),
        Keep::Table | Keep::FirstColumn => bs.iter().filter(|b| matches!(b, Block::Table(_))).collect(),
        Keep::Bullets => bs.iter().filter(|b| matches!(b, Block::Bullets(_))).collect(),
        Keep::First => bs.iter().find(|b| matches!(b, Block::Para(_))).into_iter().collect(),
        Keep::Para(p) => bs.iter().filter(|b| matches!(b, Block::Para(t) if t.starts_with(p))).collect(),
    }
}

/// What only the desktop app's booth does: a web page plays a track, it does not swap pairs, load a deck or
/// work the queue, so the sheet leaves these lines out (the checker still reads them).
const BOOTH_ONLY: &[&str] = &["swap", "move", "load", "queue", "play", "stop", "sync", "mute", "unmute", "crossfader", "deck A: play", "deck A: mute", "deck B: load"];

/// Rows of BEATSCRIPT.md's tables that only say another way to write what the table already says (`on phrase:` is `every 8 bars`, `on next bar:`
/// is no word, `end` is the default): BEATSCRIPT.md keeps them, the sheet does not need them.
const SYNONYM_ROWS: &[&str] = &["| `on phrase:` |", "| `on next bar:` |", "| `end` |"];

/// Example lines of BEATSCRIPT.md's code blocks that need the desktop (the vocal engine, a TD-3 on a cable): the sheet does not carry them.
const DESKTOP_LINES: &[&str] = &["vocal create ", "do td3 "];

fn booth_only(line: &str) -> bool {
    let t = line.trim().trim_start_matches('|').trim().trim_matches('"');
    BOOTH_ONLY.iter().any(|w| t.strip_prefix(w).is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '"', '/'])))
}

/// A block as the sheet prints it.
fn render(b: &Block, k: &Keep) -> String {
    match (b, k) {
        (Block::Table(t), Keep::FirstColumn) => first_column(t),
        (Block::Code(t), _) => squeeze(&t.lines().filter(|l| !DESKTOP_LINES.iter().any(|d| l.starts_with(d))).collect::<Vec<_>>().join("\n")),
        (Block::Table(t), _) => t.lines().filter(|r| !r.contains("td3") && !SYNONYM_ROWS.iter().any(|s| r.starts_with(s))).map(without_internal_refs).collect::<Vec<_>>().join("\n"),
        (Block::Bullets(t), _) => drop_items(t),
        (Block::Para(t), _) => t.clone(),
    }
}

/// A line without the parentheses that point into the Rust code (` (Bjorklund, as `djrust::gen::euclidean`)`):
/// a prompt has no use for where the engine keeps a function.
fn without_internal_refs(line: &str) -> String {
    let mut s = line.to_string();
    for marker in ["djrust::", "DeckAction::"] {
        while let Some(at) = s.find(marker) {
            let (Some(open), Some(close)) = (s[..at].rfind('('), s[at..].find(')')) else { break };
            let from = if s[..open].ends_with(' ') { open - 1 } else { open };
            s.replace_range(from..at + close + 1, "");
        }
    }
    s
}

/// A bullet list without the items of the desktop's own features (`import` has no snippet store here).
fn drop_items(list: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut skipping = false;
    for l in list.lines() {
        if l.starts_with("- ") {
            skipping = l.starts_with("- **`import");
        }
        if !skipping {
            out.push(l);
        }
    }
    out.join("\n")
}

/// Runs of spaces inside a line (alignment padding) made two, so a column of comments costs no more than its text.
fn squeeze(block: &str) -> String {
    block
        .lines()
        .map(|l| {
            let indent = l.len() - l.trim_start().len();
            let mut s = String::from(&l[..indent]);
            let mut run = 0;
            for c in l[indent..].trim_end().chars() {
                if c == ' ' {
                    run += 1;
                    if run <= 2 {
                        s.push(c);
                    }
                } else {
                    run = 0;
                    s.push(c);
                }
            }
            s
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A table's first column, a cell a line, in a code block (without the backticks of inline code).
fn first_column(table: &str) -> String {
    let mut out = String::from("```text\n");
    for row in table.lines().skip(2) {
        let cell = row.trim_start_matches('|').split(" | ").next().unwrap_or("").trim().trim_end_matches('|').trim();
        let cell = cell.replace('`', "").replace("\\|", "|");
        if !booth_only(&cell) {
            out.push_str(&cell);
            out.push('\n');
        }
    }
    out.push_str("```");
    out
}

/// One line of an EBNF production: its code and its trailing `(* comment *)`.
fn split_comment(l: &str) -> (String, Option<String>) {
    match l.find("(*") {
        Some(at) if l.trim_end().ends_with("*)") => (l[..at].trim_end().to_string(), Some(l[at..].trim_end().to_string())),
        _ => (l.trim_end().to_string(), None),
    }
}

/// The sections whose EBNF blocks make the grammar, in order.
const GRAMMAR: &[&str] = &["## Grammar (EBNF)"];

type Production = (String, Vec<(String, Option<String>)>);

/// The productions of one EBNF block.
fn productions(block: &str) -> Vec<Production> {
    let mut prods: Vec<Production> = Vec::new();
    for l in block.lines() {
        let t = l.trim();
        if l.starts_with("```") || t.is_empty() || (t.starts_with("(*") && t.ends_with("*)")) {
            continue;
        }
        let (code, comment) = split_comment(l);
        if l.starts_with(' ') {
            if let Some(p) = prods.last_mut() {
                p.1.push((code.trim().to_string(), comment));
            }
        } else if let Some((name, rest)) = code.split_once('=') {
            prods.push((name.trim().to_string(), vec![(rest.trim().to_string(), comment)]));
        }
    }
    prods
}

/// The grammar for this page, one block: the productions of every grammar section in order, a later
/// production replacing an earlier one of its name (`action = ... | x` adds `x` to it), the booth's own
/// actions and `import` left out, comment-only lines (banners) dropped and the trailing comments that run
/// long cut. The productions and the short examples stay.
fn merged_grammar(secs: &[Section], missing: &mut Vec<String>) -> String {
    // the sheet carries the whole grammar (grammar.ebnf, through BEATSCRIPT.md's generated block): only its short comments, the
    // examples (`4 bars   1/2 bar`), fit the compose prompt's budget; the Grammar page and BEATSCRIPT.md have them all
    const LONG: usize = 28;
    let mut all: Vec<Production> = Vec::new();
    for (gi, h) in GRAMMAR.iter().enumerate() {
        let Some(s) = secs.iter().find(|s| s.heading == *h) else {
            missing.push(format!("heading `{h}`"));
            continue;
        };
        for b in blocks(&s.body) {
            let Block::Code(t) = b else { continue };
            if !t.starts_with("```ebnf") {
                continue;
            }
            for (mut name, mut lines) in productions(&t) {
                // the additions' `call` is a def's call, a statement of its own: not the generator call an atom is
                if gi > 0 && name == "call" {
                    name = "defcall".to_string();
                }
                if gi > 0 && name == "body" {
                    for l in lines.iter_mut() {
                        l.0 = l.0.replace("| call", "| defcall");
                    }
                }
                match all.iter_mut().find(|(n, _)| *n == name) {
                    Some(prev) if lines[0].0.starts_with("...") => {
                        // an extension: its alternatives join the production's
                        lines[0].0 = lines[0].0.trim_start_matches("...").trim_start().to_string();
                        if let Some(last) = prev.1.last_mut() {
                            last.0 = last.0.trim_end_matches(';').trim_end().to_string();
                        }
                        prev.1.extend(lines);
                    }
                    Some(prev) => prev.1 = lines,
                    None => all.push((name, lines)),
                }
            }
        }
    }
    let mut out = String::from("```ebnf\n");
    for (name, mut lines) in all {
        match name.as_str() {
            // (`sample` and `use restricted` are of a program that is never public: the page checks them, it cannot play them;
            // `use SOURCE as NAME` only names what a program can write in place, and BEATSCRIPT.md teaches it: the sheet is at its cap)
            "MOVE" | "SPEC" | "import" | "sample" | "license" | "use" | "use_source" => continue,
            // (`engine` is what the control deck takes: the booth's lines, and a timed `tempo`)
            "action" | "engine" => {
                lines.retain(|(c, _)| !booth_alt(c));
                // the first alternative may be gone: the next one starts the production
                if let Some(first) = lines.first_mut() {
                    first.0 = first.0.trim_start_matches('|').trim().to_string();
                }
                if let Some(last) = lines.last_mut() {
                    if !last.0.ends_with(';') {
                        last.0.push_str(" ;");
                    }
                }
            }
            _ => {}
        }
        for (i, (code, comment)) in lines.iter().enumerate() {
            let code = code.replace("| import | end", "| end").replace("| sample | use | set", "| set");
            let line = if i == 0 { format!("{name} = {code}") } else { format!("    {code}") };
            out.push_str(&squeeze(&line));
            if let Some(c) = comment.as_ref().filter(|c| c.len() <= LONG) {
                out.push_str("  ");
                out.push_str(&squeeze(c));
            }
            out.push('\n');
        }
    }
    out.push_str("```");
    out
}

/// An alternative of `action` that is the desktop booth's (swap, move, load, queue, play / stop / sync, mute,
/// crossfader).
fn booth_alt(code: &str) -> bool {
    let c = code.trim_start_matches('|').trim_start();
    ["\"swap\"", "\"move\"", "\"load\"", "\"queue\"", "\"play\"", "\"mute\"", "\"crossfader\""].iter().any(|w| c.starts_with(w)) || c.starts_with("\"swap\" \"pairs\"")
}

/// The sheet made from `md`, and what the plan could not find in it (empty when `md` is the BEATSCRIPT.md the
/// plan was written against).
pub fn build(md: &str) -> (String, Vec<String>) {
    let secs = sections(md);
    let mut missing = Vec::new();
    let mut out = String::from(HEAD);
    for (heading, keeps) in PLAN {
        let Some(s) = secs.iter().find(|s| s.heading == *heading) else {
            missing.push(format!("heading `{heading}`"));
            continue;
        };
        let bs = blocks(&s.body);
        // blocks in the order BEATSCRIPT.md has them, each once, printed as the first rule that picked it says
        let mut picked: Vec<(usize, &Keep)> = Vec::new();
        for k in keeps.iter() {
            let found = kept(&bs, k);
            if found.is_empty() {
                missing.push(format!("under `{heading}`: {}", describe(k)));
            }
            for b in found {
                let at = bs.iter().position(|x| std::ptr::eq(x, b)).unwrap_or(0);
                if !picked.iter().any(|(i, _)| *i == at) {
                    picked.push((at, k));
                }
            }
        }
        picked.sort_by_key(|(i, _)| *i);
        if picked.is_empty() {
            continue;
        }
        out.push('\n');
        out.push_str(heading);
        out.push('\n');
        for (i, k) in picked {
            out.push('\n');
            match k {
                Keep::Ebnf => out.push_str(&merged_grammar(&secs, &mut missing)),
                _ => out.push_str(&render(&bs[i], k)),
            }
            out.push('\n');
        }
    }
    out.push('\n');
    out.push_str(TAIL);
    (out, missing)
}

fn describe(k: &Keep) -> String {
    match k {
        Keep::All => "anything".into(),
        Keep::Code | Keep::FirstCode => "a code block".into(),
        Keep::Ebnf => "an ebnf block".into(),
        Keep::Table | Keep::FirstColumn => "a table".into(),
        Keep::Bullets => "a bullet list".into(),
        Keep::First => "a paragraph".into(),
        Keep::Para(p) => format!("a paragraph starting `{p}`"),
    }
}

/// The compact grammar as markdown, generated from docs/BEATSCRIPT.md.
pub fn cheatsheet() -> String {
    build(BEATSCRIPT_MD).0
}
