//! The embeddable player (project-datum 6664): one player, several ways in.
//!
//!   GET /embed/<cid>[.<packed>][?compact=1&theme=light|dark]   a small page another site frames: play/stop, the
//!                                                               track's name, the section and bar playing, its rights
//!
//!   GET /oembed?url=<a /p/ or /c/ link>&format=json[&maxwidth=&maxheight=]   oEmbed 1.0, type rich: the iframe
//!
//! and on every embeddable program's `/p/` and `/c/` page, `<link rel="alternate" type="application/json+oembed">`, so
//! a pasted link auto-embeds where oEmbed discovery is read (WordPress, Discourse, Notion and the like).
//!
//! The page plays the program in the browser with the same wasm engine the editor uses (`/js/embed-player.js`,
//! beats-bitwrap-io `btwsite/web`), started only by a click: there is no audio file and no autoplay. It carries the
//! program as the editor's `<beatscript-view>` block (djlive `progld`), so the player reads the same JSON-LD the editor
//! and the phone read, and has no inline script or style (the CSP is the site's own).
//!
//! Only what the site stores can be embedded (the storage rule, `btwserve::gate`): a public, Creative Commons,
//! sample-free program that is stored here, or an example. Anything else, including a `<cid>.<packed>` link to a
//! program the site does not keep, is a 404: an embed never stores, writes or signs anything. Its response is the only
//! one any site may frame (`frame-ancestors *`, no `X-Frame-Options`); every other route keeps `frame-ancestors 'none'`.

use super::*;

/// The site's CSP with one change: any page may frame this one.
static FRAME_CSP: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| crate::CSP.replace("frame-ancestors 'none'", "frame-ancestors *"));

/// Where the embed lives: the one path prefix `site_headers` lets other sites frame.
pub(crate) const PREFIX: &str = "/embed/";

/// The frame's size, (width, height): the player, and its one-line form (`compact`).
pub(super) const SIZE: (u32, u32) = (480, 152);
pub(super) const SIZE_COMPACT: (u32, u32) = (480, 80);

/// What the frame may do, as the element (embed.js) and oEmbed's html set it: run, fetch the engine from its own
/// origin, and open links in a new tab.
pub(super) const SANDBOX: &str = "allow-scripts allow-same-origin allow-popups allow-popups-to-escape-sandbox";

/// Whether `p` may be embedded: it checks, is public and sample-free (`Prog::state`), and is stored here (or is an
/// example) under a Creative Commons licence. A `<cid>.<packed>` link the site does not keep is not.
pub(super) fn embeddable(p: &Prog) -> bool {
    if p.state() != St::Ok || !p.known {
        return false;
    }
    let lic = if p.meta.license.trim().is_empty() { btwserve::gate::DEFAULT_LICENSE } else { p.meta.license.trim() };
    btwserve::gate::cc_license(lic).is_some()
}

/// The query's `compact` and `theme` (`light` or `dark`, the default).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Look {
    pub compact: bool,
    pub light: bool,
}

impl Look {
    pub fn from_query(q: Option<&str>) -> Look {
        let mut l = Look::default();
        for kv in q.unwrap_or("").split('&') {
            let (k, v) = kv.split_once('=').unwrap_or((kv, "1"));
            match k {
                "compact" => l.compact = matches!(v, "1" | "true" | "yes" | ""),
                "theme" => l.light = v == "light",
                _ => {}
            }
        }
        l
    }
}

/// The program's facts line: key, tempo, length.
fn facts(r: &Report) -> String {
    let mut parts = Vec::new();
    if !r.key.is_empty() {
        parts.push(lex::clean_text(&r.key, 24));
    }
    if let Some(t) = r.tempo {
        parts.push(format!("{} BPM", lex::fmt_bpm(t)));
    }
    if r.song_bars > 0 {
        parts.push(format!("{} bars", r.song_bars));
    }
    parts.join(" · ")
}

/// The rights line: the licence (linked to its deed), the author and credits the store keeps, /authorship and the source.
fn rights_html(origin: &str, m: &Meta) -> String {
    let id = if m.license.trim().is_empty() { btwserve::gate::DEFAULT_LICENSE } else { m.license.trim() };
    let badge = esc(&lex::license_badge(id));
    let mut s = match djlive::progld::license_iri(id) {
        Some(u) => format!("<a class=\"bsp-lic\" href=\"{}\" target=\"_blank\" rel=\"noopener license\">{badge}</a>", esc_url(u)),
        None => format!("<span class=\"bsp-lic\">{badge}</span>"),
    };
    let author = lex::clean_text(&m.author, 60);
    if !author.is_empty() {
        s.push_str(&format!(" · by {}", esc(&author)));
    }
    let credits: Vec<String> = m.credits.iter().map(|c| lex::clean_text(c, 60)).filter(|c| !c.is_empty()).collect();
    if !credits.is_empty() {
        s.push_str(&format!(" · credits: {}", esc(&credits.join(", "))));
    }
    s.push_str(&format!(" · <a href=\"{}\" target=\"_blank\" rel=\"noopener\">rights</a>", esc_url(&btwserve::rights::url(origin))));
    s.push_str(&format!(" · <a href=\"{}\" target=\"_blank\" rel=\"noopener\">open source</a>", esc_url(btwserve::rights::SOURCE_URL)));
    s
}

/// The embed page of an embeddable program.
pub(super) fn page(origin: &str, p: &Prog, look: Look) -> String {
    let r = p.report.clone().unwrap();
    let name = esc(&p.label());
    let cid = esc(&p.cid);
    let page = esc_url(&format!("{origin}/p/{}", p.cid));
    let editor = esc_url(&format!("{origin}/?cid={}", p.cid));
    let mut class = String::from("bsp");
    class.push_str(if look.light { " theme-light" } else { " theme-dark" });
    if look.compact {
        class.push_str(" compact");
    }
    let mut s = String::with_capacity(4096);
    s.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n<meta name=\"robots\" content=\"noindex\">\n");
    s.push_str(&format!("<title>{name} · BeatScript</title>\n<link rel=\"canonical\" href=\"{page}\">\n"));
    s.push_str("<link rel=\"stylesheet\" href=\"/css/embed.css\">\n<script type=\"module\" src=\"/js/embed-player.js\"></script>\n</head>\n");
    s.push_str(&format!("<body class=\"{class}\">\n<main class=\"bsp-card\" data-cid=\"{cid}\">\n"));
    s.push_str(&format!("<button type=\"button\" class=\"bsp-play\" id=\"bsp-play\" aria-label=\"Play {name}\" disabled><span class=\"bsp-ic\" aria-hidden=\"true\"></span></button>\n"));
    s.push_str("<div class=\"bsp-body\">\n");
    s.push_str(&format!("<a class=\"bsp-name\" href=\"{page}\" target=\"_blank\" rel=\"noopener\">{name}</a>\n"));
    s.push_str("<p class=\"bsp-pos\"><span class=\"bsp-sec\" id=\"bsp-sec\">not playing</span> <span class=\"bsp-bar\" id=\"bsp-bar\"></span> <span class=\"bsp-state\" id=\"bsp-state\" role=\"status\" aria-live=\"polite\"></span></p>\n");
    let f = facts(&r);
    if !f.is_empty() {
        s.push_str(&format!("<p class=\"bsp-facts\">{}</p>\n", esc(&f)));
    }
    // the doc comments (6662): one line for the section playing, which the player keeps up; until play, the first
    // documented section's. Only when the shared text documented a section (its `#` lines are kept beside the text).
    if p.meta.docs.iter().any(|(k, _, _)| k == "section") {
        let first = r.sections.iter().find_map(|x| p.meta.section_doc(&x.name)).map(|d| lex::clean_text(d, 160)).unwrap_or_default();
        s.push_str(&format!("<p class=\"bsp-doc\" id=\"bsp-doc\">{}</p>\n", esc(&first)));
    }
    s.push_str(&format!("<p class=\"bsp-rights\">{}</p>\n</div>\n", rights_html(origin, &p.meta)));
    s.push_str(&format!("<a class=\"bsp-open\" href=\"{editor}\" target=\"_blank\" rel=\"noopener\" title=\"Open in the BeatScript editor\">Open in BeatScript <span aria-hidden=\"true\">&#8599;</span></a>\n</main>\n"));
    if let Some(v) = program_view(p, None) {
        s.push_str(&v);
        s.push('\n');
    }
    s.push_str("</body>\n</html>\n");
    s
}

/// `GET|HEAD /embed/<cid>[.<packed>]`.
pub(super) async fn serve(State(c): State<Arc<Cards>>, req: Request) -> Response<Body> {
    let path = crate::site::percent_decode(req.uri().path());
    let name = path.strip_prefix(PREFIX).unwrap_or("");
    let (cid, packed) = name.split_once('.').unwrap_or((name, ""));
    let mut h = HeaderMap::new();
    h.insert("x-robots-tag", HeaderValue::from_static("noindex"));
    if !valid_cid(cid) || name.contains('/') {
        return not_found(h);
    }
    let addr = addr_of(&req);
    let p = match c.resolve(Some(&addr), cid, packed).await {
        Ok(p) => p,
        Err(e) => return unavailable(&e),
    };
    if !embeddable(&p) {
        return not_found(h);
    }
    let look = Look::from_query(req.uri().query());
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(CACHE_PAGE));
    h.insert("content-security-policy", HeaderValue::from_str(&FRAME_CSP).unwrap());
    h.insert("x-btw-cid", HeaderValue::from_str(&p.cid).unwrap());
    written(req.method(), h, page(&c.origin, &p, look).into_bytes())
}

// ---------- playable cards (/c/<cid>) ----------

/// What a `/c/` page's head says to make its card play: the frame (twitter:player) and the rendered file (og:audio).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Player {
    pub url: String,
    pub width: u32,
    pub height: u32,
    /// The existing render route (`/api/render/<cid>?format=opus`): cached by CID, immutable once rendered, and only
    /// for a program the site stores (it is 404 for anything else).
    pub audio: String,
}

impl Player {
    pub(super) fn of(origin: &str, p: &Prog) -> Player {
        Player { url: format!("{origin}/embed/{}", p.cid), width: SIZE.0, height: SIZE.1, audio: btwserve::opendata::urls(origin, &p.cid).render }
    }
}

/// The tags, after the card's twitter:image (`twitter:card` itself says `player` in [`super::page_html`]).
pub(super) fn player_meta(p: &Player) -> String {
    let (u, a) = (esc(&p.url), esc(&p.audio));
    format!(
        "<meta name=\"twitter:player\" content=\"{u}\">\n<meta name=\"twitter:player:width\" content=\"{}\">\n<meta name=\"twitter:player:height\" content=\"{}\">\n<meta property=\"og:audio\" content=\"{a}\">\n<meta property=\"og:audio:secure_url\" content=\"{a}\">\n<meta property=\"og:audio:type\" content=\"audio/ogg\">\n",
        p.width, p.height
    )
}

// ---------- oEmbed ----------

/// A query value, `+` and `%XX` decoded.
fn query_value(q: Option<&str>, key: &str) -> Option<String> {
    q?.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        (k == key).then(|| crate::site::percent_decode(&v.replace('+', " ")))
    })
}

/// A query component, percent-encoded (everything but the unreserved characters).
pub(super) fn enc(s: &str) -> String {
    let mut o = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}

/// The program a page link of this site names: `<origin>/p/<cid>`, `<origin>/c/<cid>` or `<origin>/c/<cid>.<packed>`
/// (a query or fragment is ignored). None for anything else, including another site's link.
pub(super) fn linked(origin: &str, url: &str) -> Option<(String, String)> {
    let rest = url.trim().strip_prefix(origin)?;
    let path = rest.split(['?', '#']).next().unwrap_or("");
    let path = path.strip_suffix('/').unwrap_or(path);
    let (short, name) = if let Some(n) = path.strip_prefix("/p/") { (true, n) } else { (false, path.strip_prefix("/c/")?) };
    let (cid, packed) = name.split_once('.').unwrap_or((name, ""));
    if !valid_cid(cid) || (short && !packed.is_empty()) || packed.contains(['/', '.']) {
        return None;
    }
    Some((cid.to_string(), packed.to_string()))
}

/// The frame as oEmbed's `html` (and the docs' copy-paste form): no style attribute, which a host's CSP may refuse.
pub(super) fn iframe_html(origin: &str, p: &Prog, (w, h): (u32, u32), compact: bool) -> String {
    let src = format!("{origin}/embed/{}{}", p.cid, if compact { "?compact=1" } else { "" });
    format!(
        "<iframe src=\"{}\" width=\"{w}\" height=\"{h}\" title=\"{} · BeatScript player\" sandbox=\"{SANDBOX}\" loading=\"lazy\" referrerpolicy=\"strict-origin-when-cross-origin\" frameborder=\"0\"></iframe>",
        esc_url(&src),
        esc(&p.label())
    )
}

/// Who made it, as the store keeps it: the author, else the credits.
fn author_of(m: &Meta) -> String {
    let a = lex::clean_text(&m.author, 80);
    if !a.is_empty() {
        return a;
    }
    m.credits.iter().map(|c| lex::clean_text(c, 60)).filter(|c| !c.is_empty()).collect::<Vec<_>>().join(", ")
}

/// The oEmbed answer for an embeddable program, sized within `max` (maxwidth, maxheight) when they are given.
pub(super) fn oembed_json(origin: &str, p: &Prog, max: (Option<u32>, Option<u32>)) -> Value {
    let mut w = SIZE.0.min(max.0.unwrap_or(u32::MAX));
    let mut compact = false;
    let mut h = SIZE.1;
    if let Some(mh) = max.1 {
        if mh < SIZE.1 {
            compact = true;
            h = SIZE_COMPACT.1.min(mh);
        }
    }
    w = w.max(1);
    h = h.max(1);
    let lic = if p.meta.license.trim().is_empty() { btwserve::gate::DEFAULT_LICENSE } else { p.meta.license.trim() };
    let mut o = Map::new();
    o.insert("version".into(), json!("1.0"));
    o.insert("type".into(), json!("rich"));
    o.insert("provider_name".into(), json!("BeatScript"));
    o.insert("provider_url".into(), json!(format!("{origin}/")));
    o.insert("title".into(), json!(p.label()));
    let author = author_of(&p.meta);
    if !author.is_empty() {
        o.insert("author_name".into(), json!(author));
    }
    o.insert("html".into(), json!(iframe_html(origin, p, (w, h), compact)));
    o.insert("width".into(), json!(w));
    o.insert("height".into(), json!(h));
    o.insert("thumbnail_url".into(), json!(format!("{origin}/card/{}.png", p.cid)));
    o.insert("thumbnail_width".into(), json!(1200));
    o.insert("thumbnail_height".into(), json!(630));
    o.insert("cache_age".into(), json!(3600));
    // beyond oEmbed 1.0 (consumers ignore what they do not know): the rights and the program's own pages
    o.insert("license".into(), json!(lic));
    if let Some(u) = djlive::progld::license_iri(lic) {
        o.insert("license_url".into(), json!(u));
    }
    o.insert("cid".into(), json!(p.cid));
    o.insert("url".into(), json!(format!("{origin}/p/{}", p.cid)));
    Value::Object(o)
}

/// The discovery tag a page's head carries when its program embeds (`page_url` is the page's own link).
pub(super) fn discovery(origin: &str, p: &Prog, page_url: &str) -> String {
    format!(
        "<link rel=\"alternate\" type=\"application/json+oembed\" href=\"{}\" title=\"{}\">\n",
        esc_url(&format!("{origin}/oembed?url={}&format=json", enc(page_url))),
        esc(&p.label())
    )
}

fn json_error(status: u16, code: &str, message: &str) -> Response<Body> {
    let mut r = write_error(status, code, message, 0, None);
    r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

/// `GET|HEAD /oembed?url=<a /p/ or /c/ link>&format=json[&maxwidth=&maxheight=]`.
async fn oembed(State(c): State<Arc<Cards>>, req: Request) -> Response<Body> {
    let q = req.uri().query();
    let format = query_value(q, "format").unwrap_or_else(|| "json".into());
    if format != "json" {
        // oEmbed 1.0 §2.3.4: a format the provider does not serve is 501
        return json_error(501, "not_implemented", "only format=json is served");
    }
    let Some(url) = query_value(q, "url").filter(|u| !u.is_empty()) else {
        return json_error(400, "bad_request", "url= is needed: a https://beatscript.app/p/<CID> or /c/<CID> link");
    };
    let Some((cid, packed)) = linked(&c.origin, &url) else {
        return json_error(404, "not_found", "not a link to a program's page on this site");
    };
    let addr = addr_of(&req);
    let p = match c.resolve(Some(&addr), &cid, &packed).await {
        Ok(p) => p,
        Err(e) => return unavailable(&e),
    };
    if !embeddable(&p) {
        return json_error(404, "not_found", "no stored Creative Commons program at that link");
    }
    let num = |k: &str| query_value(q, k).and_then(|v| v.trim().parse::<u32>().ok()).filter(|n| *n > 0);
    let body = go_json(&oembed_json(&c.origin, &p, (num("maxwidth"), num("maxheight")))) + "\n";
    let mut h = HeaderMap::new();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json; charset=utf-8"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(CACHE_PAGE));
    h.insert("access-control-allow-origin", HeaderValue::from_static("*"));
    written(req.method(), h, body.into_bytes())
}

pub(super) fn routes() -> Router<Arc<Cards>> {
    Router::new().route("/embed/{*rest}", only!(serve, GET, HEAD)).route("/oembed", only!(oembed, GET, HEAD))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prog(text: &str, meta: Value) -> Prog {
        let rep: Value = serde_json::from_str(&btwapi::check_json(text)).unwrap();
        let r = Report::from_json(&rep);
        Prog { canonical: r.canonical.clone(), report: Some(Arc::new(r.clone())), meta: Meta::from_json(&meta), known: true, ..Prog::bare(&r.cid, St::Ok) }
    }

    const TEXT: &str = "tempo 128\nkey am\nsection groove 2 bars {\n  deck A: kick x ~ x ~\n}\narrange groove\n";

    #[test]
    fn only_a_kept_creative_commons_program_embeds() {
        assert!(embeddable(&prog(TEXT, json!({"license": "CC0-1.0", "visibility": "public"}))));
        assert!(embeddable(&prog(TEXT, json!({"license": "CC-BY-4.0", "visibility": "public"}))));
        assert!(embeddable(&prog(TEXT, json!({}))), "no licence is the default, CC0");
        assert!(!embeddable(&prog(TEXT, json!({"license": "all-rights-reserved"}))));
        assert!(!embeddable(&prog(TEXT, json!({"visibility": "private"}))));
        let link_only = Prog { known: false, ..prog(TEXT, json!({})) };
        assert!(!embeddable(&link_only), "a link the site does not keep");
        assert!(!embeddable(&Prog::bare("zX", St::Unknown)));
    }

    #[test]
    fn the_page_has_no_inline_script_or_style_and_says_its_rights() {
        let p = prog(TEXT, json!({"license": "CC-BY-4.0", "visibility": "public", "name": "night <bus>", "author": "ann", "credits": ["bo", " "]}));
        let html = page("https://beatscript.app", &p, Look::default());
        assert!(!html.contains("style=") && !html.contains("<style"), "inline style: {html}");
        // every script is the module or the program's JSON-LD block (data, never run)
        for (i, _) in html.match_indices("<script") {
            let tag = &html[i..i + html[i..].find('>').unwrap()];
            assert!(tag.contains(" src=\"/js/embed-player.js\"") || tag.contains("type=\"application/ld+json\""), "inline script: {tag}");
        }
        assert!(!["onclick=", "onload=", "onerror=", "javascript:"].iter().any(|a| html.contains(a)), "inline handler");
        assert!(html.contains("<title>night &lt;bus&gt; · BeatScript</title>"), "{html}");
        assert!(html.contains("href=\"https://creativecommons.org/licenses/by/4.0/\""), "{html}");
        assert!(html.contains(" · by ann · credits: bo · "), "{html}");
        assert!(html.contains("href=\"https://github.com/stackdump/beatscript\" target=\"_blank\" rel=\"noopener\">open source</a>"), "the source link: {html}");
        assert!(html.contains(&format!("href=\"https://beatscript.app/?cid={}\"", p.cid)), "the editor link");
        assert!(html.contains("id=\"bsp-play\"") && html.contains(" disabled>"), "play starts disabled: the script enables it");
        assert!(html.contains("am · 128 BPM · 2 bars"), "{html}");
        let v = djlive::progld::read_view(&html, true).unwrap();
        assert_eq!(v.cid, p.cid);
        assert!(html.contains("<body class=\"bsp theme-dark\">"));
        assert!(page("https://beatscript.app", &p, Look::from_query(Some("compact=1&theme=light"))).contains("<body class=\"bsp theme-light compact\">"));
    }

    /// 6662 on the embed: a documented section's doc is a line on the page (the player swaps it for the playing
    /// section's from the block's `section` list); a program with no section docs has no such line.
    #[test]
    fn a_documented_sections_doc_is_a_line_on_the_page() {
        let written = "tempo 128\nsection intro 2 bars {\n  deck A: kick x ~ ~ ~\n}\n# drop: the kick and <the> bass\nsection drop 2 bars {\n  deck A: kick x ~ x ~\n}\narrange intro drop\n";
        let docs: Vec<Value> = djlive::doc::doc_comments(written).into_iter().map(|d| json!({"kind": d.kind, "name": d.name, "doc": d.doc})).collect();
        let p = prog(written, json!({"license": "CC0-1.0", "visibility": "public", "docs": docs}));
        let html = page("https://beatscript.app", &p, Look::default());
        assert!(html.contains("<p class=\"bsp-doc\" id=\"bsp-doc\">the kick and &lt;the&gt; bass</p>"), "{html}");
        let v = djlive::progld::read_view(&html, true).unwrap();
        assert_eq!(v.cid, p.cid, "the CID is the text's, comments or not");
        assert!(html.contains(r#""section":[{"name":"drop","description":"the kick and "#), "the block carries it for the player: {html}");
        assert!(!page("https://beatscript.app", &prog(TEXT, json!({})), Look::default()).contains("bsp-doc"), "no docs, no line");
    }

    #[test]
    fn oembed_answers_rich_with_the_frame_sized_within_the_consumers_limits() {
        let p = prog(TEXT, json!({"license": "CC-BY-4.0", "visibility": "public", "name": "night bus", "credits": ["bo"]}));
        let o = oembed_json("https://beatscript.app", &p, (None, None));
        assert_eq!((o["version"].as_str(), o["type"].as_str(), o["provider_name"].as_str()), (Some("1.0"), Some("rich"), Some("BeatScript")));
        assert_eq!((o["title"].as_str(), o["author_name"].as_str()), (Some("night bus"), Some("bo")));
        assert_eq!((o["width"].as_u64(), o["height"].as_u64()), (Some(480), Some(152)));
        let html = o["html"].as_str().unwrap();
        assert!(html.starts_with(&format!("<iframe src=\"https://beatscript.app/embed/{}\" width=\"480\" height=\"152\"", p.cid)), "{html}");
        assert!(html.contains(&format!("sandbox=\"{SANDBOX}\"")) && !html.contains("style="), "{html}");
        assert_eq!(o["license_url"], "https://creativecommons.org/licenses/by/4.0/");
        let small = oembed_json("https://beatscript.app", &p, (Some(300), Some(100)));
        assert_eq!((small["width"].as_u64(), small["height"].as_u64()), (Some(300), Some(80)));
        assert!(small["html"].as_str().unwrap().contains("?compact=1"));
    }

    #[test]
    fn oembed_reads_only_this_sites_program_pages() {
        let o = "https://beatscript.app";
        let c = "z4EBG9j9Y9qUpRxjvhRAxx9vWtN8X6xNp6Ci1wAXsQabD7BVmNM";
        assert_eq!(linked(o, &format!("{o}/p/{c}")), Some((c.into(), String::new())));
        assert_eq!(linked(o, &format!("{o}/c/{c}/")), Some((c.into(), String::new())));
        assert_eq!(linked(o, &format!("{o}/c/{c}.eJwrSS0u?x=1#y")), Some((c.into(), "eJwrSS0u".into())));
        for bad in [format!("https://evil.example/p/{c}"), format!("{o}/p/{c}.eJw"), format!("{o}/docs/"), format!("{o}/p/zabc"), format!("{o}/embed/{c}"), format!("{o}/c/{c}.a.b"), String::new()] {
            assert_eq!(linked(o, &bad), None, "{bad}");
        }
        assert_eq!(enc("https://beatscript.app/p/z1"), "https%3A%2F%2Fbeatscript.app%2Fp%2Fz1");
        let p = prog(TEXT, json!({}));
        let d = discovery(o, &p, &format!("{o}/p/{}", p.cid));
        assert_eq!(d, format!("<link rel=\"alternate\" type=\"application/json+oembed\" href=\"https://beatscript.app/oembed?url=https%3A%2F%2Fbeatscript.app%2Fp%2F{}&amp;format=json\" title=\"{}\">\n", p.cid, esc(&p.label())));
    }

    #[test]
    fn a_player_card_names_the_frame_and_the_render() {
        let p = prog(TEXT, json!({}));
        let pl = Player::of("https://beatscript.app", &p);
        let m = player_meta(&pl);
        assert!(m.contains(&format!("<meta name=\"twitter:player\" content=\"https://beatscript.app/embed/{}\">", p.cid)), "{m}");
        assert!(m.contains("<meta name=\"twitter:player:width\" content=\"480\">\n<meta name=\"twitter:player:height\" content=\"152\">"));
        assert!(m.contains(&format!("<meta property=\"og:audio\" content=\"https://beatscript.app/api/render/{}?format=opus\">", p.cid)), "{m}");
        assert!(m.contains("<meta property=\"og:audio:type\" content=\"audio/ogg\">"));
    }

    #[test]
    fn the_frame_policy_differs_from_the_sites_only_in_who_may_frame() {
        assert!(FRAME_CSP.ends_with("frame-ancestors *"), "{}", *FRAME_CSP);
        assert_eq!(FRAME_CSP.replace("frame-ancestors *", "frame-ancestors 'none'"), crate::CSP);
    }
}
