//! What a model (or a person) reads to learn the language: `/llms.txt`, `/llms-full.txt`, the raw `/docs/BEATSCRIPT.md`,
//! `/docs/cheatsheet.md`, `/docs/api.md`, and their HTML pages `/docs/` (index), `/docs/cheatsheet`,
//! `/docs/language`, `/docs/api` with `/docs/assets/docs.css|js`. The Go server's `internal/btwsite/docs`
//! (docs.go + html.go, branch feat/docs-html), port for port.
//!
//! Where the texts come from, as in Go: `-docs` holds the generated `BEATSCRIPT.md` and `cheatsheet.md` (either may be
//! absent; the cheat-sheet then comes from `beatscript_api::cheatsheet()`, in-process, where Go asked its beatscript child).
//! `api.md` and the two assets are embedded in the Go binary; here they are read from `-docs-src` (the Go
//! package's directory, `internal/btwsite/docs` of a beats-bitwrap-io checkout), so the bytes are the same file.
//!
//! The HTML is goldmark's (CommonMark + tables + strikethrough, auto heading ids, hard wraps, raw HTML omitted)
//! as written by the Go package's own renderers, reproduced over pulldown-cmark's events: `Writer` below.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{header, HeaderMap, HeaderValue, Method, Response};
use bytes::Bytes;
use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use serde_json::json;

use crate::content;
use crate::site::{html_escape, Examples};

pub const EXTERNAL_LINKS: [&str; 1] = ["https://blog.stackdump.com/posts/generator-you-can-read"];

pub struct Docs {
    docs_dir: Option<PathBuf>,
    src_dir: Option<PathBuf>,
    public: String,
    full_example: String,
    cs: Mutex<Option<(String, Instant)>>,
    cache: Mutex<HashMap<String, (String, Bytes, String)>>,
}

/// The schema page (docs/SCHEMA.md): built in, so `/schema` answers on every install (a JSON-LD processor fetches it).
const SCHEMA_MD: &str = include_str!("../../../docs/SCHEMA.md");
/// The open-data page (docs/OPEN-DATA.md), built in like the schema: the dumps' manifests link it.
const OPEN_DATA_MD: &str = include_str!("../../../docs/OPEN-DATA.md");
/// Found phrases: borrowing text for its shape (docs/beatscript/found-phrases.md, `beatscript_serve::found`): `/docs/found-phrases` and its
/// markdown, listed in the docs index, its JSON-LD, llms.txt and the sitemap. Built in, like the schema and the open-data page.
const FOUND_MD: &str = include_str!("../../../docs/beatscript/found-phrases.md");
const FOUND_PATH: &str = "/docs/found-phrases";
const FOUND_TITLE: &str = "Found phrases";
const FOUND_BLURB: &str = "Borrowing public-domain text for its shape: pick a line for its meter, match the meter to the groove, keep the vocal low, cycle it buried and spotlit. The technique behind the phrasebook and the phrase templates.";
/// Forms: arrange shapes from classical music (docs/beatscript/forms.md, `beatscript_serve::forms`, the form templates): `/docs/forms` and its
/// markdown, listed in the docs index, its JSON-LD, llms.txt and the sitemap. Built in, like found phrases.
const FORMS_MD: &str = include_str!("../../../docs/beatscript/forms.md");
const FORMS_PATH: &str = beatscript_serve::forms::DOC_PATH;
const FORMS_TITLE: &str = "Forms";
const FORMS_BLURB: &str = "Arrange shapes from classical music: quodlibet, passacaglia, canon, rondo, fugue, theme and variations, each a section shape and a mutation plan, as templates built from public-domain themes.";
/// Embedding a program on another site (docs/beatscript/embed.md, cards/embed.rs): `/docs/embed` and its markdown, listed in the
/// docs index, its JSON-LD, llms.txt and the sitemap. Built in, like the forms.
const EMBED_MD: &str = include_str!("../../../docs/beatscript/embed.md");
const EMBED_PATH: &str = "/docs/embed";
const EMBED_TITLE: &str = "Embed a program";
const EMBED_BLURB: &str = "One embed config for every way to embed a program: the player, or the whole studio (an MCP server inside the page), in a Claude artifact or any page; an iframe, oEmbed, link previews that play, a README card, notebooks, VS Code, plain audio and feeds.";
/// Radio: stations (docs/beatscript/radio.md, `djlive::station`, the MCP `station` tool and `POST /api/station`): `/docs/radio` and its
/// markdown, listed in the docs index, its JSON-LD, llms.txt and the sitemap. Built in, like the forms.
const RADIO_MD: &str = include_str!("../../../docs/beatscript/radio.md");
const RADIO_PATH: &str = "/docs/radio";
const RADIO_TITLE: &str = "Radio: stations";
const RADIO_BLURB: &str = "Stations: endless, deterministic runs of tracks, each station one small program. Position N is always the same track; turn a knob (genre, seed, step, forms, tempo, key, length) and you move to a neighbouring station, at the same position.";
/// Polish a mix: the listen loop (docs/beatscript/polish.md, the MCP `listen` tool's mix analysis, crates/beatscript-cli/src/mix.rs):
/// `/docs/polish` and its markdown, listed in the docs index, its JSON-LD, llms.txt and the sitemap. Built in, like the forms.
const POLISH_MD: &str = include_str!("../../../docs/beatscript/polish.md");
const POLISH_PATH: &str = "/docs/polish";
const POLISH_TITLE: &str = "Polish a mix";
const POLISH_BLURB: &str = "The listen loop: check, listen, change one line, listen again with compare. What listen measures (bands, clashes, true peak, loudness range, width), the delivery targets, the order of fixes and the BeatScript lines that make each one.";

/// Who authored a program and what rights it has (docs/AUTHORSHIP.md, `beatscript_serve::rights`): `/authorship` and
/// `/authorship.md`, listed in the docs index, its JSON-LD and llms.txt.
use beatscript_serve::rights::{AUTHORSHIP_MD, FOOTER as RIGHTS_FOOTER, PATH as AUTHORSHIP_PATH};
const AUTHORSHIP_TITLE: &str = "Authorship and rights";
const AUTHORSHIP_BLURB: &str = "Who authored a program on BeatScript and what rights it has: CC0, prompts and authorship, lineage, private work. Plain language, not legal advice.";

/// The schema page's terms as fragments: each `<a id="T"></a>**T**:` paragraph of SCHEMA.md (its raw anchor is dropped
/// by the renderer, as all raw HTML is) gets `id="T"`, so `https://beatscript.app/schema#T` lands on its definition.
/// The terms are the block's context's and the BeatScript context's (`/context/beatscript-v1.jsonld`: one vocabulary).
fn term_anchors(html: &str) -> String {
    let mut out = html.to_string();
    let ctx = djlive::progld::schema_context();
    let mut terms: Vec<String> = match ctx.get("@context") {
        Some(djengine::json::JsonValue::Object(o)) => o.iter().map(|(k, _)| k.to_string()).filter(|k| !k.starts_with('@')).collect(),
        _ => Vec::new(),
    };
    let bs: serde_json::Value = serde_json::from_slice(crate::public::CONTEXT_BEATSCRIPT_V1).unwrap_or_default();
    for k in bs["@context"].as_object().into_iter().flat_map(|o| o.keys()) {
        if !k.starts_with('@') && !terms.contains(k) {
            terms.push(k.clone());
        }
    }
    terms.push("ProgramDump".into());
    for t in terms {
        // `<a id="T"></a>` is two pieces of inline HTML, each omitted
        let from = format!("<p><!-- raw HTML omitted --><!-- raw HTML omitted --><strong>{t}</strong>");
        out = out.replacen(&from, &format!("<p id=\"{t}\"><strong>{t}</strong>"), 1);
    }
    out
}

/// The request asks for JSON-LD or JSON before HTML: an `Accept` naming `application/ld+json` or `application/json`
/// with no `text/html` ahead of it.
fn wants_ld(headers: &HeaderMap) -> bool {
    let a = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok()).unwrap_or("").to_ascii_lowercase();
    let pos = |m: &str| a.find(m);
    let ld = [pos("application/ld+json"), pos("application/json")].into_iter().flatten().min();
    match (ld, pos("text/html")) {
        (Some(l), Some(h)) => l < h,
        (Some(_), None) => true,
        _ => false,
    }
}

struct Page {
    slug: &'static str,
    raw: &'static str,
    title: &'static str,
    blurb: &'static str,
}

const PAGES: [Page; 4] = [
    Page { slug: "cheatsheet", raw: "/docs/cheatsheet.md", title: "Cheat sheet", blurb: "The language on one page: the compact grammar, the same text the compose prompt uses. Read this first." },
    Page { slug: "language", raw: "/docs/BEATSCRIPT.md", title: "Language reference", blurb: "Grammar (EBNF), mini-notation, control lines, evolve, canonical form and share links." },
    Page { slug: "arrange", raw: "/docs/arrange.md", title: "Arrange mutations", blurb: "`arrange { }`: mutations between sections that rewrite patterns as the song goes, deterministic and seeded by the program's CID." },
    Page { slug: "api", raw: "/docs/api.md", title: "API reference", blurb: "Every HTTP route, sign-in and tokens, compose, render streaming, sessions and the MCP tools, with examples." },
];

/// JSON safe inside a `<script>` element: `<`, `>` and `&` written as `<` `>` `&` (they only occur
/// inside strings, so it is the same JSON).
fn script_json(v: &serde_json::Value) -> String {
    v.to_string().replace('<', "\\u003c").replace('>', "\\u003e").replace('&', "\\u0026")
}

/// Go's `html/template` escaper (text, RCDATA and attribute contexts alike): `"` `&` `'` `+` `<` `>` and NUL.
fn tmpl(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => o.push_str("&#34;"),
            '&' => o.push_str("&amp;"),
            '\'' => o.push_str("&#39;"),
            '+' => o.push_str("&#43;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '\0' => o.push('\u{FFFD}'),
            c => o.push(c),
        }
    }
    o
}

/// goldmark's `util.EscapeHTML`: `"` `&` `<` `>`.
fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => o.push_str("&quot;"),
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            c => o.push(c),
        }
    }
    o
}

/// goldmark's `util.URLEscape(v, false)` then `EscapeHTML`: percent-encode what is not URL-safe, keep `%XX`.
fn esc_url(u: &str) -> String {
    let mut o = String::new();
    let b = u.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'%' && i + 2 < b.len() && b[i + 1].is_ascii_hexdigit() && b[i + 2].is_ascii_hexdigit() {
            o.push_str(&u[i..i + 3]);
            i += 3;
            continue;
        }
        if c.is_ascii_alphanumeric() || b"-._~:/?#[]@!$&'()*+,;=".contains(&c) {
            o.push(c as char);
        } else {
            o.push_str(&format!("%{c:02X}"));
        }
        i += 1;
    }
    esc(&o)
}

/// goldmark's default heading ids (`parser.WithAutoHeadingID`): ASCII letters and digits lower-cased, space `-` `_`
/// as `-`, everything else (non-ASCII too) dropped, "heading" when empty, `-1`, `-2`, ... on a repeat.
fn heading_id(text: &str, seen: &mut HashMap<String, ()>) -> String {
    let mut r = String::new();
    for c in text.trim().chars() {
        if !c.is_ascii() {
            continue;
        }
        if c.is_ascii_alphanumeric() {
            r.push(c.to_ascii_lowercase());
        } else if c.is_ascii_whitespace() || c == '-' || c == '_' {
            r.push('-');
        }
    }
    if r.is_empty() {
        r = "heading".into();
    }
    if seen.insert(r.clone(), ()).is_none() {
        return r;
    }
    let mut i = 1;
    loop {
        let n = format!("{r}-{i}");
        if seen.insert(n.clone(), ()).is_none() {
            return n;
        }
        i += 1;
    }
}

/// `^(?:https?://[^/#?]+)?/docs/(BEATSCRIPT|BTW|cheatsheet|api|arrange)\.md(#[^\s]*)?$` → `/docs/<slug><#frag>` (BTW.md is the
/// language reference's name before the rename).
fn rewrite_doc_link(dest: &str) -> Option<String> {
    let mut rest = dest;
    for scheme in ["https://", "http://"] {
        if let Some(r) = rest.strip_prefix(scheme) {
            let end = r.find(['/', '#', '?']).unwrap_or(r.len());
            if end == 0 {
                return None;
            }
            rest = &r[end..];
            break;
        }
    }
    let r = rest.strip_prefix("/docs/")?;
    let (file, tail) = r.split_once(".md")?;
    let slug = match file {
        "BEATSCRIPT" | "BTW" => "language",
        "cheatsheet" => "cheatsheet",
        "api" => "api",
        "arrange" => "arrange",
        _ => return None,
    };
    if !(tail.is_empty() || (tail.starts_with('#') && !tail.chars().any(char::is_whitespace))) {
        return None;
    }
    Some(format!("/docs/{slug}{tail}"))
}

/// `(?m)^\s*(?:[^\s#]+\s+)?(?:deck\s+[A-Za-z0-9_]+\s*:|tempo\s+\d)`: the block is a whole program.
fn is_program(code: &str) -> bool {
    fn ws(s: &str) -> &str {
        s.trim_start_matches(|c: char| c.is_ascii_whitespace())
    }
    fn tail_ok(s: &str) -> bool {
        if let Some(r) = s.strip_prefix("deck") {
            let r2 = ws(r);
            if r2.len() == r.len() {
                return false;
            }
            let n = r2.len() - r2.trim_start_matches(|c: char| c.is_ascii_alphanumeric() || c == '_').len();
            return n > 0 && ws(&r2[n..]).starts_with(':');
        }
        if let Some(r) = s.strip_prefix("tempo") {
            let r2 = ws(r);
            return r2.len() < r.len() && r2.starts_with(|c: char| c.is_ascii_digit());
        }
        false
    }
    // (?m)^ with \s* able to span lines: any line start, then leading whitespace (newlines too)
    let starts = std::iter::once(0).chain(code.match_indices('\n').map(|(i, _)| i + 1));
    for st in starts {
        let s = ws(&code[st..]);
        if tail_ok(s) {
            return true;
        }
        // one optional word that has no whitespace and no '#', then whitespace
        let w = s.len() - s.trim_start_matches(|c: char| !c.is_ascii_whitespace() && c != '#').len();
        if w > 0 {
            let after = &s[w..];
            let a2 = ws(after);
            if a2.len() < after.len() && tail_ok(a2) {
                return true;
            }
        }
    }
    false
}

fn is_beatscript(lang: &str, code: &str) -> (bool, bool) {
    match lang.to_lowercase().as_str() {
        "btw" | "beatscript" => (true, true),
        "" | "text" | "txt" => {
            let p = is_program(code);
            (p, p)
        }
        _ => (false, false),
    }
}

struct Rendered {
    body: String,
    toc: Vec<(u8, String, String)>,
}

fn level(l: HeadingLevel) -> u8 {
    l as u8
}

/// The heading's text as goldmark's `Node.Text` gives it: text, code spans and the text inside links/emphasis.
fn collect_text(evs: &[Event]) -> String {
    let mut t = String::new();
    for e in evs {
        match e {
            Event::Text(s) | Event::Code(s) => t.push_str(s),
            _ => {}
        }
    }
    t
}

/// The heading's text as [`collect_text`] gives it, with each code span between [`CODE_OPEN`] and [`CODE_CLOSE`], so the
/// table of contents shows a heading's `btw test` as code, as the heading does.
fn collect_marked(evs: &[Event]) -> String {
    let mut t = String::new();
    for e in evs {
        match e {
            Event::Text(s) => t.push_str(s),
            Event::Code(s) => {
                t.push(CODE_OPEN);
                t.push_str(s);
                t.push(CODE_CLOSE);
            }
            _ => {}
        }
    }
    t
}

const CODE_OPEN: char = '\u{1}';
const CODE_CLOSE: char = '\u{2}';

/// Markdown to the body HTML (goldmark's output) and the table of contents (h2, h3).
fn render_markdown(src: &str) -> Rendered {
    let evs: Vec<Event> = Parser::new_ext(src, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH).collect();
    let mut seen = HashMap::new();
    let mut out = String::new();
    let mut toc = Vec::new();
    // pulldown already decides tightness: a tight item's text comes without a Paragraph (goldmark's text block)
    let mut table_align: Vec<Alignment> = Vec::new();
    let mut in_head = false;
    let mut cell = 0usize;
    let mut i = 0;
    // a tight item's text block: "\n" after it when something follows inside the item
    let mut pending_tb = false;
    while i < evs.len() {
        let e = &evs[i];
        // a text block of a tight list item ends where a block starts or the item ends
        let flush_tb = |out: &mut String, pending: &mut bool, more: bool| {
            if *pending {
                if more {
                    out.push('\n');
                }
                *pending = false;
            }
        };
        match e {
            Event::Start(Tag::Heading { level: l, .. }) => {
                flush_tb(&mut out, &mut pending_tb, true);
                let j = (i..evs.len()).find(|&j| matches!(evs[j], Event::End(TagEnd::Heading(_)))).unwrap_or(evs.len());
                let text = collect_text(&evs[i + 1..j]);
                let id = heading_id(&text, &mut seen);
                let lv = level(*l);
                if lv == 2 || lv == 3 {
                    // the entry keeps the heading's code spans as code (marked here, made <code> by the shell)
                    toc.push((lv, id.clone(), collect_marked(&evs[i + 1..j]).trim().to_string()));
                }
                out.push_str(&format!("<h{lv} id=\"{}\"><a class=\"anchor\" href=\"#{}\" aria-label=\"Link to this section\">#</a>", esc(&id), esc(&id)));
            }
            Event::End(TagEnd::Heading(l)) => out.push_str(&format!("</h{}>\n", level(*l))),
            Event::Start(Tag::Paragraph) => {
                flush_tb(&mut out, &mut pending_tb, true);
                out.push_str("<p>");
            }
            Event::End(TagEnd::Paragraph) => out.push_str("</p>\n"),
            Event::Start(Tag::BlockQuote(_)) => {
                flush_tb(&mut out, &mut pending_tb, true);
                out.push_str("<blockquote>\n");
            }
            Event::End(TagEnd::BlockQuote(_)) => out.push_str("</blockquote>\n"),
            Event::Start(Tag::CodeBlock(kind)) => {
                flush_tb(&mut out, &mut pending_tb, true);
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => info.split_whitespace().next().unwrap_or("").to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                let mut code = String::new();
                i += 1;
                while i < evs.len() && !matches!(evs[i], Event::End(TagEnd::CodeBlock)) {
                    if let Event::Text(t) = &evs[i] {
                        code.push_str(t);
                    }
                    i += 1;
                }
                let (hl, tr) = is_beatscript(&lang, &code);
                let mut label = lang.clone();
                if hl && (lang.is_empty() || ["btw", "text", "txt", "beatscript"].iter().any(|x| lang.eq_ignore_ascii_case(x))) {
                    label = "BeatScript".into();
                }
                let mut attrs = String::new();
                if hl {
                    attrs.push_str(" data-beatscript=\"1\"");
                }
                if tr {
                    attrs.push_str(" data-try=\"1\"");
                }
                let cls = if lang.is_empty() { String::new() } else { format!(" class=\"language-{}\"", html_escape(&lang)) };
                out.push_str(&format!("<figure class=\"code\"{attrs}><figcaption><span class=\"lang\">{}</span><span class=\"acts\"><button type=\"button\" class=\"cp\" hidden>Copy</button>", html_escape(&label)));
                if tr {
                    out.push_str("<button type=\"button\" class=\"try\" hidden>Try it</button>");
                }
                out.push_str(&format!("</span></figcaption><pre><code{cls}>{}</code></pre></figure>\n", esc(&code)));
            }
            Event::Start(Tag::List(start)) => {
                flush_tb(&mut out, &mut pending_tb, true);
                match start {
                    Some(1) => out.push_str("<ol>\n"),
                    Some(n) => out.push_str(&format!("<ol start=\"{n}\">\n")),
                    None => out.push_str("<ul>\n"),
                }
            }
            Event::End(TagEnd::List(ordered)) => {
                out.push_str(if *ordered { "</ol>\n" } else { "</ul>\n" });
            }
            Event::Start(Tag::Item) => {
                out.push_str("<li>");
                // goldmark: a newline unless the first child is a text block (a tight item's text) or there is none
                let empty = matches!(evs.get(i + 1), Some(Event::End(TagEnd::Item)));
                let inline_first = !matches!(evs.get(i + 1), Some(Event::Start(_))) || matches!(evs.get(i + 1), Some(Event::Start(t)) if is_inline(t));
                if !empty && !inline_first {
                    out.push('\n');
                }
                if inline_first && !empty {
                    // pulldown gives a tight item's text without a Paragraph: it is goldmark's text block
                    i += 1;
                    let start = i;
                    while i < evs.len() && !is_block_boundary(&evs[i]) {
                        i += 1;
                    }
                    inline(&evs[start..i], &mut out);
                    pending_tb = true;
                    continue;
                }
            }
            Event::End(TagEnd::Item) => {
                pending_tb = false;
                out.push_str("</li>\n");
            }
            Event::Rule => {
                flush_tb(&mut out, &mut pending_tb, true);
                out.push_str("<hr>\n");
            }
            Event::Html(_) => {
                flush_tb(&mut out, &mut pending_tb, true);
                // one comment per HTML block (pulldown gives a block's lines as several events)
                if !matches!(evs.get(i.wrapping_sub(1)), Some(Event::Html(_))) {
                    out.push_str("<!-- raw HTML omitted -->\n");
                }
            }
            Event::Start(Tag::Table(al)) => {
                flush_tb(&mut out, &mut pending_tb, true);
                table_align = al.clone();
                out.push_str("<table>\n");
            }
            Event::End(TagEnd::Table) => {
                if !in_head {
                    out.push_str("</tbody>\n");
                }
                out.push_str("</table>\n");
            }
            Event::Start(Tag::TableHead) => {
                in_head = true;
                cell = 0;
                out.push_str("<thead>\n<tr>\n");
            }
            Event::End(TagEnd::TableHead) => {
                out.push_str("</tr>\n</thead>\n");
                // goldmark writes <tbody> only when there are body rows
                if matches!(evs.get(i + 1), Some(Event::Start(Tag::TableRow))) {
                    in_head = false;
                    out.push_str("<tbody>\n");
                }
            }
            Event::Start(Tag::TableRow) => {
                cell = 0;
                out.push_str("<tr>\n");
            }
            Event::End(TagEnd::TableRow) => out.push_str("</tr>\n"),
            Event::Start(Tag::TableCell) => {
                let t = if in_head { "th" } else { "td" };
                let a = match table_align.get(cell) {
                    // classes, not style= (the site's CSP is style-src 'self'); docs.css aligns them
                    Some(Alignment::Left) => " class=\"align-left\"",
                    Some(Alignment::Center) => " class=\"align-center\"",
                    Some(Alignment::Right) => " class=\"align-right\"",
                    _ => "",
                };
                out.push_str(&format!("<{t}{a}>"));
                i += 1;
                let start = i;
                while i < evs.len() && !matches!(evs[i], Event::End(TagEnd::TableCell)) {
                    i += 1;
                }
                inline(&evs[start..i], &mut out);
                out.push_str(&format!("</{t}>\n"));
                cell += 1;
            }
            _ => {
                // inline run (a paragraph's content)
                let start = i;
                while i < evs.len() && !is_block_boundary(&evs[i]) {
                    i += 1;
                }
                inline(&evs[start..i.max(start + 1)], &mut out);
                if i == start {
                    i += 1;
                }
                continue;
            }
        }
        i += 1;
    }
    Rendered { body: out, toc }
}

fn is_inline(t: &Tag) -> bool {
    matches!(t, Tag::Emphasis | Tag::Strong | Tag::Strikethrough | Tag::Link { .. } | Tag::Image { .. })
}

fn is_block_boundary(e: &Event) -> bool {
    match e {
        Event::Start(t) => !is_inline(t),
        Event::End(t) => !matches!(t, TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link | TagEnd::Image),
        Event::Rule | Event::Html(_) => true,
        _ => false,
    }
}

fn inline(evs: &[Event], out: &mut String) {
    let mut k = 0;
    while k < evs.len() {
        match &evs[k] {
            Event::Text(t) => out.push_str(&esc(t)),
            Event::Code(t) => out.push_str(&format!("<code>{}</code>", esc(t))),
            Event::SoftBreak | Event::HardBreak => out.push_str("<br>\n"),
            Event::InlineHtml(_) | Event::Html(_) => out.push_str("<!-- raw HTML omitted -->"),
            Event::Start(Tag::Emphasis) => out.push_str("<em>"),
            Event::End(TagEnd::Emphasis) => out.push_str("</em>"),
            Event::Start(Tag::Strong) => out.push_str("<strong>"),
            Event::End(TagEnd::Strong) => out.push_str("</strong>"),
            Event::Start(Tag::Strikethrough) => out.push_str("<del>"),
            Event::End(TagEnd::Strikethrough) => out.push_str("</del>"),
            Event::Start(Tag::Link { dest_url, title, .. }) => {
                let d = rewrite_doc_link(dest_url).unwrap_or_else(|| dest_url.to_string());
                let lower = d.to_ascii_lowercase();
                // goldmark drops dangerous destinations (javascript:, vbscript:, file:, data: but images)
                let safe = !["javascript:", "vbscript:", "file:", "data:"].iter().any(|p| lower.starts_with(p));
                out.push_str("<a href=\"");
                if safe {
                    out.push_str(&esc_url(&d));
                }
                out.push('"');
                if !title.is_empty() {
                    out.push_str(&format!(" title=\"{}\"", esc(title)));
                }
                out.push('>');
            }
            Event::End(TagEnd::Link) => out.push_str("</a>"),
            Event::Start(Tag::Image { dest_url, title, .. }) => {
                let mut alt = String::new();
                k += 1;
                while k < evs.len() && !matches!(evs[k], Event::End(TagEnd::Image)) {
                    if let Event::Text(t) | Event::Code(t) = &evs[k] {
                        alt.push_str(t);
                    }
                    k += 1;
                }
                out.push_str(&format!("<img src=\"{}\" alt=\"{}\"", esc_url(dest_url), esc(&alt)));
                if !title.is_empty() {
                    out.push_str(&format!(" title=\"{}\"", esc(title)));
                }
                out.push('>');
            }
            _ => {}
        }
        k += 1;
    }
}

// ---------- the page shell (html.go's template) ----------

fn shell(title: &str, blurb: &str, canonical: &str, raw: &str, slug: &str, nav: &[&Page], toc: &[(u8, String, String)], body: &str) -> String {
    let mut s = String::new();
    s.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1, viewport-fit=cover\">\n<meta name=\"theme-color\" content=\"#16213e\">\n<meta name=\"color-scheme\" content=\"dark\">\n");
    s.push_str(&format!("<title>{} · BeatScript docs</title>\n<meta name=\"description\" content=\"{}\">\n<link rel=\"canonical\" href=\"{}\">\n", tmpl(title), tmpl(blurb), tmpl(canonical)));
    if !raw.is_empty() {
        s.push_str(&format!("<link rel=\"alternate\" type=\"text/markdown\" href=\"{}\" title=\"{} (markdown)\">\n", tmpl(raw), tmpl(title)));
    }
    s.push_str("<link rel=\"icon\" type=\"image/svg+xml\" href=\"/favicon.svg\">\n<link rel=\"stylesheet\" href=\"/docs/assets/docs.css\">\n</head>\n<body>\n<a class=\"skip\" href=\"#main\">Skip to the content</a>\n<header class=\"top\">\n  <a class=\"brand\" href=\"/\" aria-label=\"BeatScript editor\"><span class=\"brand-mark\" aria-hidden=\"true\"></span><span class=\"brand-name\">BeatScript</span></a>\n  <nav class=\"crumbs\" aria-label=\"Docs\">\n");
    s.push_str(&format!("    <a href=\"/docs/\"{}>Docs</a>\n    ", if slug.is_empty() { " aria-current=\"page\"" } else { "" }));
    for p in nav {
        s.push_str(&format!("<a href=\"/docs/{}\"{}>{}</a>\n    ", p.slug, if p.slug == slug { " aria-current=\"page\"" } else { "" }, tmpl(p.title)));
    }
    s.push_str("\n  </nav>\n  <span class=\"spacer\"></span>\n  <a class=\"tb\" href=\"/\" title=\"Back to the editor\"><span class=\"gl-ic\" aria-hidden=\"true\">←</span><span class=\"lbl\">Editor</span></a>\n</header>\n<div class=\"layout\">\n");
    if !toc.is_empty() {
        s.push_str("<nav class=\"toc\" aria-label=\"On this page\"><p class=\"toc-title\">On this page</p>\n<ul>\n");
        for (l, id, t) in toc {
            let t = tmpl(t).replace(CODE_OPEN, "<code>").replace(CODE_CLOSE, "</code>");
            s.push_str(&format!("<li class=\"l{l}\"><a href=\"#{}\">{t}</a></li>\n", tmpl(id)));
        }
        s.push_str("</ul>\n</nav>\n");
    }
    s.push_str("<main id=\"main\" class=\"doc\">\n");
    s.push_str(body);
    s.push('\n');
    if !raw.is_empty() {
        s.push_str(&format!("<p class=\"src\"><a href=\"{}\" rel=\"alternate\" type=\"text/markdown\">View source</a> (the raw markdown of this page, for models and scripts)</p>", tmpl(raw)));
    }
    s.push_str("\n</main>\n</div>\n");
    s.push_str(RIGHTS_FOOTER);
    s.push_str("<script type=\"module\" src=\"/docs/assets/docs.js\"></script>\n</body>\n</html>\n");
    s
}

// ---------- the server ----------

/// The answer to one docs request.
pub enum Answer {
    Body(&'static str, Bytes, Option<String>, bool),
    NotInstalled,
    Error(&'static str),
}

impl Docs {
    pub fn new(docs_dir: Option<PathBuf>, src_dir: Option<PathBuf>, public_url: &str) -> Docs {
        Docs {
            docs_dir,
            src_dir,
            public: public_url.trim_end_matches('/').to_string(),
            full_example: "markers-demo".into(),
            cs: Mutex::new(None),
            cache: Mutex::new(HashMap::new()),
        }
    }

    fn read_doc(&self, name: &str) -> Option<String> {
        let b = std::fs::read(self.docs_dir.as_ref()?.join(name)).ok()?;
        let s = String::from_utf8_lossy(&b).into_owned();
        if s.trim().is_empty() {
            None
        } else {
            Some(s)
        }
    }

    fn src(&self, rel: &str) -> Option<Vec<u8>> {
        std::fs::read(self.src_dir.as_ref()?.join(rel)).ok()
    }

    /// api.md with the site's origin filled in (Go: at Mount). The file writes https://beatscript.app (older copies
    /// https://btw.bitwrap.io); either becomes this server's -public-url.
    fn api_md(&self) -> Option<String> {
        let b = self.src("api.md")?;
        Some(String::from_utf8_lossy(&b).replace("https://btw.bitwrap.io", &self.public).replace("https://beatscript.app", &self.public))
    }

    /// `-docs`/cheatsheet.md, else the engine's own (`beatscript_api::cheatsheet`, cached ten minutes as Go caches its beatscript's).
    fn cheatsheet(&self) -> Option<String> {
        if let Some(s) = self.read_doc("cheatsheet.md") {
            return Some(s);
        }
        let mut c = self.cs.lock().unwrap();
        if let Some((s, at)) = c.as_ref() {
            if at.elapsed() < Duration::from_secs(600) {
                return Some(s.clone());
            }
        }
        let s = beatscript_api::cheatsheet();
        if s.trim().is_empty() {
            return None;
        }
        *c = Some((s.clone(), Instant::now()));
        Some(s)
    }

    fn page_src(&self, slug: &str) -> Option<String> {
        match slug {
            "cheatsheet" => self.cheatsheet(),
            "language" => self.read_doc("BEATSCRIPT.md"),
            "arrange" => self.read_doc("arrange.md"),
            "api" => self.api_md(),
            _ => None,
        }
    }

    fn installed(&self) -> Vec<&'static Page> {
        PAGES.iter().filter(|p| self.page_src(p.slug).is_some()).collect()
    }

    fn render_page(&self, p: &Page) -> Option<(Bytes, String)> {
        let src = self.page_src(p.slug)?;
        let key = content::sha_hex(format!("{}\0{}\0{}", p.slug, self.public, src).as_bytes(), 32);
        if let Some((k, b, e)) = self.cache.lock().unwrap().get(p.slug) {
            if *k == key {
                return Some((b.clone(), e.clone()));
            }
        }
        let r = render_markdown(&src);
        let nav = self.installed();
        let html = shell(p.title, p.blurb, &format!("{}/docs/{}", self.public, p.slug), p.raw, p.slug, &nav, &r.toc, &r.body);
        let b = Bytes::from(html);
        let etag = format!("\"{}\"", &key[..16]);
        self.cache.lock().unwrap().insert(p.slug.to_string(), (key, b.clone(), etag.clone()));
        Some((b, etag))
    }

    /// The docs as linked data (schema.org), on the index page: a `CollectionPage` whose parts are every page it lists,
    /// each with its raw form, the Grammar page (the `DefinedTermSet` that page describes itself as, by the same `@id`)
    /// with grammar.ebnf, the schema, the open data and llms.txt.
    fn index_ld(&self, nav: &[&Page]) -> serde_json::Value {
        let p = self.public.trim_end_matches('/');
        let mut parts: Vec<serde_json::Value> = nav
            .iter()
            .map(|pg| json!({"@type": "TechArticle", "@id": format!("{p}/docs/{}", pg.slug), "name": pg.title, "description": pg.blurb, "url": format!("{p}/docs/{}", pg.slug), "encoding": {"@type": "MediaObject", "contentUrl": format!("{p}{}", pg.raw), "encodingFormat": "text/markdown"}}))
            .collect();
        parts.push(json!({"@type": "DefinedTermSet", "@id": format!("{p}/grammar"), "name": "The grammar of BeatScript", "description": "Every production of the language as a railroad diagram, generated from grammar.ebnf, its one grammar.", "url": format!("{p}/grammar"), "encoding": {"@type": "MediaObject", "name": "grammar.ebnf", "contentUrl": format!("{p}/grammar.ebnf"), "encodingFormat": "text/plain"}}));
        parts.push(json!({"@type": "TechArticle", "@id": format!("{p}/schema"), "name": "BeatScript schema", "url": format!("{p}/schema"), "encoding": {"@type": "MediaObject", "contentUrl": format!("{p}/schema.jsonld"), "encodingFormat": "application/ld+json"}}));
        parts.push(json!({"@type": "TechArticle", "@id": format!("{p}/docs/open-data"), "name": "Open data", "url": format!("{p}/docs/open-data"), "encoding": {"@type": "MediaObject", "contentUrl": format!("{p}/docs/open-data.md"), "encodingFormat": "text/markdown"}}));
        parts.push(json!({"@type": "TechArticle", "@id": format!("{p}{FOUND_PATH}"), "name": FOUND_TITLE, "description": FOUND_BLURB, "url": format!("{p}{FOUND_PATH}"), "encoding": {"@type": "MediaObject", "contentUrl": format!("{p}{FOUND_PATH}.md"), "encodingFormat": "text/markdown"}}));
        parts.push(json!({"@type": "TechArticle", "@id": format!("{p}{FORMS_PATH}"), "name": FORMS_TITLE, "description": FORMS_BLURB, "url": format!("{p}{FORMS_PATH}"), "encoding": {"@type": "MediaObject", "contentUrl": format!("{p}{FORMS_PATH}.md"), "encodingFormat": "text/markdown"}}));
        parts.push(json!({"@type": "TechArticle", "@id": format!("{p}{RADIO_PATH}"), "name": RADIO_TITLE, "description": RADIO_BLURB, "url": format!("{p}{RADIO_PATH}"), "encoding": {"@type": "MediaObject", "contentUrl": format!("{p}{RADIO_PATH}.md"), "encodingFormat": "text/markdown"}}));
        parts.push(json!({"@type": "TechArticle", "@id": format!("{p}{POLISH_PATH}"), "name": POLISH_TITLE, "description": POLISH_BLURB, "url": format!("{p}{POLISH_PATH}"), "encoding": {"@type": "MediaObject", "contentUrl": format!("{p}{POLISH_PATH}.md"), "encodingFormat": "text/markdown"}}));
        parts.push(json!({"@type": "TechArticle", "@id": format!("{p}{EMBED_PATH}"), "name": EMBED_TITLE, "description": EMBED_BLURB, "url": format!("{p}{EMBED_PATH}"), "encoding": {"@type": "MediaObject", "contentUrl": format!("{p}{EMBED_PATH}.md"), "encodingFormat": "text/markdown"}}));
        parts.push(json!({"@type": "TechArticle", "@id": format!("{p}{AUTHORSHIP_PATH}"), "name": AUTHORSHIP_TITLE, "description": AUTHORSHIP_BLURB, "url": format!("{p}{AUTHORSHIP_PATH}"), "encoding": {"@type": "MediaObject", "contentUrl": format!("{p}{AUTHORSHIP_PATH}.md"), "encodingFormat": "text/markdown"}}));
        parts.push(json!({"@type": "Dataset", "@id": format!("{p}{}", beatscript_serve::themes::PATH), "name": beatscript_serve::themes::TITLE, "description": beatscript_serve::themes::BLURB, "url": format!("{p}{}", beatscript_serve::themes::PATH), "encoding": {"@type": "MediaObject", "contentUrl": format!("{p}/themes.txt"), "encodingFormat": "text/plain"}}));
        parts.push(json!({"@type": "Dataset", "@id": format!("{p}{}", beatscript_serve::phrasebook::PATH), "name": beatscript_serve::phrasebook::TITLE, "description": beatscript_serve::phrasebook::BLURB, "url": format!("{p}{}", beatscript_serve::phrasebook::PATH), "encoding": {"@type": "MediaObject", "contentUrl": format!("{p}/phrasebook.txt"), "encodingFormat": "text/plain"}}));
        parts.push(json!({"@type": "MediaObject", "@id": format!("{p}/llms.txt"), "name": "llms.txt", "contentUrl": format!("{p}/llms.txt"), "encodingFormat": "text/plain"}));
        json!({"@context": "https://schema.org", "@type": "CollectionPage", "@id": format!("{p}/docs/"), "name": "BeatScript docs", "url": format!("{p}/docs/"), "isPartOf": {"@type": "WebSite", "name": "BeatScript", "url": format!("{p}/"), "isAccessibleForFree": true, "codeRepository": beatscript_serve::rights::SOURCE_URL, "license": "https://opensource.org/licenses/MIT"}, "hasPart": parts})
    }

    fn render_index(&self) -> Bytes {
        let mut b = String::from("<h1>BeatScript docs</h1>\n<p>BeatScript is a small, total, deterministic language for writing whole music tracks as text. These are the pages that teach it. Every page has a raw markdown file for models and scripts.</p>\n<ul class=\"cards\">\n");
        let nav = self.installed();
        for p in &nav {
            b.push_str(&format!("<li><a href=\"/docs/{}\"><strong>{}</strong></a><span>{}</span> <a class=\"raw\" href=\"{}\">markdown</a></li>\n", p.slug, html_escape(p.title), html_escape(p.blurb), p.raw));
        }
        b.push_str("</ul>\n<h2 id=\"more\"><a class=\"anchor\" href=\"#more\" aria-label=\"Link to this section\">#</a>More</h2>\n<ul>\n");
        b.push_str("<li><a href=\"/api/instruments\">Instruments</a>: the voices a program may name (JSON)</li>\n");
        b.push_str("<li><a href=\"/api/examples\">Examples</a>: the public example programs (JSON); open one from the editor's examples menu</li>\n");
        b.push_str("<li><a href=\"/grammar\">Grammar</a>: every production of the language as a railroad diagram, generated from <a href=\"/grammar.ebnf\">grammar.ebnf</a></li>\n");
        b.push_str("<li><a href=\"/docs/forms\">Forms</a>: arrange shapes from classical music (quodlibet, passacaglia, canon, rondo, fugue, theme and variations), each a template built from public-domain themes</li>\n");
        b.push_str("<li><a href=\"/themes\">Theme library</a>: melodies, basses and figures composed and published before 1929, CC0, as BeatScript bars in a stated key, to borrow and morph</li>\n");
        b.push_str("<li><a href=\"/docs/found-phrases\">Found phrases</a>: borrowing public-domain text for its shape: a line chosen for its meter, matched to the groove, buried and spotlit. The technique behind the phrasebook and the phrase templates</li>\n");
        b.push_str("<li><a href=\"/docs/radio\">Radio: stations</a>: endless, deterministic stations, each one small program: position N is always the same track, and a knob turned moves you to a neighbouring station at the same position</li>\n");
        b.push_str("<li><a href=\"/docs/polish\">Polish a mix</a>: the listen loop: check, listen, change one line, listen again with compare; what listen measures, the delivery targets, the order of fixes and the lines that make them</li>\n");
        b.push_str("<li><a href=\"/docs/embed\">Embed a program</a>: one embed config for every surface: the player and the studio (the editor as an MCP server inside a Claude artifact or any page), iframe, the &lt;beatscript-player&gt; element, oEmbed, link previews that play, a README card, Jupyter, VS Code, plain audio</li>\n");
        b.push_str("<li><a href=\"/authorship\">Authorship and rights</a>: who authored a program and what rights it has, in plain language</li>\n");
        b.push_str("<li><a href=\"/phrasebook\">Phrasebook</a>: lines of verse, scripture, speech and work song first published before 1929, CC0, each with its meter as a step pattern, to sing by id</li>\n");
        b.push_str("<li><a href=\"/llms.txt\">llms.txt</a>: this site, indexed for a language model</li>\n");
        b.push_str("<li><a href=\"/ontology/\">Ontology</a>: the BeatScript ontology, the vocabulary behind BeatScript (genres, grooves, rhythms, sections, the engine, the language's own terms), as Turtle, JSON-LD, SPARQL and MCP. Derived from the bitwrap ontology 0.2.0, it evolves on its own</li>\n");
        b.push_str("<li><a href=\"/\">The editor</a></li>\n</ul>\n");
        b.push_str(&format!("<script type=\"application/ld+json\">{}</script>\n", script_json(&self.index_ld(&nav))));
        Bytes::from(shell("Docs", "BeatScript docs: the cheat sheet, the language reference and the API reference.", &format!("{}/docs/", self.public), "", "", &nav, &[], &b))
    }

    /// The answer for `path`, None when it is not a docs route.
    pub fn answer(&self, path: &str, ex: &Examples) -> Option<Answer> {
        const TXT: &str = "text/plain; charset=utf-8";
        const MD: &str = "text/markdown; charset=utf-8";
        const HTML: &str = "text/html; charset=utf-8";
        const CSS: &str = "text/css; charset=utf-8";
        let body = |ct, s: Option<String>| Some(s.map_or(Answer::NotInstalled, |s| Answer::Body(ct, Bytes::from(s), None, true)));
        match path {
            "/llms.txt" => body(TXT, Some(self.llms_txt(ex))),
            "/llms-full.txt" => body(TXT, Some(self.llms_full(ex))),
            // the language reference, and under its name before the rename (the MCP instructions and old llms.txt copies link it)
            "/docs/BEATSCRIPT.md" | "/docs/BTW.md" => body(MD, self.read_doc("BEATSCRIPT.md")),
            "/docs/arrange.md" => body(MD, self.read_doc("arrange.md")),
            "/schema.jsonld" => Some(Answer::Body("application/ld+json; charset=utf-8", Bytes::from(djlive::progld::schema_context().js_stringified()), None, true)),
            "/schema.md" => body(MD, Some(SCHEMA_MD.to_string())),
            "/docs/open-data.md" => body(MD, Some(OPEN_DATA_MD.to_string())),
            "/docs/found-phrases.md" => body(MD, Some(FOUND_MD.to_string())),
            "/docs/forms.md" => body(MD, Some(FORMS_MD.to_string())),
            "/docs/forms" => {
                let r = render_markdown(FORMS_MD);
                let nav = self.installed();
                let html = shell(FORMS_TITLE, FORMS_BLURB, &format!("{}{FORMS_PATH}", self.public), "/docs/forms.md", "forms", &nav, &r.toc, &r.body);
                Some(Answer::Body(HTML, Bytes::from(html), None, true))
            }
            // the public-domain theme library: one corpus (beatscript_serve::themes), this page one of its views
            "/themes" => {
                let (body, toc) = beatscript_serve::themes::page(&self.public);
                let nav = self.installed();
                let html = shell(beatscript_serve::themes::TITLE, beatscript_serve::themes::BLURB, &beatscript_serve::themes::url(&self.public), "", "themes", &nav, &toc, &body);
                Some(Answer::Body(HTML, Bytes::from(html), None, true))
            }
            "/docs/found-phrases" => {
                let r = render_markdown(FOUND_MD);
                let nav = self.installed();
                let html = shell(FOUND_TITLE, FOUND_BLURB, &format!("{}{FOUND_PATH}", self.public), "/docs/found-phrases.md", "found-phrases", &nav, &r.toc, &r.body);
                Some(Answer::Body(HTML, Bytes::from(html), None, true))
            }
            "/docs/radio.md" => body(MD, Some(RADIO_MD.to_string())),
            "/docs/radio" => {
                let r = render_markdown(RADIO_MD);
                let nav = self.installed();
                let html = shell(RADIO_TITLE, RADIO_BLURB, &format!("{}{RADIO_PATH}", self.public), "/docs/radio.md", "radio", &nav, &r.toc, &r.body);
                Some(Answer::Body(HTML, Bytes::from(html), None, true))
            }
            "/docs/polish.md" => body(MD, Some(POLISH_MD.to_string())),
            "/docs/polish" => {
                let r = render_markdown(POLISH_MD);
                let nav = self.installed();
                let html = shell(POLISH_TITLE, POLISH_BLURB, &format!("{}{POLISH_PATH}", self.public), "/docs/polish.md", "polish", &nav, &r.toc, &r.body);
                Some(Answer::Body(HTML, Bytes::from(html), None, true))
            }
            "/docs/embed.md" => body(MD, Some(EMBED_MD.to_string())),
            "/docs/embed" => {
                let r = render_markdown(EMBED_MD);
                let nav = self.installed();
                let html = shell(EMBED_TITLE, EMBED_BLURB, &format!("{}{EMBED_PATH}", self.public), "/docs/embed.md", "embed", &nav, &r.toc, &r.body);
                Some(Answer::Body(HTML, Bytes::from(html), None, true))
            }
            "/authorship.md" => body(MD, Some(AUTHORSHIP_MD.to_string())),
            "/authorship" => {
                let r = render_markdown(AUTHORSHIP_MD);
                let nav = self.installed();
                let html = shell(AUTHORSHIP_TITLE, AUTHORSHIP_BLURB, &format!("{}{AUTHORSHIP_PATH}", self.public), "/authorship.md", "authorship", &nav, &r.toc, &r.body);
                Some(Answer::Body(HTML, Bytes::from(html), None, true))
            }
            // the public-domain phrasebook: one corpus (beatscript_serve::phrasebook), this page one of its views
            "/phrasebook" => {
                let (body, toc) = beatscript_serve::phrasebook::page(&self.public);
                let nav = self.installed();
                let html = shell(beatscript_serve::phrasebook::TITLE, beatscript_serve::phrasebook::BLURB, &beatscript_serve::phrasebook::url(&self.public), "", "phrasebook", &nav, &toc, &body);
                Some(Answer::Body(HTML, Bytes::from(html), None, true))
            }
            // the grammar (djlive's grammar.ebnf) and its page of railroad diagrams, generated from it
            "/grammar.ebnf" => body(TXT, Some(djlive::grammar::SOURCE.to_string())),
            "/grammar" => Some(Answer::Body(HTML, Bytes::from(djlive::grammar::page::grammar_page(&self.public)), None, true)),
            // the generated pages' stylesheets: the site's CSP is style-src 'self', so no page styles itself inline
            "/grammar.css" => body(CSS, Some(djlive::grammar::page::stylesheet())),
            "/lineage.css" => body(CSS, Some(beatscript_serve::lineage::page::STYLE.to_string())),
            "/mcp.css" => body(CSS, Some(crate::mcp::tools::LANDING_CSS.to_string())),
            "/docs/open-data" => {
                let r = render_markdown(OPEN_DATA_MD);
                let nav = self.installed();
                let html = shell("Open data", "Every public CC0 BeatScript program in every format: per-program URLs, feeds, the sitemap and bulk dumps.", &format!("{}/docs/open-data", self.public), "/docs/open-data.md", "open-data", &nav, &r.toc, &r.body);
                Some(Answer::Body(HTML, Bytes::from(html), None, true))
            }
            "/schema" => {
                let mut r = render_markdown(SCHEMA_MD);
                r.body = term_anchors(&r.body);
                let nav = self.installed();
                let html = shell("BeatScript schema", "The JSON-LD a BeatScript program travels as: the <beatscript-view> block, its terms and context.", &format!("{}/schema", self.public), "/schema.md", "schema", &nav, &r.toc, &r.body);
                Some(Answer::Body(HTML, Bytes::from(html), None, true))
            }
            "/docs/cheatsheet.md" => body(MD, self.cheatsheet()),
            "/docs/api.md" => body(MD, self.api_md()),
            "/docs/assets/docs.css" => Some(self.src("assets/docs.css").map_or(Answer::NotInstalled, |b| Answer::Body("text/css; charset=utf-8", Bytes::from(b), None, true))),
            "/docs/assets/docs.js" => Some(self.src("assets/docs.js").map_or(Answer::NotInstalled, |b| Answer::Body("text/javascript; charset=utf-8", Bytes::from(b), None, true))),
            "/docs/" => {
                let b = self.render_index();
                let tag = format!("\"{}\"", content::sha_hex(&b, 8));
                Some(Answer::Body(HTML, b, Some(tag), false))
            }
            _ => {
                let slug = path.strip_prefix("/docs/")?;
                let p = PAGES.iter().find(|p| p.slug == slug)?;
                if self.page_src(p.slug).is_none() {
                    return Some(Answer::NotInstalled);
                }
                Some(match self.render_page(p) {
                    Some((b, tag)) => Answer::Body(HTML, b, Some(tag), false),
                    None => Answer::Error("could not render this document"),
                })
            }
        }
    }

    /// The HTTP response for a docs route (`serve` / `serveHTML` / `writeCached` / `writeBody`).
    pub fn respond(&self, method: &Method, headers: &HeaderMap, path: &str, ex: &Examples) -> Option<Response<Body>> {
        if !self.is_route(path) {
            return None;
        }
        if method != Method::GET && method != Method::HEAD {
            let mut r = content::text_error(405, "method not allowed");
            r.headers_mut().insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
            return Some(r);
        }
        // `/schema`: the JSON-LD context to a processor (`Accept: application/ld+json` or `application/json`), the page
        // to anyone else; it varies by Accept and says where the other form is
        let schema = path == "/schema";
        let path = if schema && wants_ld(headers) { "/schema.jsonld" } else { path };
        Some(match self.answer(path, ex)? {
            Answer::NotInstalled => content::text_error(404, "this document is not installed on this server"),
            Answer::Error(m) => content::text_error(500, m),
            Answer::Body(ct, b, etag, cors) => {
                let mut h = HeaderMap::new();
                let tag = etag.unwrap_or_else(|| format!("\"{}\"", content::sha_hex(&b, 8)));
                h.insert(header::ETAG, HeaderValue::from_str(&tag).unwrap());
                h.insert(header::CONTENT_TYPE, HeaderValue::from_static(ct));
                h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
                h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
                if cors {
                    h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
                }
                if schema {
                    h.insert(header::VARY, HeaderValue::from_static("Accept"));
                    h.insert(header::LINK, HeaderValue::from_static("</schema.jsonld>; rel=\"alternate\"; type=\"application/ld+json\", </schema.md>; rel=\"alternate\"; type=\"text/markdown\""));
                }
                let n = b.len() as u64;
                content::serve(h, &content::Req { method, headers }, n, Some(b))
            }
        })
    }

    pub fn is_route(&self, path: &str) -> bool {
        matches!(
            path,
            "/llms.txt" | "/llms-full.txt" | "/docs/BEATSCRIPT.md" | "/docs/BTW.md" | "/docs/cheatsheet.md" | "/docs/api.md" | "/docs/assets/docs.css" | "/docs/assets/docs.js" | "/docs/" | "/docs/cheatsheet" | "/docs/language" | "/docs/api" | "/docs/arrange.md" | "/docs/arrange" | "/schema" | "/schema.jsonld" | "/schema.md" | "/docs/open-data" | "/docs/open-data.md" | "/grammar" | "/grammar.ebnf" | "/grammar.css" | "/lineage.css" | "/mcp.css" | "/authorship" | "/authorship.md" | "/phrasebook" | "/docs/found-phrases" | "/docs/found-phrases.md" | "/docs/forms" | "/docs/forms.md" | "/themes"
                | "/docs/embed" | "/docs/embed.md" | "/docs/radio" | "/docs/radio.md" | "/docs/polish" | "/docs/polish.md"
        )
    }

    // ---------- llms.txt ----------

    fn llms_txt(&self, ex: &Examples) -> String {
        let p = &self.public;
        let mut b = String::new();
        b.push_str("# BeatScript\n\n");
        b.push_str("> BeatScript: AI-assisted algorithmic composition. BeatScript is also the name of its small, total, deterministic language for writing whole music tracks as text. Programs compile to Petri nets, play in the browser, in the djrust desktop app and in the BeatScript Android app, and are content-addressed: the same canonical text always has the same CID. Programs are CC0 by default and remixable by fork.\n\n");
        b.push_str("Key facts:\n");
        b.push_str(&format!("- Everything generated here is public: programs, share links, cards and rendered audio are stored publicly under CC0-1.0, and a compose prompt goes to a third-party model. Never put anything private in a prompt. Terms: {p}/privacy.\n"));
        b.push_str(&format!("- Who authored a program and what rights it has (CC0, prompts and authorship, lineage, private work), in plain language: {p}{AUTHORSHIP_PATH}.\n"));
        b.push_str(&format!("- Source: {} (MIT); programs CC0. Feature requests welcome as issues; no outside code or pull requests.\n", beatscript_serve::rights::SOURCE_URL));
        b.push_str("- One line per deck (`deck A: kick x ~ x ~ | snare ~ x ~ x`), parts joined with `|`, 16 steps a bar. Mini-notation: `x` hit, `~` rest, `[..]` group, `<..>` per-bar alternation, `c2!` accent, `e2^` slide, `chord(am)`, `e(5,8)` euclid.\n");
        b.push_str("- Structure: `let`, `def` (no recursion), `repeat N`, `section NAME N bars { ... }`, `arrange`, `end after`, `loop`.\n");
        b.push_str("- Control lines: ramps `a -> b over N bars`, `at bar N:`, `every N bars:`, `before <section> N bars:`, `do <noun> <verb>`. Seeded transforms (`rotate`, `transpose`, `degrade`, `humanize`); `evolve every N bars { ... }` with a reported tail and period.\n");
        b.push_str(&format!("- Only instruments from {p}/api/instruments. Always run check (the MCP tool, or POST {p}/api/check) and fix every line:column error before playing: a program with an error is refused whole.\n"));
        b.push_str(&format!("- Share link: {p}/#<CID>.<base64url(deflate(formatted text))>: the program as `fmt` prints it, its `#!` header lines and comments kept. The CID is of the canonical text, so formatting and comments never change it (they travel unsealed).\n\n"));
        b.push_str("## Docs\n");
        if self.read_doc("BEATSCRIPT.md").is_some() {
            b.push_str(&format!("- [BEATSCRIPT.md]({p}/docs/BEATSCRIPT.md): the language reference: its two halves (structure, the track's own source, and engine calls, pinned and versioned), grammar (EBNF), mini-notation, control lines, evolve, canonical form, share links\n"));
        }
        if self.cheatsheet().is_some() {
            b.push_str(&format!("- [cheatsheet.md]({p}/docs/cheatsheet.md): the compact grammar, the same text the compose prompt uses; read this first\n"));
        }
        b.push_str(&format!("- [Found phrases]({p}{FOUND_PATH}.md): the technique for sung lines: borrow public-domain text for its shape, pick a line for its meter (trochees chant on the kick, iambs lilt on the offbeat, anapests and dactyls want a triplet feel, shanties call and respond), keep the vocal low, cycle it buried and spotlit, cross it onto the drums with xfer, fix pronunciation with phoneme escapes ([page]({p}{FOUND_PATH}))\n"));
        b.push_str(&format!("- [Forms]({p}{FORMS_PATH}.md): arrange shapes from classical music: quodlibet (themes take turns and cross-morph), passacaglia (a fixed ground under cumulative variations), canon (a theme chasing its own copy), rondo (a refrain returning mutated), fugue (a subject re-entering transposed), theme and variations (one source, a transform a section); each a template built from public-domain themes, `arrange from template(quodlibet, @psalm42_deep_calleth)` ([page]({p}{FORMS_PATH}))\n"));
        b.push_str(&format!("- [Radio: stations]({p}{RADIO_PATH}.md): a station is one small program (`station {{ from gen@2 techno seed=42; next seed=~; forms seeded passacaglia }}`) naming an endless, deterministic run of tracks: position N is always the same track (the same CID), the walk steps the seed (`seed=+N` counts on, `seed=~` draws from the CID of the track before), the forms take turns, and the segue from N to N+1 is the cross form of their seeds; a listener turns knobs (genre, seed, step, forms, tempo, key, length) and keeps N; the MCP `station` tool and POST {p}/api/station answer it ([page]({p}{RADIO_PATH}))\n"));
        b.push_str(&format!("- [Polish a mix]({p}{POLISH_PATH}.md): the loop for polishing a mix with the MCP `listen` tool: check, listen, change one line, listen again with `compare` (the previous version's CID or text: deltas, and the warnings fixed and introduced); what it measures per part and on the master (bands, centroid, width, correlation, crest; true peak, PLR, short-term max, loudness range), clashes between parts, delivery `targets` (streaming -14 LUFS / -1 dBTP, apple, broadcast, club), the order of fixes (not heard, clipping, clashes, balance, loudness, width) and the lines that make each one (`level`, `vel=`, `mix {{ B eq low -6 }}`, `duck B under A.kick 6db`, `filter hp`, `drive`), and what the language cannot do yet ([page]({p}{POLISH_PATH}))\n"));
        b.push_str(&format!("- [Embed a program]({p}{EMBED_PATH}.md): one embed config ({p}/embed-config.schema.json) for every surface: <beatscript-track> and <beatscript-studio> (the editor as an MCP server inside a Claude artifact or any page: check, fmt, expand, generate, share and the rest offline, compose and store through the artifact's capabilities), an iframe of {p}/embed/<CID> (?compact=1, ?theme=light), the <beatscript-player cid=…> element ({p}/embed.js), oEmbed ({p}/oembed?url=<a /p/ or /c/ link>&format=json), /c/<CID> cards that play (twitter:player, og:audio), a README card ({p}/card/<CID>.svg), Jupyter, VS Code, Obsidian, and the rendered audio ({p}/api/render/<CID>?format=opus); per target, whether the engine runs and what it gets instead; the MCP embed tool writes each one ([page]({p}{EMBED_PATH}))\n"));
        b.push_str(&format!("- [Instruments]({p}/api/instruments): the voices, families, drum kits and palettes a program may name (JSON)\n"));
        b.push_str(&format!("- [API reference]({p}/docs/api.md): every HTTP route, sign-in and tokens, compose, render streaming, sessions, the MCP tools with examples\n"));
        b.push_str(&format!("- [Ontology]({p}/ontology/): the BeatScript ontology, the vocabulary behind BeatScript (genres, grooves, rhythms, sections, the engine concepts and the language's own grammar, voices, controls and MCP tools) as Turtle, JSON-LD, SPARQL ({p}/ontology/sparql) and a read-only MCP server ({p}/ontology/mcp); term IRIs under {p}/ontology/ns#. Derived from the bitwrap ontology 0.2.0 (each term linked to its bitwrap IRI by prov:wasDerivedFrom), it evolves on its own\n"));
        b.push_str(&format!("- [llms-full.txt]({p}/llms-full.txt): this file with the cheat-sheet, the grammar and an example program inlined\n"));
        b.push_str(&format!("- [Grammar]({p}/grammar): every production as a railroad diagram; [grammar.ebnf]({p}/grammar.ebnf) is the language's one grammar (EBNF), held to the parser both ways\n"));
        b.push_str(&format!("- [Authorship and rights]({p}{AUTHORSHIP_PATH}): who authored a program and what rights it has, in plain language ([markdown]({p}{AUTHORSHIP_PATH}.md))\n\n"));
        b.push_str("## Open data\n");
        b.push_str("Every public CC0 program, in every format, free to fetch (GET, CORS open, no sign-in):\n");
        b.push_str(&format!("- A program by CID: {p}/p/<CID> (its page), {p}/p/<CID>.beatscript (its canonical text, which hashes to the CID), {p}/p/<CID>.jsonld (its linked data), {p}/api/render/<CID>?format=opus (its audio); {p}/p/<CID> also answers by Accept (text/beatscript or text/plain, application/ld+json, audio/*)\n"));
        b.push_str(&format!("- [JSON Feed]({p}/feed.json) and [Atom]({p}/feed.atom): the newest public CC0 programs, each with its .beatscript, .jsonld and render\n"));
        b.push_str(&format!("- [Sitemap]({p}/sitemap.xml): every public CC0 program's page\n"));
        b.push_str(&format!("- [Dumps]({p}/dumps/): content-addressed archives of all of them (programs/<CID>.beatscript, programs/<CID>.jsonld, a manifest of CIDs); [the newest]({p}/dumps/latest.json)\n"));
        b.push_str(&format!("- [Schema]({p}/schema): the vocabulary (@vocab {p}/schema#), the embedded <beatscript-view> block, and links to the BeatScript ontology ({p}/ontology/ns#)\n"));
        b.push_str(&format!("- [Phrasebook]({p}{}): lines of verse, scripture, speech and work song first published before 1929 (CC0; each with its source, its meter as a step pattern and a sing example), to sing by id (sing @tempest_such_stuff on melody); [the file]({p}/phrasebook.txt) and [linked data]({p}/phrasebook.jsonld), and by CID at {p}/phrasebook/<CID>.txt and {p}/phrasebook/<CID>.jsonld (404 for any CID this server does not ship)\n", beatscript_serve::phrasebook::PATH));
        b.push_str(&format!("- [Theme library]({p}{}): melodies, basses and figures composed and first published before 1929 (CC0; each as BeatScript bars in a stated key, let-ready, with its role, meter, work, composer, year and pairings), to borrow as found material and build forms from; [the file]({p}/themes.txt) and [linked data]({p}/themes.jsonld), and by CID at {p}/themes/<CID>.txt and {p}/themes/<CID>.jsonld\n", beatscript_serve::themes::PATH));
        b.push_str(&format!("- [Open data]({p}/docs/open-data): what is published, what never is (private and non-CC0 programs), and how to mirror it\n\n"));
        b.push_str("## Examples\n");
        for e in &ex.list {
            let mut note = first_sentence(&e.description);
            let mut facts = Vec::new();
            if !e.license.is_empty() {
                facts.push(e.license.clone());
            }
            if e.bpm > 0.0 {
                facts.push(format!("{} BPM", go_g(e.bpm)));
            }
            if e.bars > 0 {
                facts.push(format!("{} bars", e.bars));
            }
            if !facts.is_empty() {
                note.push_str(&format!(" ({})", facts.join(", ")));
            }
            b.push_str(&format!("- [{}]({p}/examples/{}.beatscript): {note}\n", e.name, e.name));
        }
        b.push_str(&format!("- [All examples as JSON]({p}/api/examples): name, title, description, cid, bpm, bars, license\n\n"));
        b.push_str("## Phrase templates\n");
        b.push_str(&format!("- [Phrase templates]({p}/docs/BEATSCRIPT.md#phrase-templates): `arrange from template(NAME, PHRASE)` compiles a phrase, a phrasebook id (`@tempest_such_stuff`) or any words in quotes, to a whole track, written out as plain BeatScript like a seed: the phrase is the vocal line (its syllables set the line's length, its stress the accents and the beats, its meter the subdivision) and the arrangement is the template's own; `expand` shows the text\n"));
        for t in djlive::template::TEMPLATES.iter() {
            let progs: Vec<String> = t.examples.iter().filter(|n| ex.list.iter().any(|e| e.name == **n)).map(|n| format!("[{n}]({p}/examples/{n}.beatscript)")).collect();
            b.push_str(&format!("- template `{}`: {}. `{}`{}\n", t.name, t.summary, t.example, if progs.is_empty() { String::new() } else { format!("; examples: {}", progs.join(", ")) }));
        }
        b.push('\n');
        b.push_str("## API\n");
        b.push_str(&format!("- [MCP server]({p}/mcp): streamable HTTP; tools check, fmt, eval, load, get, share, open, render, record, list_examples, instruments, stop, devices, queue (an up-next list that never composes); writes need a bearer token. There is no compose tool: write the program yourself (cheat-sheet, list_examples, instruments, then check until ok)\n"));
        b.push_str(&format!("- [Compose]({p}/docs/api.md#compose): POST {p}/api/compose writes a program from a prompt for the web app (signed in, rate limited, capped; not an MCP tool)\n"));
        b.push_str(&format!("- [Render]({p}/docs/api.md#render): GET {p}/api/render/<CID>?format=opus|wav streams the offline render as it renders\n"));
        b.push_str(&format!("- [Linked data]({p}/docs/api.md#linked-data-json-ld): the versioned JSON-LD @context ({p}/context/beatscript-v1.jsonld, the BeatScript vocabulary under {p}/schema#; BeatScript 1 documents name the first one, {p}/context/v1.jsonld) and a program's JSON-LD document ({p}/c/<CID>.jsonld)\n"));
        b.push_str(&format!("- [Share links]({p}/docs/api.md#share-links): the link format, and how to open one safely\n"));
        b.push_str(&format!("- [Sign-in and tokens]({p}/docs/api.md#sign-in-and-tokens): wallet or browser-key challenge, cookie, bearer tokens for MCP\n\n"));
        b.push_str("## Optional\n");
        b.push_str(&format!("- [Generator you can read]({}): the post behind the language: a generator whose output is text you can read and edit\n", EXTERNAL_LINKS[0]));
        b
    }

    fn llms_full(&self, ex: &Examples) -> String {
        let p = &self.public;
        let mut b = self.llms_txt(ex);
        if let Some(cs) = self.cheatsheet() {
            b.push_str(&format!("\n---\n\nCheat-sheet (also at {p}/docs/cheatsheet.md):\n\n{}\n", cs.trim()));
        }
        if let Some(md) = self.read_doc("BEATSCRIPT.md") {
            let parts: Vec<String> = ["## The rules in one screen", "## Grammar (EBNF)"].iter().map(|h| section(&md, h)).filter(|s| !s.is_empty()).collect();
            if !parts.is_empty() {
                b.push_str(&format!("\n---\n\nGrammar, from the language reference ({p}/docs/BEATSCRIPT.md):\n\n{}\n", parts.join("\n\n")));
            }
        }
        let pick = ex.text.get(&self.full_example).map(|t| (self.full_example.clone(), t.clone())).or_else(|| ex.list.iter().find_map(|e| ex.text.get(&e.name).map(|t| (e.name.clone(), t.clone()))));
        if let Some((name, text)) = pick {
            b.push_str(&format!("\n---\n\nExample program {} (CC0-1.0, {p}/examples/{name}.beatscript):\n\n```beatscript\n{}\n```\n", crate::site::go_quote(&name), text.trim_end_matches('\n')));
        }
        b
    }
}

/// Go's `%g` for a tempo: the shortest representation (no exponent for the values a tempo takes).
fn go_g(x: f64) -> String {
    format!("{x}")
}

/// The part of a description up to its first ". " (at most 150 characters).
fn first_sentence(d: &str) -> String {
    let mut d = d.replace('\n', " ").trim().to_string();
    if let Some(i) = d.find(". ") {
        if i > 0 {
            d.truncate(i + 1);
        }
    }
    let r: Vec<char> = d.chars().collect();
    if r.len() > 150 {
        d = r[..147].iter().collect::<String>().trim_end_matches([' ', ',', ';', ':']).to_string() + "...";
    }
    d
}

fn heading_level(l: &str) -> Option<usize> {
    let n = l.len() - l.trim_start_matches('#').len();
    if (1..=6).contains(&n) && l[n..].starts_with(|c: char| c.is_whitespace()) {
        Some(n)
    } else {
        None
    }
}

/// `docs.Section`: the markdown from the line equal to `heading` to the next heading of the same or a higher level,
/// ignoring `#` lines inside code fences.
pub fn section(md: &str, heading: &str) -> String {
    let Some(level) = heading_level(&format!("{heading} ")) else { return String::new() };
    let lines: Vec<&str> = md.split('\n').collect();
    let (mut fence, mut start) = (false, None);
    for (i, l) in lines.iter().enumerate() {
        if l.starts_with("```") {
            fence = !fence;
            continue;
        }
        if fence {
            continue;
        }
        match start {
            None => {
                if l.trim_end_matches([' ', '\t']) == heading {
                    start = Some(i);
                }
            }
            Some(s) => {
                if heading_level(l).is_some_and(|n| n <= level) {
                    return lines[s..i].join("\n").trim_end_matches('\n').to_string();
                }
            }
        }
    }
    start.map_or(String::new(), |s| lines[s..].join("\n").trim_end_matches('\n').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_like_goldmark() {
        let mut s = HashMap::new();
        assert_eq!(heading_id("Grammar (EBNF)", &mut s), "grammar-ebnf");
        assert_eq!(heading_id("Grammar (EBNF)", &mut s), "grammar-ebnf-1");
        assert_eq!(heading_id("Patterns: mini-notation", &mut s), "patterns-mini-notation");
        assert_eq!(heading_id("éé", &mut s), "heading");
    }

    #[test]
    fn links() {
        assert_eq!(rewrite_doc_link("/docs/BEATSCRIPT.md#x").as_deref(), Some("/docs/language#x"));
        assert_eq!(rewrite_doc_link("https://beatscript.app/docs/BTW.md#x").as_deref(), Some("/docs/language#x"), "the old name of the reference");
        assert_eq!(rewrite_doc_link("https://beatscript.app/docs/api.md").as_deref(), Some("/docs/api"));
        assert_eq!(rewrite_doc_link("/docs/other.md"), None);
        assert_eq!(rewrite_doc_link("/docs/arrange.md#the-block").as_deref(), Some("/docs/arrange#the-block"));
    }

    #[test]
    fn the_arrange_page_is_served_when_installed() {
        let dir = std::env::temp_dir().join(format!("btw-docs-arrange-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("arrange.md"), "# Arrange mutations\n\nA [link](https://beatscript.app/docs/arrange.md).\n").unwrap();
        let d = Docs::new(Some(dir.clone()), None, "https://beatscript.app");
        let ex = Examples { list: Vec::new(), text: HashMap::new() };
        for path in ["/docs/arrange", "/docs/arrange.md"] {
            assert!(d.is_route(path), "{path}");
            assert!(matches!(d.answer(path, &ex), Some(Answer::Body(..))), "{path}");
        }
        let Some(Answer::Body(_, b, _, _)) = d.answer("/docs/arrange", &ex) else { unreachable!() };
        let html = String::from_utf8_lossy(&b);
        assert!(html.contains("Arrange mutations") && html.contains("href=\"/docs/arrange\""), "{html}");
        assert!(String::from_utf8_lossy(&d.render_index()).contains("/docs/arrange"));
        // not installed: listed nowhere, answered 404
        let none = Docs::new(Some(std::env::temp_dir().join("btw-docs-none-x")), None, "https://beatscript.app");
        assert!(matches!(none.answer("/docs/arrange", &ex), Some(Answer::NotInstalled)));
        assert!(!String::from_utf8_lossy(&none.render_index()).contains("/docs/arrange"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_schema_is_a_context_to_a_processor_and_a_page_to_a_person() {
        let d = Docs::new(None, None, "https://beatscript.app");
        let ex = Examples { list: Vec::new(), text: HashMap::new() };
        let get = |accept: &str, path: &str| {
            let mut h = HeaderMap::new();
            if !accept.is_empty() {
                h.insert(header::ACCEPT, HeaderValue::from_str(accept).unwrap());
            }
            d.respond(&Method::GET, &h, path, &ex).unwrap()
        };
        let ct = |r: &Response<Body>| r.headers()[header::CONTENT_TYPE].to_str().unwrap().to_string();
        for (accept, want) in [("application/ld+json", "application/ld+json"), ("application/json", "application/ld+json"), ("text/html,application/xhtml+xml", "text/html"), ("", "text/html"), ("application/ld+json;q=0.9, text/html", "application/ld+json")] {
            let r = get(accept, "/schema");
            assert!(ct(&r).starts_with(want), "{accept:?} -> {}", ct(&r));
            assert_eq!(r.headers()[header::VARY], "Accept");
            assert_eq!(r.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
        }
        assert!(ct(&get("text/html", "/schema.jsonld")).starts_with("application/ld+json"));
        assert!(ct(&get("", "/schema.md")).starts_with("text/markdown"));
        let Some(Answer::Body(_, b, _, _)) = d.answer("/schema.jsonld", &ex) else { unreachable!() };
        let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
        assert_eq!(v["@context"]["@vocab"], "https://beatscript.app/schema#");
        assert_eq!(v["@context"]["Program"], "bs:Program");
        let Some(Answer::Body(_, b, _, _)) = d.answer("/schema", &ex) else { unreachable!() };
        let html = String::from_utf8_lossy(&b);
        assert!(html.contains("<title>BeatScript schema · BeatScript docs</title>") && html.contains("beatscript-view"), "{html}");
    }

    #[test]
    fn every_term_of_the_vocabulary_is_defined_on_the_schema_page() {
        // the context's terms (not its prefixes) each have an anchor, so `https://beatscript.app/schema#<term>` resolves
        let ctx = djlive::progld::schema_context();
        let Some(djengine::json::JsonValue::Object(o)) = ctx.get("@context") else { panic!() };
        let prefixes = ["@vocab", "bs", "bw", "schema", "MusicComposition", "encoding"];
        for (k, _) in o.iter() {
            if prefixes.contains(&k) {
                continue;
            }
            assert!(SCHEMA_MD.contains(&format!("<a id=\"{k}\"></a>")), "SCHEMA.md does not define `{k}`");
        }
        // and the page has them as fragments
        let html = term_anchors(&render_markdown(SCHEMA_MD).body);
        for (k, _) in o.iter().filter(|(k, _)| !prefixes.contains(k)) {
            assert!(html.contains(&format!("<p id=\"{k}\"><strong>{k}</strong>")), "no #{k} on the page");
        }
        assert!(html.contains("<p id=\"ProgramDump\">"));
        // the open-data page is built in and listed in llms.txt with the feeds, sitemap and dumps
        let d = Docs::new(None, None, "https://beatscript.app");
        let ex = Examples { list: Vec::new(), text: HashMap::new() };
        assert!(matches!(d.answer("/docs/open-data", &ex), Some(Answer::Body(..))));
        let l = d.llms_txt(&ex);
        for u in ["/feed.json", "/feed.atom", "/sitemap.xml", "/dumps/latest.json", "/schema", "/p/<CID>.beatscript", "/docs/open-data"] {
            assert!(l.contains(&format!("https://beatscript.app{u}")), "llms.txt lacks {u}");
        }
    }

    /// An inline `<style>` block or a `style=` attribute in an HTML page: what the site's CSP (`style-src 'self'`,
    /// main.rs) blocks, leaving the page unstyled. The JSON-LD `<script type="application/ld+json">` is data, never run,
    /// so `script-src` does not touch it; any other inline `<script>` with a body is blocked too.
    pub(crate) fn inline_styles(html: &str) -> Vec<String> {
        let mut bad = Vec::new();
        let lower = html.to_ascii_lowercase();
        if lower.contains("<style") {
            bad.push("a <style> block".to_string());
        }
        for (i, _) in lower.match_indices("style=") {
            // an attribute: whitespace before it, inside a tag
            if i > 0 && lower.as_bytes()[i - 1].is_ascii_whitespace() && lower[..i].rfind('<') > lower[..i].rfind('>') {
                bad.push(format!("a style= attribute: {}", &html[i..(i + 40).min(html.len())]));
            }
        }
        let mut at = 0;
        while let Some(j) = lower[at..].find("<script") {
            let k = at + j;
            let end = k + lower[k..].find('>').unwrap_or(lower.len() - k);
            let tag = &lower[k..end];
            let close = end + 1 + lower[end..].find("</script>").map_or(0, |c| c - 1);
            let body = lower.get(end + 1..close).unwrap_or("").trim();
            if !tag.contains("src=") && !tag.contains("application/ld+json") && !body.is_empty() {
                bad.push(format!("an inline script: {}", &html[k..(end + 1).min(html.len())]));
            }
            at = end;
        }
        bad
    }

    /// Every HTML page the docs routes serve is styled from 'self' alone, and every stylesheet they link is served,
    /// text/css. The Grammar page wrote its styles inline (`<style>`) and rendered unstyled on beatscript.app
    /// (2026-11-22); this keeps it, and every page beside it, from doing that again.
    #[test]
    fn no_served_page_styles_itself_inline() {
        let dir = std::env::temp_dir().join(format!("btw-docs-csp-{}", std::process::id()));
        let src = dir.join("src");
        std::fs::create_dir_all(src.join("assets")).unwrap();
        let btw = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/BEATSCRIPT.md")).unwrap();
        std::fs::write(dir.join("BEATSCRIPT.md"), btw).unwrap();
        // a table with every alignment: the renderer's classes, never style=
        let table = "# T\n\n| l | c | r | n |\n|:--|:-:|--:|---|\n| a | b | c | d |\n";
        for f in ["cheatsheet.md", "arrange.md"] {
            std::fs::write(dir.join(f), table).unwrap();
        }
        std::fs::write(src.join("api.md"), table).unwrap();
        std::fs::write(src.join("assets/docs.css"), "body{}").unwrap();
        let d = Docs::new(Some(dir.clone()), Some(src), "https://beatscript.app");
        let ex = Examples { list: Vec::new(), text: HashMap::new() };
        let routes = ["/docs/", "/docs/cheatsheet", "/docs/language", "/docs/arrange", "/docs/api", "/schema", "/docs/open-data", "/grammar", "/authorship", "/phrasebook", FOUND_PATH, FORMS_PATH, RADIO_PATH, "/themes"];
        let mut sheets = std::collections::BTreeSet::new();
        for path in routes {
            assert!(d.is_route(path), "{path}");
            let Some(Answer::Body(ct, b, _, _)) = d.answer(path, &ex) else { panic!("{path} is not served") };
            assert!(ct.starts_with("text/html"), "{path}: {ct}");
            let html = String::from_utf8_lossy(&b).to_string();
            assert_eq!(inline_styles(&html), Vec::<String>::new(), "{path} styles itself inline: the CSP blocks it");
            assert!(html.contains(RIGHTS_FOOTER), "{path} has no footer link to /authorship");
            for (i, _) in html.match_indices("<link rel=\"stylesheet\" href=\"") {
                let rest = &html[i + 29..];
                sheets.insert(rest[..rest.find('"').unwrap()].to_string());
            }
        }
        let Some(Answer::Body(_, b, _, _)) = d.answer("/docs/arrange", &ex) else { unreachable!() };
        let html = String::from_utf8_lossy(&b);
        assert!(html.contains("<th class=\"align-left\">") && html.contains("<td class=\"align-center\">") && html.contains("<td class=\"align-right\">"), "{html}");
        assert!(sheets.contains("/grammar.css"), "{sheets:?}");
        // the generated pages' own stylesheets, and every one a page links, are served here as text/css
        sheets.extend(["/grammar.css", "/lineage.css", "/mcp.css"].map(String::from));
        for path in sheets.iter().filter(|p| p.starts_with('/') && !p.starts_with("/card/")) {
            assert!(d.is_route(path), "{path} is linked but not served");
            let Some(Answer::Body(ct, b, _, _)) = d.answer(path, &ex) else { panic!("{path} is not served") };
            assert!(ct.starts_with("text/css") && b.len() > 100 || path == "/docs/assets/docs.css", "{path}: {ct}, {} bytes", b.len());
        }
        let Some(Answer::Body(_, css, _, _)) = d.answer("/grammar.css", &ex) else { unreachable!() };
        let css = String::from_utf8_lossy(&css);
        assert!(css.contains("prefers-color-scheme:light") && css.contains(".rr .rr-term rect"), "light and dark, and the diagrams'");
        // the pages served from elsewhere in beatscript-serve: the share page and the MCP landing page link theirs too
        assert!(inline_styles(&crate::cards::test_page_html()).is_empty());
        assert!(inline_styles(&beatscript_serve::lineage::page::missing_page("https://beatscript.app")).is_empty());
        // and end with the same footer link to /authorship
        assert!(crate::cards::test_page_html().contains(RIGHTS_FOOTER), "the share page has no footer link to /authorship");
        assert!(beatscript_serve::lineage::page::missing_page("https://beatscript.app").contains(RIGHTS_FOOTER), "the lineage pages have no footer link to /authorship");
        let tools = crate::mcp::tools::tool_list("https://beatscript.app", 16384, 60, 30, (50, 20, 30, 8, 30));
        let landing = crate::mcp::tools::landing_html("https://beatscript.app", &tools);
        assert!(inline_styles(&landing).is_empty() && landing.contains(RIGHTS_FOOTER), "the MCP landing page has no footer link to /authorship");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// No source in beatscript-serve writes an inline style into a page: a new page that does fails here before it ships
    /// unstyled. (Tests may hold the strings they look for.)
    #[test]
    fn no_source_writes_an_inline_style() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![root.clone()];
        let mut hits = Vec::new();
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                if p.extension().and_then(|x| x.to_str()) != Some("rs") || p.file_name().and_then(|n| n.to_str()) == Some("tests.rs") {
                    continue;
                }
                // the MCP artifact page is published on another host (a claude.ai Artifact, whose CSP allows an inline
                // <style>, as pflow.xyz's pflow_artifact pages have), never served under this site's CSP
                if p.ends_with("mcp/artifact.rs") {
                    continue;
                }
                let s = std::fs::read_to_string(&p).unwrap();
                let code = &s[..s.find("#[cfg(test)]").unwrap_or(s.len())];
                for (n, line) in code.lines().enumerate() {
                    if line.trim_start().starts_with("//") {
                        continue;
                    }
                    if line.contains("<style") || line.contains(" style=\\\"") || line.contains(" style=\"") {
                        hits.push(format!("{}:{}", p.strip_prefix(&root).unwrap().display(), n + 1));
                    }
                }
            }
        }
        assert_eq!(hits, Vec::<String>::new(), "inline styles, which the site's CSP (style-src 'self') blocks: link a stylesheet instead");
    }

    #[test]
    fn the_grammar_page_and_its_source_are_served() {
        let d = Docs::new(None, None, "https://beatscript.app");
        let ex = Examples { list: Vec::new(), text: HashMap::new() };
        let Some(Answer::Body(ct, b, _, cors)) = d.answer("/grammar.ebnf", &ex) else { panic!() };
        assert!(ct.starts_with("text/plain") && cors && b == djlive::grammar::SOURCE.as_bytes());
        let Some(Answer::Body(ct, b, _, _)) = d.answer("/grammar", &ex) else { panic!() };
        let html = String::from_utf8_lossy(&b);
        assert!(ct.starts_with("text/html") && html.contains("<link rel=\"canonical\" href=\"https://beatscript.app/grammar\">"));
        assert!(html.contains("<section class=\"rule\" id=\"rule-statement\">") && html.contains("<svg class=\"rr\""));
        assert!(d.llms_txt(&ex).contains("https://beatscript.app/grammar.ebnf"));
        // the page is linked data, and the docs' JSON-LD index lists it by the same @id
        assert!(html.contains("<script type=\"application/ld+json\">{\"@context\":\"https://schema.org\",\"@type\":\"DefinedTermSet\",\"@id\":\"https://beatscript.app/grammar\""));
        let index = String::from_utf8_lossy(&d.render_index()).to_string();
        let at = index.find("<script type=\"application/ld+json\">").expect("the docs index has no JSON-LD") + 35;
        let ld: serde_json::Value = serde_json::from_str(&index[at..at + index[at..].find("</script>").unwrap()]).unwrap();
        assert_eq!((ld["@type"].as_str(), ld["@id"].as_str()), (Some("CollectionPage"), Some("https://beatscript.app/docs/")));
        let parts = ld["hasPart"].as_array().unwrap();
        let grammar = parts.iter().find(|p| p["@id"] == "https://beatscript.app/grammar").expect("the JSON-LD index does not list the Grammar page");
        assert_eq!((grammar["@type"].as_str(), grammar["encoding"]["contentUrl"].as_str()), (Some("DefinedTermSet"), Some("https://beatscript.app/grammar.ebnf")));
        assert!(parts.iter().any(|p| p["@id"] == "https://beatscript.app/llms.txt"));
    }

    /// The wording Matt approved on 2026-11-22 (project-datum 6638), checked against the Copyright Office's Part 2
    /// report (January 2025). A change to it is his to make: change it here and in docs/AUTHORSHIP.md together.
    const APPROVED: &str = "Programs here are CC0: anyone can use them for anything. If an AI wrote a program from your prompts, current US guidance says prompts alone don't make you its author, so much of it may not be copyrightable at all. What you choose, arrange and edit yourself is your expression. Under CC0 you give that away too. Every edit gets a new CID and lineage records which version came from which, so your choices are on the record. Want to keep something? Keep it private on your own devices: private work is sealed and never stored here.";

    /// One technique (docs/beatscript/found-phrases.md), many views: the page and its markdown, the docs index and its JSON-LD,
    /// llms.txt and the sitemap, with no inline style; and the page names every family of the phrasebook, every groove the
    /// code advises (`beatscript_serve::found::FAMILY_GROOVES`) and both reference programs, so the advice cannot drift from the page.
    #[test]
    fn the_found_phrases_page_is_one_technique_in_many_views() {
        let d = Docs::new(None, None, "https://beatscript.app");
        let ex = Examples { list: Vec::new(), text: HashMap::new() };
        let Some(Answer::Body(ct, b, _, _)) = d.answer(FOUND_PATH, &ex) else { panic!("{FOUND_PATH} is not served") };
        assert!(ct.starts_with("text/html"));
        let html = String::from_utf8_lossy(&b);
        assert!(inline_styles(&html).is_empty(), "{:?}", inline_styles(&html));
        assert!(html.contains("<link rel=\"canonical\" href=\"https://beatscript.app/docs/found-phrases\">"));
        let r = d.respond(&Method::GET, &HeaderMap::new(), "/docs/found-phrases.md", &ex).expect("the markdown is a route");
        assert_eq!(r.status(), 200);
        let l = d.llms_txt(&ex);
        assert!(l.contains("https://beatscript.app/docs/found-phrases.md") && l.contains("https://beatscript.app/docs/found-phrases)"), "llms.txt lacks the page");
        let index = String::from_utf8_lossy(&d.render_index()).to_string();
        assert!(index.contains("<a href=\"/docs/found-phrases\">Found phrases</a>"));
        let marker = "<script type=\"application/ld+json\">";
        let from = index.find(marker).unwrap() + marker.len();
        let ld: serde_json::Value = serde_json::from_str(&index[from..from + index[from..].find("</script>").unwrap()].replace("\\u003c", "<").replace("\\u003e", ">").replace("\\u0026", "&")).unwrap();
        assert!(ld["hasPart"].as_array().unwrap().iter().any(|p| p["@id"] == "https://beatscript.app/docs/found-phrases" && p["encoding"]["contentUrl"] == "https://beatscript.app/docs/found-phrases.md"));
        assert!(beatscript_serve::opendata::sitemap("https://beatscript.app", &[]).contains("<loc>https://beatscript.app/docs/found-phrases</loc>"));
        // the page and the code say the same things
        for f in djengine::vocal::phoneme::PD_FAMILIES.iter() {
            assert!(FOUND_MD.contains(f.id), "the page does not name the family {}", f.id);
        }
        for (family, groove, _) in beatscript_serve::found::FAMILY_GROOVES.iter() {
            assert!(!groove.is_empty() && beatscript_serve::found::groove(family) == Some(*groove));
        }
        for cid in ["z4EBG9j94aGDk5revau2rGjEE46x9bebqNtKwiUyPqGZy7TRHCj", "z4EBG9jDkEcovjGFiHHNsiTGLk6jskJRAKn6MvUTyvTJSojtRa5"] {
            assert!(FOUND_MD.contains(cid), "the page does not name the reference program {cid}");
        }
        // every `word_word` the page writes in backticks is a line of the book (the `drm_*` parts aside)
        for tok in FOUND_MD.split('`').skip(1).step_by(2) {
            if tok.contains('_') && !tok.contains(' ') && !tok.starts_with("drm_") && !["genre_template", "list_examples"].contains(&tok) {
                assert!(djengine::vocal::phoneme::pd_phrase(tok).is_some(), "the page names `{tok}`, which is no phrasebook line");
            }
        }
    }

    /// One doc (docs/beatscript/embed.md), many views: the page and its markdown, the docs index and its JSON-LD, llms.txt and
    /// the sitemap, with no inline style; and the page names what cards/embed.rs serves.
    #[test]
    fn the_embed_page_is_one_doc_in_many_views() {
        let d = Docs::new(None, None, "https://beatscript.app");
        let ex = Examples { list: Vec::new(), text: HashMap::new() };
        let Some(Answer::Body(ct, b, _, _)) = d.answer(EMBED_PATH, &ex) else { panic!("{EMBED_PATH} is not served") };
        assert!(ct.starts_with("text/html"));
        let html = String::from_utf8_lossy(&b);
        assert!(inline_styles(&html).is_empty(), "{:?}", inline_styles(&html));
        assert!(html.contains("<link rel=\"canonical\" href=\"https://beatscript.app/docs/embed\">"));
        assert!(d.is_route("/docs/embed") && d.is_route("/docs/embed.md"));
        assert_eq!(d.respond(&Method::GET, &HeaderMap::new(), "/docs/embed.md", &ex).expect("the markdown is a route").status(), 200);
        assert!(beatscript_serve::opendata::sitemap("https://beatscript.app", &[]).contains("<loc>https://beatscript.app/docs/embed</loc>"));
        let l = d.llms_txt(&ex);
        assert!(l.contains("https://beatscript.app/docs/embed.md") && l.contains("https://beatscript.app/docs/embed)"), "llms.txt lacks the embed page");
        let index = String::from_utf8_lossy(&d.render_index()).to_string();
        assert!(index.contains("<a href=\"/docs/embed\">Embed a program</a>"));
        assert!(index.contains("\"@id\":\"https://beatscript.app/docs/embed\""), "the index's JSON-LD");
        for said in ["/embed/<CID>", "<beatscript-player", "embed.js", "/oembed?url=", "twitter:player", "og:audio", "/api/render/<CID>?format=opus", "?compact=1", "?theme=light", "allow-scripts allow-same-origin allow-popups allow-popups-to-escape-sandbox"] {
            assert!(EMBED_MD.contains(said), "the page does not say {said}");
        }
    }

    /// One doc (docs/beatscript/radio.md), many views: the page and its markdown, the docs index and its JSON-LD, llms.txt and the
    /// sitemap, with no inline style. (What the page says of stations is the docs' own; the MCP exposure table holds the
    /// station tool to it.)
    #[test]
    fn the_radio_page_is_one_doc_in_many_views() {
        let d = Docs::new(None, None, "https://beatscript.app");
        let ex = Examples { list: Vec::new(), text: HashMap::new() };
        let Some(Answer::Body(ct, b, _, _)) = d.answer(RADIO_PATH, &ex) else { panic!("{RADIO_PATH} is not served") };
        assert!(ct.starts_with("text/html"));
        let html = String::from_utf8_lossy(&b);
        assert!(inline_styles(&html).is_empty(), "{:?}", inline_styles(&html));
        assert!(html.contains("<link rel=\"canonical\" href=\"https://beatscript.app/docs/radio\">"));
        assert!(d.is_route("/docs/radio") && d.is_route("/docs/radio.md"));
        let md = d.respond(&Method::GET, &HeaderMap::new(), "/docs/radio.md", &ex).expect("the markdown is a route");
        assert_eq!(md.status(), 200);
        assert!(beatscript_serve::opendata::sitemap("https://beatscript.app", &[]).contains("<loc>https://beatscript.app/docs/radio</loc>"));
        let l = d.llms_txt(&ex);
        assert!(l.contains("https://beatscript.app/docs/radio.md") && l.contains("https://beatscript.app/docs/radio)"), "llms.txt lacks the radio page");
        let index = String::from_utf8_lossy(&d.render_index()).to_string();
        assert!(index.contains("<a href=\"/docs/radio\">Radio: stations</a>"));
        assert!(index.contains("\"@id\":\"https://beatscript.app/docs/radio\""), "the index's JSON-LD");
        // the MCP tool points at this page
        let tools = crate::mcp::tools::tool_list("https://beatscript.app", 16384, 60, 30, (50, 20, 30, 8, 30));
        let st = tools.iter().find(|t| t["name"] == "station").expect("the station tool");
        assert!(st["description"].as_str().unwrap_or("").contains("https://beatscript.app/docs/radio.md"));
    }

    /// One doc (docs/beatscript/polish.md), many views: the page and its markdown, the docs index and its JSON-LD, llms.txt and the
    /// sitemap, with no inline style; the listen tool's description and the MCP instructions' authoring loop point at it.
    #[test]
    fn the_polish_page_is_one_doc_in_many_views() {
        let d = Docs::new(None, None, "https://beatscript.app");
        let ex = Examples { list: Vec::new(), text: HashMap::new() };
        let Some(Answer::Body(ct, b, _, _)) = d.answer(POLISH_PATH, &ex) else { panic!("{POLISH_PATH} is not served") };
        assert!(ct.starts_with("text/html"));
        let html = String::from_utf8_lossy(&b);
        assert!(inline_styles(&html).is_empty(), "{:?}", inline_styles(&html));
        assert!(html.contains("<link rel=\"canonical\" href=\"https://beatscript.app/docs/polish\">"));
        assert!(d.is_route("/docs/polish") && d.is_route("/docs/polish.md"));
        assert_eq!(d.respond(&Method::GET, &HeaderMap::new(), "/docs/polish.md", &ex).expect("the markdown is a route").status(), 200);
        assert!(beatscript_serve::opendata::sitemap("https://beatscript.app", &[]).contains("<loc>https://beatscript.app/docs/polish</loc>"));
        let l = d.llms_txt(&ex);
        assert!(l.contains("https://beatscript.app/docs/polish.md") && l.contains("https://beatscript.app/docs/polish)"), "llms.txt lacks the polish page");
        let index = String::from_utf8_lossy(&d.render_index()).to_string();
        assert!(index.contains("<a href=\"/docs/polish\">Polish a mix</a>"));
        assert!(index.contains("\"@id\":\"https://beatscript.app/docs/polish\""), "the index's JSON-LD");
        let tools = crate::mcp::tools::tool_list("https://beatscript.app", 16384, 60, 30, (50, 20, 30, 8, 30));
        let t = tools.iter().find(|t| t["name"] == "listen").expect("the listen tool");
        assert!(t["description"].as_str().unwrap_or("").contains("https://beatscript.app/docs/polish.md"));
        assert!(t["inputSchema"]["properties"]["compare"].is_object() && t["inputSchema"]["properties"]["targets"].is_object());
        assert!(crate::mcp::text::INSTRUCTIONS.contains("%[1]s/docs/polish.md"), "the authoring loop's listen step names the page");
        // the page names every line it tells you to write, and what the language lacks
        for said in ["duck B under A.kick 6db", "mix { B eq low -6 }", "filter hp 150", "master: drive", "vel=", "no pan or width control", "a limiter and a compressor", "docs/ROADMAP.md"] {
            assert!(POLISH_MD.contains(said), "the page does not say {said}");
        }
    }

    /// One doc (docs/beatscript/forms.md), many views: the page and its markdown, the docs index and its JSON-LD, llms.txt and the
    /// sitemap, with no inline style; and the page says what the code does: every form by name and label, each form's themes
    /// and the mutation operators its plan uses, the reference programs' CIDs, and every theme id it writes is a theme. The
    /// theme library's page is served beside it (200, no inline style, in the index, llms.txt and the sitemap).
    #[test]
    fn the_forms_page_is_one_doc_in_many_views() {
        let d = Docs::new(None, None, "https://beatscript.app");
        let ex = Examples { list: Vec::new(), text: HashMap::new() };
        for path in [FORMS_PATH, beatscript_serve::themes::PATH] {
            let Some(Answer::Body(ct, b, _, _)) = d.answer(path, &ex) else { panic!("{path} is not served") };
            assert!(ct.starts_with("text/html"));
            let html = String::from_utf8_lossy(&b);
            assert!(inline_styles(&html).is_empty(), "{path}: {:?}", inline_styles(&html));
            assert!(html.contains(&format!("<link rel=\"canonical\" href=\"https://beatscript.app{path}\">")), "{path}");
            assert!(beatscript_serve::opendata::sitemap("https://beatscript.app", &[]).contains(&format!("<loc>https://beatscript.app{path}</loc>")), "{path}");
        }
        let r = d.respond(&Method::GET, &HeaderMap::new(), "/docs/forms.md", &ex).expect("the markdown is a route");
        assert_eq!(r.status(), 200);
        let l = d.llms_txt(&ex);
        assert!(l.contains("https://beatscript.app/docs/forms.md") && l.contains("https://beatscript.app/docs/forms)") && l.contains("https://beatscript.app/themes.txt"), "llms.txt lacks the forms or the themes");
        let index = String::from_utf8_lossy(&d.render_index()).to_string();
        assert!(index.contains("<a href=\"/docs/forms\">Forms</a>") && index.contains("<a href=\"/themes\">Theme library</a>"));
        let marker = "<script type=\"application/ld+json\">";
        let from = index.find(marker).unwrap() + marker.len();
        let ld: serde_json::Value = serde_json::from_str(&index[from..from + index[from..].find("</script>").unwrap()].replace("\\u003c", "<").replace("\\u003e", ">").replace("\\u0026", "&")).unwrap();
        let parts = ld["hasPart"].as_array().unwrap();
        assert!(parts.iter().any(|p| p["@id"] == "https://beatscript.app/docs/forms" && p["encoding"]["contentUrl"] == "https://beatscript.app/docs/forms.md"));
        assert!(parts.iter().any(|p| p["@id"] == "https://beatscript.app/themes"));
        // the page and the code say the same things
        for (t, f) in djlive::template::forms() {
            assert!(FORMS_MD.contains(&format!("template({}, ", t.name)), "the page does not write the form {}", t.name);
            let mut label = f.label.to_string();
            label[..1].make_ascii_uppercase();
            assert!(FORMS_MD.contains(&format!("## {label}\n")), "the page has no section for {}", f.label);
            for th in f.themes {
                assert!(FORMS_MD.contains(&format!("`{th}`")), "the page does not name {}'s theme {th}", t.name);
            }
            if let Some((name, cid)) = f.reference {
                assert!(FORMS_MD.contains(cid) && FORMS_MD.contains(&format!("`{name}`")), "the page does not name {}'s reference {name}", t.name);
            }
        }
        // every theme id the page writes in backticks is a theme of the library (`word_word` that is no phrasebook line)
        for tok in FORMS_MD.split('`').skip(1).step_by(2) {
            // (a role's lets and sections are named for the role: `a_melody`, `b_leads`, `ep_ground`, the quodlibet's `tune_a`,
            // docs/beatscript/ext-roles.md)
            let role_name = ["a_", "b_", "ep_", "tune_"].iter().any(|p| tok.starts_with(p));
            if tok.contains('_') && !role_name && !tok.contains('=') && !tok.contains(' ') && !tok.contains('*') && !tok.contains('(') && !tok.contains('/') && djengine::vocal::phoneme::pd_phrase(tok.trim_start_matches('@')).is_none() && !["genre_template", "list_examples"].contains(&tok) {
                let bar_of = tok.rsplit_once('_').filter(|(_, n)| n.bytes().all(|c| c.is_ascii_digit())).map(|(t, _)| t);
                assert!(djengine::themes::theme(tok).is_some() || bar_of.is_some_and(|t| djengine::themes::theme(t).is_some()), "the page names `{tok}`, which is no theme (nor a theme's bar)");
            }
        }
    }

    /// One text (docs/AUTHORSHIP.md), many views: the page and its markdown, the docs index and its JSON-LD,
    /// llms.txt and the sitemap all name it, and none of them is a second copy.
    #[test]
    fn the_authorship_page_is_one_text_in_many_views() {
        assert!(AUTHORSHIP_MD.contains(APPROVED), "docs/AUTHORSHIP.md is not the approved wording");
        assert!(AUTHORSHIP_MD.contains("(https://www.copyright.gov/ai/)"), "the source is not linked");
        assert_eq!(AUTHORSHIP_MD.trim_end().lines().last(), Some("This is plain language, not legal advice."));
        let d = Docs::new(None, None, "https://beatscript.app");
        let ex = Examples { list: Vec::new(), text: HashMap::new() };
        // the page: 200, text/html, the text, its source, no inline style, the raw markdown beside it
        let r = d.respond(&Method::GET, &HeaderMap::new(), AUTHORSHIP_PATH, &ex).expect("/authorship is a route");
        assert_eq!(r.status(), 200);
        assert!(r.headers()[header::CONTENT_TYPE].to_str().unwrap().starts_with("text/html"));
        let Some(Answer::Body(_, b, _, _)) = d.answer(AUTHORSHIP_PATH, &ex) else { panic!() };
        let html = String::from_utf8_lossy(&b).to_string();
        assert!(html.contains(&esc(APPROVED)), "{html}");
        assert!(html.contains("<a href=\"https://www.copyright.gov/ai/\">") && html.contains("This is plain language, not legal advice."));
        assert!(html.contains("<link rel=\"canonical\" href=\"https://beatscript.app/authorship\">"));
        assert_eq!(inline_styles(&html), Vec::<String>::new());
        let r = d.respond(&Method::GET, &HeaderMap::new(), "/authorship.md", &ex).unwrap();
        assert!(r.headers()[header::CONTENT_TYPE].to_str().unwrap().starts_with("text/markdown"));
        // llms.txt, the docs index and its JSON-LD, the sitemap
        assert!(d.llms_txt(&ex).contains("https://beatscript.app/authorship"));
        assert!(d.llms_txt(&ex).contains("- Source: https://github.com/stackdump/beatscript (MIT); programs CC0."), "llms.txt does not name the source");
        let index = String::from_utf8_lossy(&d.render_index()).to_string();
        assert!(index.contains("<a href=\"/authorship\">Authorship and rights</a>"));
        let parts = d.index_ld(&d.installed());
        assert_eq!(parts["isPartOf"]["codeRepository"], "https://github.com/stackdump/beatscript", "the docs' WebSite has no codeRepository");
        assert!(parts["hasPart"].as_array().unwrap().iter().any(|p| p["@id"] == "https://beatscript.app/authorship" && p["encoding"]["contentUrl"] == "https://beatscript.app/authorship.md"));
        assert!(beatscript_serve::opendata::sitemap("https://beatscript.app", &[]).contains("<loc>https://beatscript.app/authorship</loc>"));
    }

    /// Every standalone "btw" (any case) in `text`, with what is around it: a word, not part of a name, a path, a host
    /// or an identifier (`btw.bitwrap.io`, `css/beatscript.css`, `btw_...`, `data-beatscript`, `djapp_btw`, `#! btw 1` all pass).
    fn standalone_btw(where_: &str, text: &str) -> Vec<String> {
        let joined = |c: char| c.is_ascii_alphanumeric() || "_./:#$`'-".contains(c);
        let lower = text.to_ascii_lowercase();
        let mut hits = Vec::new();
        for (i, _) in lower.match_indices("btw") {
            let before = text[..i].chars().next_back();
            let mut rest = text[i + 3..].chars();
            let after = rest.next();
            // a following '.' joins only a host or a file name (btw.bitwrap.io, beatscript.css), not a sentence's end
            let after_joins = match after {
                Some('.') => rest.next().is_some_and(|c| c.is_ascii_alphanumeric()),
                a => a.is_some_and(joined),
            };
            if before.is_some_and(joined) || after_joins {
                continue;
            }
            let (a, b) = (text[..i].char_indices().rev().nth(40).map_or(0, |x| x.0), (i + 43).min(text.len()));
            let (a, b) = ((0..=a).rev().find(|&k| text.is_char_boundary(k)).unwrap_or(0), (b..=text.len()).find(|&k| text.is_char_boundary(k)).unwrap_or(text.len()));
            hits.push(format!("{where_}: ...{}...", text[a..b].split_whitespace().collect::<Vec<_>>().join(" ")));
        }
        hits
    }

    /// What a person reads on an HTML page: its title, its meta descriptions and its text. Code (`pre`, `code`, `kbd`,
    /// `samp`), scripts, styles, comments and attributes are not read.
    fn visible_html(html: &str) -> String {
        let mut out = String::new();
        for name in ["description", "og:title", "og:description", "twitter:title", "twitter:description"] {
            for attr in ["name", "property"] {
                let tag = format!("<meta {attr}=\"{name}\" content=\"");
                for (i, _) in html.match_indices(&tag) {
                    let rest = &html[i + tag.len()..];
                    out.push_str(&rest[..rest.find('"').unwrap_or(0)]);
                    out.push('\n');
                }
            }
        }
        let mut s = html.to_string();
        let lower = |s: &str| s.to_ascii_lowercase();
        for (open, close) in [("<!--", "-->"), ("<script", "</script>"), ("<style", "</style>"), ("<pre", "</pre>"), ("<code", "</code>"), ("<kbd", "</kbd>"), ("<samp", "</samp>")] {
            while let Some(a) = lower(&s).find(open) {
                let b = lower(&s)[a..].find(close).map_or(s.len(), |b| a + b + close.len());
                s.replace_range(a..b, " ");
            }
        }
        let mut text = String::new();
        let mut in_tag = false;
        for c in s.chars() {
            match c {
                '<' => in_tag = true,
                '>' if in_tag => {
                    in_tag = false;
                    text.push(' ');
                }
                c if !in_tag => text.push(c),
                _ => {}
            }
        }
        out + &text.replace("&nbsp;", " ").replace("&amp;", "&").replace("&#39;", "'").replace("&quot;", "\"")
    }

    /// The same for markdown and plain text: no fenced blocks, inline code, link targets or bare URLs.
    fn visible_md(md: &str) -> String {
        let mut out = String::new();
        let mut fence = false;
        for line in md.lines() {
            if line.trim_start().starts_with("```") {
                fence = !fence;
                continue;
            }
            if fence {
                continue;
            }
            let mut l = String::new();
            let mut code = false;
            for c in line.chars() {
                if c == '`' {
                    code = !code;
                    l.push(' ');
                } else if !code {
                    l.push(c);
                }
            }
            let l: Vec<&str> = l.split_whitespace().filter(|w| !w.contains("://") && !w.starts_with("](")).collect();
            out.push_str(&l.join(" ").replace("](", "] ("));
            out.push('\n');
        }
        out
    }

    /// The name is BeatScript (product imperative BeatScript #11): no page beatscript-serve serves, no MCP description and no
    /// llms.txt says "btw" where a person reads it. Identifiers stay as they are and pass: the `#! btw 1` header, `.btw`,
    /// the crates and binaries, `data-beatscript`, URL paths, the JSON-LD IRIs, API fields, commands in code. beats' btwsite
    /// test/unit.mjs checks its static pages (the editor, /privacy, /delete-account) and api.md the same way.
    #[test]
    fn no_page_a_person_reads_says_btw() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let dir = std::env::temp_dir().join(format!("btw-docs-name-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(root.join("docs/BEATSCRIPT.md"), dir.join("BEATSCRIPT.md")).unwrap();
        std::fs::copy(root.join("docs/ARRANGE.md"), dir.join("arrange.md")).unwrap();
        // api.md and the docs assets: site/docs-src (the deploy's -docs-src)
        let src = root.join("site/docs-src");
        let d = Docs::new(Some(dir.clone()), src.is_dir().then_some(src.clone()), "https://beatscript.app");
        let ex = Examples { list: Vec::new(), text: HashMap::new() };
        let mut hits = Vec::new();
        for path in ["/docs/", "/docs/cheatsheet", "/docs/language", "/docs/arrange", "/docs/api", "/schema", "/docs/open-data", "/grammar", AUTHORSHIP_PATH, "/phrasebook", FOUND_PATH, FORMS_PATH, RADIO_PATH, "/themes"] {
            match d.answer(path, &ex) {
                Some(Answer::Body(_, b, _, _)) => hits.extend(standalone_btw(path, &visible_html(&String::from_utf8_lossy(&b)))),
                _ => assert!(path == "/docs/api" && !src.is_dir(), "{path} is not served"),
            }
        }
        hits.extend(standalone_btw("/llms-full.txt", &visible_md(&d.llms_full(&ex))));
        hits.extend(standalone_btw("share page", &visible_html(&crate::cards::test_page_html())));
        hits.extend(standalone_btw("lineage page", &visible_html(&beatscript_serve::lineage::page::missing_page("https://beatscript.app"))));
        hits.extend(standalone_btw("pairing page", &visible_html(crate::hub::PAIR_PAGE)));
        let tools = crate::mcp::tools::tool_list("https://beatscript.app", 16384, 60, 30, (50, 20, 30, 8, 30));
        hits.extend(standalone_btw("/mcp", &visible_html(&crate::mcp::tools::landing_html("https://beatscript.app", &tools))));
        hits.extend(standalone_btw("MCP instructions", &visible_md(&crate::mcp::text::fill(crate::mcp::text::INSTRUCTIONS, &["https://beatscript.app"]))));
        for t in &tools {
            let name = t["name"].as_str().unwrap_or("");
            hits.extend(standalone_btw(&format!("MCP {name}"), &visible_md(t["description"].as_str().unwrap_or(""))));
            for (k, v) in t["inputSchema"]["properties"].as_object().into_iter().flatten() {
                hits.extend(standalone_btw(&format!("MCP {name}.{k}"), &visible_md(v["description"].as_str().unwrap_or(""))));
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(hits, Vec::<String>::new(), "say BeatScript where a person reads it (identifiers and code may keep btw)");
        // the check sees what it should, and leaves code and identifiers alone
        assert_eq!(standalone_btw("t", &visible_html("<title>btw</title><p>a <code>#! btw 1</code> line in <a href=\"/css/btw.css\">btw.css</a>, BTW.</p>")).len(), 2);
        assert!(standalone_btw("t", &visible_md("the `btw` command, [a link](https://btw.bitwrap.io/x), data-btw\n```\nbtw jsonld\n```\n")).is_empty());
    }

    /// The language reference answers under its name before the rename, /docs/BTW.md, with the same bytes: the MCP
    /// instructions and copies of llms.txt from before link it.
    #[test]
    fn the_reference_answers_under_its_old_name_too() {
        let dir = std::env::temp_dir().join(format!("bs-docs-btw-md-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("BEATSCRIPT.md"), "# BeatScript\n\nThe language reference.\n").unwrap();
        let d = Docs::new(Some(dir.clone()), None, "https://beatscript.app");
        let ex = Examples { list: Vec::new(), text: HashMap::new() };
        let (Some(Answer::Body(ct_new, new, ..)), Some(Answer::Body(ct_old, old, ..))) = (d.answer("/docs/BEATSCRIPT.md", &ex), d.answer("/docs/BTW.md", &ex)) else { panic!("both are served") };
        assert_eq!((ct_new, &new), (ct_old, &old));
        assert!(ct_new.starts_with("text/markdown"));
        let (a, b) = (d.respond(&Method::GET, &HeaderMap::new(), "/docs/BEATSCRIPT.md", &ex).unwrap(), d.respond(&Method::GET, &HeaderMap::new(), "/docs/BTW.md", &ex).unwrap());
        assert_eq!((a.status(), b.status()), (axum::http::StatusCode::OK, axum::http::StatusCode::OK));
        assert_eq!(a.headers()[header::ETAG], b.headers()[header::ETAG]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A fence tagged ```beatscript or ```btw (its tag before the rename) is BeatScript code: highlighted (data-beatscript,
    /// which docs.js reads) and tried.
    #[test]
    fn both_fence_tags_are_beatscript() {
        for tag in ["beatscript", "btw", "BeatScript"] {
            let html = render_markdown(&format!("```{tag}\ntempo 120\ndeck A: kick x ~ x ~\n```\n")).body;
            assert!(html.contains("<figure class=\"code\" data-beatscript=\"1\" data-try=\"1\">") && html.contains("<span class=\"lang\">BeatScript</span>"), "{tag}: {html}");
        }
        assert!(!render_markdown("```sh\nbeatscript fmt x.beatscript\n```\n").body.contains("data-beatscript"));
    }

    #[test]
    fn programs() {
        assert!(is_program("tempo 120\ndeck A: kick x"));
        assert!(is_program("  deck A: kick"));
        assert!(is_program("x deck A1 :"));
        assert!(!is_program("kick x ~ x"));
        assert!(!is_program("# deck A:"));
    }
}
