//! The server-rendered pages: `/p/<cid>/tree` and `/p/<a>/compare/<b>`. Plain HTML that reads without JavaScript,
//! with the same JSON-LD document a processor gets embedded in the head.

use serde_json::Value;

use super::{compare_url, tree_url, Change, Edge, Node, Tree};

/// `html/template`'s escaping of text and attribute values.
pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '&' => o.push_str("&amp;"),
            '\'' => o.push_str("&#39;"),
            '"' => o.push_str("&#34;"),
            c => o.push(c),
        }
    }
    o
}

/// A JSON-LD document inside `<script>`: `<`, `>` and `&` escaped so nothing in it can close the script.
fn script_json(v: &Value) -> String {
    serde_json::to_string(v).unwrap().replace('<', "\\u003c").replace('>', "\\u003e").replace('&', "\\u0026")
}

/// The pages' styles, served at [`STYLESHEET`] (the site's CSP is `style-src 'self'`: no inline `<style>`).
pub const STYLE: &str = ":root{color-scheme:dark}\nbody{margin:0;background:#070b12;color:#e6ecf5;font:16px/1.5 system-ui,-apple-system,Segoe UI,sans-serif}\nmain{width:min(94vw,820px);margin:0 auto;padding:28px 0 48px}\na{color:#5eead4;text-decoration:none}a:hover{text-decoration:underline}\nh1{font-size:1.35rem;margin:0 0 4px}h2{font-size:1rem;margin:28px 0 8px;color:#8d9bb5;text-transform:uppercase;letter-spacing:.06em}\np.sub{margin:0 0 12px;color:#8d9bb5}\n.node{border:1px solid #1d2a42;border-radius:10px;padding:10px 14px;margin:6px 0;background:#0b1220}\n.node.me{border-color:#5eead4;box-shadow:0 0 0 1px #5eead4 inset}\n.node .n{font-weight:600}.node .m{color:#8d9bb5;font-size:.88rem}\n.edge{margin:2px 0 2px 18px;padding:2px 0 2px 14px;border-left:2px solid #1d2a42;color:#c9d4e6;font-size:.9rem}\n.edge ul{margin:2px 0;padding-left:18px}.edge .same{color:#a3e635}\nul.tree{list-style:none;padding-left:0;margin:0}ul.tree ul{list-style:none;padding-left:22px;margin:0}\ntable{border-collapse:collapse;width:100%;font-size:.92rem}td,th{border-bottom:1px solid #1d2a42;padding:6px 8px;text-align:left;vertical-align:top}\n.none{color:#8d9bb5}\nfooter.rights{margin:2rem auto 1.5rem;padding:0 1rem;max-width:72rem;font-size:.85rem;opacity:.8}\n";

/// Where [`STYLE`] is served.
pub const STYLESHEET: &str = "/lineage.css";

fn head(title: &str, canonical: &str, ld: &Value) -> String {
    let t = esc(title);
    let c = esc(canonical);
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{t}</title>\n<link rel=\"canonical\" href=\"{c}\">\n<link rel=\"alternate\" type=\"application/ld+json\" href=\"{c}.jsonld\">\n<meta property=\"og:title\" content=\"{t}\">\n<meta property=\"og:url\" content=\"{c}\">\n<meta property=\"og:site_name\" content=\"BeatScript\">\n<script type=\"application/ld+json\">{}</script>\n<link rel=\"stylesheet\" href=\"{STYLESHEET}\">\n</head>\n<body>\n<main>\n",
        script_json(ld)
    )
}

/// The pages' end: the footer every page has (`crate::rights`), then the document's close.
const FOOT: &str = "</main>\n<footer class=\"rights\"><a href=\"/authorship\">Authorship and rights</a> &middot; <a href=\"https://github.com/stackdump/beatscript\">source</a> &middot; Code MIT &middot; programs CC0</footer>\n</body>\n</html>\n";

fn meta_line(n: &Node) -> String {
    let mut parts = Vec::new();
    if let Some(t) = n.tempo {
        parts.push(format!("{} BPM", fmt_num(t)));
    }
    if !n.key.is_empty() {
        parts.push(format!("key {}", n.key));
    }
    parts.push(n.license.clone());
    if n.example {
        parts.push("example".into());
    }
    if !n.credits.is_empty() {
        parts.push(format!("credits: {}", n.credits.join(", ")));
    }
    if !n.created.is_empty() {
        parts.push(n.created.chars().take(10).collect());
    }
    parts.push(match n.fork_count {
        0 => "no forks".into(),
        1 => "1 fork".into(),
        k => format!("{k} forks"),
    });
    esc(&parts.join(" · "))
}

fn fmt_num(x: f64) -> String {
    if x.fract() == 0.0 {
        format!("{}", x as i64)
    } else {
        format!("{x}")
    }
}

fn node_html(origin: &str, n: &Node, me: bool) -> String {
    let cls = if me { "node me" } else { "node" };
    let tree = if me { String::new() } else { format!(" · <a href=\"{}\">tree</a>", esc(&tree_url(origin, &n.cid))) };
    format!(
        "<div class=\"{cls}\"><a class=\"n\" href=\"{}\">{}</a> <span class=\"m\">{}</span><br><span class=\"m\"><code>{}</code> · <a href=\"/#{}\">open</a>{tree}</span></div>\n",
        esc(&format!("{origin}/p/{}", n.cid)),
        esc(&n.label()),
        meta_line(n),
        esc(&super::short_cid(&n.cid)),
        esc(&n.cid),
    )
}

/// What changed on an edge, as the lines under it.
fn change_html(origin: &str, e: &Edge) -> String {
    let cmp = format!(" <a href=\"{}\">compare</a>", esc(&compare_url(origin, &e.from, &e.to)));
    match &e.change {
        None => format!("<div class=\"edge\"><span class=\"none\">forked (what changed is not worked out yet)</span>{cmp}</div>\n"),
        Some(c) if c.same => format!("<div class=\"edge\"><span class=\"same\">the same song</span> (rewritten, not changed){cmp}</div>\n"),
        Some(c) => {
            let mut s = String::from("<div class=\"edge\">forked:");
            s.push_str(&cmp);
            s.push_str(&lines_html(c));
            s.push_str("</div>\n");
            s
        }
    }
}

fn lines_html(c: &Change) -> String {
    if c.summary.is_empty() {
        return format!("<ul><li>{}</li></ul>", esc(&c.why));
    }
    let mut s = String::from("<ul>");
    for l in c.summary.iter().take(12) {
        s.push_str(&format!("<li>{}</li>", esc(l)));
    }
    if c.summary.len() > 12 {
        s.push_str(&format!("<li class=\"none\">and {} more</li>", c.summary.len() - 12));
    }
    s.push_str("</ul>");
    s
}

/// The forks of `at` in the tree, nested.
fn forks_html(origin: &str, t: &Tree, at: &str, out: &mut String) {
    let kids: Vec<&Node> = t.forks.iter().filter(|n| n.forked_from == at).collect();
    if kids.is_empty() {
        return;
    }
    out.push_str("<ul class=\"tree\">\n");
    for k in kids {
        out.push_str("<li>");
        if let Some(e) = t.edge_to(&k.cid) {
            out.push_str(&change_html(origin, e));
        }
        out.push_str(&node_html(origin, k, false));
        forks_html(origin, t, &k.cid, out);
        out.push_str("</li>\n");
    }
    out.push_str("</ul>\n");
}

/// `/p/<cid>/tree`.
pub fn tree_page(origin: &str, t: &Tree, ld: &Value) -> String {
    let r = &t.root;
    let title = format!("{} — family tree · BeatScript", r.label());
    let mut s = head(&title, &tree_url(origin, &r.cid), ld);
    s.push_str(&format!("<h1>{}</h1>\n", esc(&r.label())));
    let fc = match r.fork_count {
        0 => "no forks yet".to_string(),
        1 => "1 fork".to_string(),
        k => format!("{k} forks"),
    };
    let desc = if t.forks.len() > r.fork_count { format!("{fc}, {} in all generations", t.forks.len()) } else { fc };
    s.push_str(&format!("<p class=\"sub\">Family tree · {} · {} · <a href=\"{}\">the program</a></p>\n", esc(&desc), plural(t.ancestors.len(), "ancestor"), esc(&format!("{origin}/p/{}", r.cid))));
    s.push_str("<h2>Lineage</h2>\n");
    if t.ancestors_cut {
        s.push_str("<p class=\"none\">Forked from a program that is not public here.</p>\n");
    } else if t.ancestors.is_empty() {
        s.push_str("<p class=\"none\">An original: it was not forked from anything.</p>\n");
    }
    for a in t.ancestors.iter().rev() {
        s.push_str(&node_html(origin, a, false));
        let child = t.edges.iter().find(|e| e.from == a.cid && (e.to == r.cid || t.ancestors.iter().any(|x| x.cid == e.to)));
        if let Some(e) = child {
            s.push_str(&change_html(origin, e));
        }
    }
    s.push_str(&node_html(origin, r, true));
    let mut f = String::new();
    forks_html(origin, t, &r.cid, &mut f);
    s.push_str(&f);
    if !t.siblings.is_empty() {
        s.push_str("<h2>Siblings</h2>\n<p class=\"sub\">Other forks of the same parent.</p>\n");
        for n in &t.siblings {
            if let Some(e) = t.edge_to(&n.cid) {
                s.push_str(&change_html(origin, e));
            }
            s.push_str(&node_html(origin, n, false));
        }
    }
    if t.truncated {
        s.push_str("<p class=\"none\">This tree is large; only part of it is shown.</p>\n");
    }
    s.push_str(&format!("<p class=\"sub\">Also as <a href=\"{}.jsonld\">JSON-LD</a>.</p>\n", esc(&tree_url(origin, &r.cid))));
    s.push_str(FOOT);
    s
}

fn plural(n: usize, w: &str) -> String {
    match n {
        0 => format!("no {w}s"),
        1 => format!("1 {w}"),
        k => format!("{k} {w}s"),
    }
}

/// `/p/<a>/compare/<b>`.
pub fn compare_page(origin: &str, a: &Node, b: &Node, c: Option<&Change>, ld: &Value) -> String {
    let title = format!("{} → {} · BeatScript", a.label(), b.label());
    let mut s = head(&title, &compare_url(origin, &a.cid, &b.cid), ld);
    s.push_str(&format!("<h1>Compare</h1>\n<p class=\"sub\">What changed in music from the first program to the second. <a href=\"{}\">Swap</a></p>\n", esc(&compare_url(origin, &b.cid, &a.cid))));
    s.push_str(&node_html(origin, a, false));
    match c {
        None => s.push_str("<div class=\"edge\"><span class=\"none\">These two could not be compared.</span></div>\n"),
        Some(c) if c.text_same => s.push_str("<div class=\"edge\"><span class=\"same\">the same program</span></div>\n"),
        Some(c) if c.same => s.push_str(&format!("<div class=\"edge\"><span class=\"same\">the same song</span>: {}</div>\n", esc(&c.why))),
        Some(c) => s.push_str(&format!("<div class=\"edge\">{}</div>\n", lines_html(c))),
    }
    s.push_str(&node_html(origin, b, false));
    if let Some(song) = c.and_then(|c| c.report["diff"]["song"].as_array().cloned()).filter(|x| !x.is_empty()) {
        s.push_str("<h2>Song settings</h2>\n<table><tr><th>setting</th><th>first</th><th>second</th></tr>\n");
        for x in song {
            let show = |v: &Value| match v {
                Value::Null => "none".to_string(),
                Value::String(s) => s.clone(),
                o => o.to_string(),
            };
            s.push_str(&format!("<tr><td>{}</td><td>{}</td><td>{}</td></tr>\n", esc(x["field"].as_str().unwrap_or("")), esc(&show(&x["old"])), esc(&show(&x["new"]))));
        }
        s.push_str("</table>\n");
    }
    s.push_str(&format!("<p class=\"sub\">Also as <a href=\"{}.jsonld\">JSON-LD</a>.</p>\n", esc(&compare_url(origin, &a.cid, &b.cid))));
    s.push_str(FOOT);
    s
}

/// The page for a CID with no tree to show: unknown here, or not public. The same words for both, so a private
/// program's existence is not told.
pub fn missing_page(origin: &str) -> String {
    let ld = serde_json::json!({});
    let mut s = head("Not found · BeatScript", &format!("{origin}/"), &ld);
    s.push_str("<h1>No such program here</h1>\n<p class=\"sub\">No public program with that CID is stored on this site.</p>\n<p><a href=\"/\">Open BeatScript</a></p>\n");
    s.push_str(FOOT);
    s
}

/// A program page (`/p/<cid>`) with a line naming its forks and linking its family tree, before `</main>`.
pub fn with_fork_line(html: &str, cid: &str, forks: usize) -> String {
    let n = match forks {
        0 => "no forks yet".to_string(),
        1 => "1 fork".to_string(),
        k => format!("{k} forks"),
    };
    let line = format!("<p class=\"lineage\"><a href=\"/p/{}/tree\">Family tree</a> · {n}</p>\n", esc(cid));
    match html.rfind("</main>") {
        Some(at) => format!("{}{line}{}", &html[..at], &html[at..]),
        None => html.to_string(),
    }
}
