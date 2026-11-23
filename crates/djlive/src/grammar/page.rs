//! The Grammar page: every production of `grammar.ebnf` as a railroad diagram, its note, its EBNF, and the productions
//! that use it, cross-linked by name; the token classes after them. No script to run, light and dark. Served with an origin
//! (beatscript.app's `/grammar`) its styles are a stylesheet of their own, [`stylesheet`] at [`STYLESHEET`]: the site's
//! CSP is `style-src 'self'`, which blocks an inline `<style>`, and the page has no `style=` attribute either. Without
//! an origin (the Android app's copy, which no CSP governs) it is one self-contained document, the same styles inline.
//! Served with an origin it also carries itself as JSON-LD ([`linked_data`]: a `DefinedTermSet`, a term per
//! production), the entry the docs' JSON-LD index links to.

use std::collections::BTreeMap;

use djengine::json::{JsonObject, JsonValue};

use super::ebnf::Grammar;
use super::railroad::{class_id, rule_id, svg, CSS};

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Backticked words of a note as code.
fn note_html(s: &str) -> String {
    let mut out = String::new();
    for (i, part) in esc(s).split('`').enumerate() {
        if i % 2 == 1 {
            out.push_str(&format!("<code>{part}</code>"));
        } else {
            out.push_str(part);
        }
    }
    out
}

/// The page's own look, beside the diagrams': the app's dark stage and its light counterpart.
const PAGE_CSS: &str = r#":root{color-scheme:dark;--bg:#1a1a2e;--panel:#16213e;--line:#243055;--ink:#eceef6;--dim:#8d95b0;--link:#88aaff;--accent:#e94560;--code-bg:#12192f}
@media (prefers-color-scheme:light){:root:not([data-theme=dark]){color-scheme:light;--bg:#f7f8fc;--panel:#ffffff;--line:#d7dcec;--ink:#1a1a2e;--dim:#59607a;--link:#2848b8;--accent:#c2304b;--code-bg:#eef1fa}}
:root[data-theme=light]{color-scheme:light;--bg:#f7f8fc;--panel:#ffffff;--line:#d7dcec;--ink:#1a1a2e;--dim:#59607a;--link:#2848b8;--accent:#c2304b;--code-bg:#eef1fa}
*{box-sizing:border-box}
html{scroll-padding-top:16px}
body{margin:0;background:var(--bg);color:var(--ink);font:15px/1.55 system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif}
a{color:var(--link);text-decoration:none}
a:hover{text-decoration:underline}
code,pre{font:13px/1.5 ui-monospace,"SF Mono",SFMono-Regular,Menlo,Monaco,Consolas,"DejaVu Sans Mono",monospace}
code{background:var(--code-bg);padding:1px 4px;border-radius:3px}
header{display:flex;align-items:center;gap:12px;padding:14px 20px;border-bottom:1px solid var(--line)}
header .brand{font-weight:700;color:var(--ink)}
header .brand::before{content:"";display:inline-block;width:10px;height:10px;border-radius:50%;background:var(--accent);margin-right:8px;vertical-align:1px}
main{max-width:1040px;margin:0 auto;padding:8px 20px 64px}
h1{font-size:1.6rem;margin:20px 0 6px}
h2{font-size:1.15rem;margin:40px 0 4px;padding-top:8px;border-top:1px solid var(--line)}
h3{font-size:1rem;margin:0 0 6px;font-family:ui-monospace,"SF Mono",Menlo,Consolas,monospace}
p.lead{color:var(--dim);max-width:72ch}
nav.toc{columns:3 220px;column-gap:24px;margin:16px 0 8px;font-size:14px}
nav.toc div{break-inside:avoid;margin:0 0 12px}
nav.toc strong{display:block;color:var(--dim);font-weight:600;font-size:12px;text-transform:uppercase;letter-spacing:.04em}
nav.toc a{font-family:ui-monospace,"SF Mono",Menlo,Consolas,monospace;font-size:13px;margin-right:8px;display:inline-block}
section.rule{background:var(--panel);border:1px solid var(--line);border-radius:8px;padding:12px 14px;margin:12px 0}
section.rule:target{border-color:var(--accent);box-shadow:0 0 0 1px var(--accent)}
.diagram{overflow-x:auto;padding:4px 0}
p.note{margin:6px 0;color:var(--dim)}
pre.ebnf{margin:6px 0;padding:8px 10px;background:var(--code-bg);border-radius:6px;overflow-x:auto;white-space:pre}
p.used{margin:6px 0 0;font-size:13px;color:var(--dim)}
p.used a{font-family:ui-monospace,"SF Mono",Menlo,Consolas,monospace;margin-right:6px}
dl.classes{display:grid;grid-template-columns:max-content 1fr;gap:6px 16px}
dl.classes dt{font-family:ui-monospace,"SF Mono",Menlo,Consolas,monospace;font-weight:600}
dl.classes dt:target{color:var(--accent)}
dl.classes dd{margin:0;color:var(--dim)}
footer.rights{margin:2rem auto 1.5rem;padding:0 1rem;max-width:72rem;font-size:.85rem;opacity:.8}"#;

/// Who authored a program and what rights it has: the link every page of the site ends with (beatscript.app/authorship).
const FOOTER: &str = "<footer class=\"rights\"><a href=\"/authorship\">Authorship and rights</a> &middot; <a href=\"https://github.com/stackdump/beatscript\">source</a> &middot; Code MIT &middot; programs CC0</footer>\n";

/// Where the served page's stylesheet is ([`stylesheet`]), on the page's own origin.
pub const STYLESHEET: &str = "/grammar.css";

/// The page's styles and the diagrams', light and dark: what the served page links as [`STYLESHEET`].
pub fn stylesheet() -> String {
    format!("{PAGE_CSS}\n{CSS}\n")
}

/// The page. `origin` is where it is served (`https://beatscript.app`, or "" for a page with relative links only, as the
/// phone shows it); `classes` names each token class.
pub fn html(g: &Grammar, origin: &str, classes: &[(&str, &str)]) -> String {
    let origin = origin.trim_end_matches('/');
    // who uses whom
    let mut used_by: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for r in &g.rules {
        let mut refs = Vec::new();
        r.node.refs(&mut refs);
        for n in refs {
            if let Some(t) = g.rules.iter().find(|x| x.name == n) {
                let list = used_by.entry(t.name.as_str()).or_default();
                if !list.contains(&r.name.as_str()) {
                    list.push(r.name.as_str());
                }
            }
        }
    }
    let mut s = String::with_capacity(256 * 1024);
    s.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n<title>Grammar · BeatScript</title>\n");
    s.push_str("<meta name=\"description\" content=\"The grammar of BeatScript, every production as a railroad diagram, generated from grammar.ebnf.\">\n");
    if !origin.is_empty() {
        s.push_str(&format!("<link rel=\"canonical\" href=\"{}/grammar\">\n<link rel=\"alternate\" type=\"text/plain\" href=\"{}/grammar.ebnf\" title=\"grammar.ebnf\">\n", esc(origin), esc(origin)));
        s.push_str(&format!("<script type=\"application/ld+json\">{}</script>\n", crate::progld::script_json(&linked_data(g, origin))));
    }
    if origin.is_empty() {
        s.push_str(&format!("<style>\n{}</style>\n", stylesheet()));
    } else {
        s.push_str(&format!("<link rel=\"stylesheet\" href=\"{STYLESHEET}\">\n"));
    }
    s.push_str("</head>\n<body>\n");
    let home = if origin.is_empty() { "#top".to_string() } else { format!("{origin}/") };
    s.push_str(&format!("<header id=\"top\"><a class=\"brand\" href=\"{}\">BeatScript</a><span>Grammar</span></header>\n<main>\n", esc(&home)));
    s.push_str("<h1>Grammar</h1>\n");
    let ebnf_link = if origin.is_empty() { "<code>grammar.ebnf</code>".to_string() } else { format!("<a href=\"{}/grammar.ebnf\"><code>grammar.ebnf</code></a>", esc(origin)) };
    s.push_str(&format!(
        "<p class=\"lead\">Every production of BeatScript, drawn from {ebnf_link}, the language's one grammar. Read a diagram left to right along the line: a rounded box is text written as it is, a square box is another production (tap it to go there), a dashed box is a kind of token (a name, a number, a note). The grammar is held to the parser both ways: every example program is a sentence of it, and every sentence it writes is a program the parser reads. {} productions, {} kinds of token.</p>\n",
        g.rules.len(),
        g.classes().len()
    ));
    // the table of contents
    s.push_str("<nav class=\"toc\" aria-label=\"Productions\">\n");
    for (title, rules) in g.sections() {
        s.push_str(&format!("<div><strong>{}</strong>", esc(&title)));
        for r in rules {
            s.push_str(&format!("<a href=\"#{}\">{}</a>", esc(&rule_id(&r.name)), esc(&r.name)));
        }
        s.push_str("</div>\n");
    }
    s.push_str("<div><strong>Tokens</strong><a href=\"#tokens\">token classes</a></div>\n</nav>\n");
    for (title, rules) in g.sections() {
        let sid = format!("section-{}", title.to_lowercase().replace(|c: char| !c.is_ascii_alphanumeric(), "-"));
        s.push_str(&format!("<h2 id=\"{}\">{}</h2>\n", esc(&sid), esc(&title)));
        for r in rules {
            let id = rule_id(&r.name);
            s.push_str(&format!("<section class=\"rule\" id=\"{}\">\n<h3><a href=\"#{}\">{}</a></h3>\n", esc(&id), esc(&id), esc(&r.name)));
            s.push_str(&format!("<div class=\"diagram\">{}</div>\n", svg(&r.name, &r.node, g)));
            if let Some(n) = &r.note {
                s.push_str(&format!("<p class=\"note\">{}</p>\n", note_html(n)));
            }
            s.push_str(&format!("<pre class=\"ebnf\">{}</pre>\n", esc(&r.source)));
            if let Some(users) = used_by.get(r.name.as_str()) {
                s.push_str("<p class=\"used\">Used by ");
                for u in users {
                    s.push_str(&format!("<a href=\"#{}\">{}</a>", esc(&rule_id(u)), esc(u)));
                }
                s.push_str("</p>\n");
            }
            s.push_str("</section>\n");
        }
    }
    s.push_str("<h2 id=\"tokens\">Token classes</h2>\n<p class=\"lead\">The upper-case names that are no production: the kinds of token the lexer reads. A comment (<code>#</code> to the end of the line) is not a token.</p>\n<dl class=\"classes\">\n");
    let used = g.classes();
    for (name, what) in classes.iter().filter(|(n, _)| used.contains(*n)) {
        s.push_str(&format!("<dt id=\"{}\">{}</dt><dd>{}</dd>\n", esc(&class_id(name)), esc(name), note_html(what)));
    }
    s.push_str("</dl>\n</main>\n");
    // the footer every beatscript.app page ends with (btwserve's `rights::FOOTER`, the same bytes): a relative link, so
    // the phone's copy (based at beatscript.app) opens the same page
    s.push_str(FOOTER);
    s.push_str("</body>\n</html>\n");
    s
}

/// The page as linked data (schema.org): a `DefinedTermSet` whose terms are the productions, each at its diagram
/// (`/grammar#rule-<name>`) with its note, and the set encoded as `grammar.ebnf`. Only with an origin: the IRIs are
/// absolute, so the phone's page (relative links) carries none.
pub fn linked_data(g: &Grammar, origin: &str) -> JsonValue {
    let origin = origin.trim_end_matches('/');
    let page = format!("{origin}/grammar");
    let mut terms = Vec::new();
    for r in &g.rules {
        let mut t = JsonObject::new();
        t.set("@type", JsonValue::str("DefinedTerm"));
        t.set("@id", JsonValue::str(format!("{page}#{}", rule_id(&r.name))));
        t.set("name", JsonValue::str(r.name.clone()));
        t.set("termCode", JsonValue::str(r.name.clone()));
        if let Some(n) = &r.note {
            t.set("description", JsonValue::str(n.replace('`', "")));
        }
        t.set("inDefinedTermSet", JsonValue::str(page.clone()));
        terms.push(JsonValue::Object(t));
    }
    let mut enc = JsonObject::new();
    enc.set("@type", JsonValue::str("MediaObject"));
    enc.set("name", JsonValue::str("grammar.ebnf"));
    enc.set("contentUrl", JsonValue::str(format!("{origin}/grammar.ebnf")));
    enc.set("encodingFormat", JsonValue::str("text/plain"));
    let mut site = JsonObject::new();
    site.set("@type", JsonValue::str("WebSite"));
    site.set("name", JsonValue::str("BeatScript"));
    site.set("url", JsonValue::str(format!("{origin}/")));
    let mut d = JsonObject::new();
    d.set("@context", JsonValue::str("https://schema.org"));
    d.set("@type", JsonValue::str("DefinedTermSet"));
    d.set("@id", JsonValue::str(page.clone()));
    d.set("name", JsonValue::str("The grammar of BeatScript"));
    d.set("description", JsonValue::str("Every production of BeatScript as a railroad diagram, generated from grammar.ebnf, the language's one grammar."));
    d.set("url", JsonValue::str(page));
    d.set("inLanguage", JsonValue::str("en"));
    d.set("isPartOf", JsonValue::Object(site));
    d.set("encoding", JsonValue::Object(enc));
    d.set("hasDefinedTerm", JsonValue::Array(terms));
    JsonValue::Object(d)
}

/// The page for the grammar in the build, as the site serves it.
pub fn grammar_page(origin: &str) -> String {
    html(&super::grammar(), origin, super::recognize::CLASSES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_production_is_on_the_page_and_every_link_lands() {
        let g = crate::grammar::grammar();
        let page = grammar_page("https://beatscript.app");
        for r in &g.rules {
            assert!(page.contains(&format!("<section class=\"rule\" id=\"rule-{}\">", r.name)), "no diagram for {}", r.name);
        }
        // every href="#..." points at an id on the page
        let ids: std::collections::BTreeSet<&str> = page.match_indices(" id=\"").map(|(i, _)| { let rest = &page[i + 5..]; &rest[..rest.find('"').unwrap()] }).collect();
        for (i, _) in page.match_indices("href=\"#") {
            let rest = &page[i + 7..];
            let id = &rest[..rest.find('"').unwrap()];
            assert!(ids.contains(id), "a link to #{id}, which is not on the page");
        }
        for c in g.classes() {
            assert!(page.contains(&format!("<dt id=\"class-{c}\">")), "no entry for the class {c}");
        }
        assert!(page.contains("<title>Grammar · BeatScript</title>"));
        // served, its styles are a stylesheet on the same origin (the site's CSP is style-src 'self'): nothing inline
        assert!(page.contains("<link rel=\"stylesheet\" href=\"/grammar.css\">"));
        assert!(!page.contains("<style") && !page.contains("style="), "an inline style on the served page");
        let css = stylesheet();
        assert!(css.contains("prefers-color-scheme:light") && css.contains(":root[data-theme=light]") && css.contains(".rr .rr-term rect") && !css.contains('<'));
        // the phone's: relative links only, and self-contained (the same styles inline, no stylesheet to fetch)
        let phone = grammar_page("");
        assert!(!phone.contains("https://beatscript.app"));
        assert!(phone.contains(&format!("<style>\n{css}</style>")) && !phone.contains("grammar.css"));
        // the served page is linked data too: a DefinedTermSet, a term per production at its diagram
        let ld = &page[page.find("<script type=\"application/ld+json\">").expect("no JSON-LD") + 35..];
        let ld = &ld[..ld.find("</script>").unwrap()];
        assert!(ld.starts_with("{\"@context\":\"https://schema.org\",\"@type\":\"DefinedTermSet\",\"@id\":\"https://beatscript.app/grammar\""), "{ld}");
        assert_eq!(ld.matches("\"@type\":\"DefinedTerm\"").count(), g.rules.len());
        assert!(ld.contains("\"@id\":\"https://beatscript.app/grammar#rule-pname\",\"name\":\"pname\""));
        assert!(ld.contains("\"contentUrl\":\"https://beatscript.app/grammar.ebnf\"") && !ld.contains('<'));
        assert!(!grammar_page("").contains("application/ld+json"));
    }
}
