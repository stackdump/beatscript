//! The share card as an SVG, `GET|HEAD /card/<cid>.svg` (and `/card/default.svg`): the no-JavaScript embed. A Markdown
//! or GitHub README shows it as an image that links to the program, beside a plain audio link; nothing in it runs.
//!
//! It is drawn from the data the PNG card is drawn from (`Content`, built as `render_png` builds it): the name, the
//! tempo and key, the sections as proportional bars, the licence badge and the short CID, with the wordmark, and a play
//! triangle where the PNG has code. Only an embeddable program (`embed::embeddable`, the storage rule) gets its own
//! card; any other CID gets the generic one, and `x-btw-card` says why, as the PNG's does.
//!
//! The image has no script, no foreignObject, no event attribute and no link out: its fonts are the reader's system
//! fonts, every reference is to its own `#id`, and every string from a program is XML-escaped and capped as the PNG caps
//! it. Its CSP lets it load nothing at all.

use super::*;

/// Part of the ETag: a new layout is a new tag.
const SVG_VERSION: &str = "1";
const SVG_CSP: &str = "default-src 'none'; style-src 'unsafe-inline'";

const SANS: &str = "system-ui, -apple-system, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif";
const MONO: &str = "ui-monospace, SFMono-Regular, Menlo, Consolas, 'Liberation Mono', monospace";

// the PNG's colours (draw.rs)
const TEXT: &str = "#e6ecf5";
const SUB: &str = "#8d9bb5";
const DIM: &str = "#66768f";
const ACCENT: &str = "#5eead4";
const AMBER: &str = "#ffb020";
const BLOCK: &str = "#16213a";
const BLOCK_T: &str = "#93a3bf";

/// The ground and the stage, the same on every card: the PNG's gradient and its two blooms.
const GROUND: &str = concat!(
    "<defs>\n",
    "<linearGradient id=\"bsc-bg\" x1=\"0\" y1=\"0\" x2=\"0\" y2=\"1\"><stop offset=\"0\" stop-color=\"#070b12\"/><stop offset=\"1\" stop-color=\"#0b111c\"/></linearGradient>\n",
    "<radialGradient id=\"bsc-glow-a\" gradientUnits=\"userSpaceOnUse\" cx=\"1160\" cy=\"0\" r=\"660\"><stop offset=\"0\" stop-color=\"#5eead4\" stop-opacity=\"0.07\"/><stop offset=\"1\" stop-color=\"#5eead4\" stop-opacity=\"0\"/></radialGradient>\n",
    "<radialGradient id=\"bsc-glow-b\" gradientUnits=\"userSpaceOnUse\" cx=\"0\" cy=\"630\" r=\"600\"><stop offset=\"0\" stop-color=\"#7c5cff\" stop-opacity=\"0.06\"/><stop offset=\"1\" stop-color=\"#7c5cff\" stop-opacity=\"0\"/></radialGradient>\n",
    "<clipPath id=\"bsc-words\"><rect x=\"330\" y=\"92\" width=\"806\" height=\"350\"/></clipPath>\n",
    "</defs>\n",
    "<rect width=\"1200\" height=\"630\" fill=\"url(#bsc-bg)\"/>\n",
    "<rect width=\"1200\" height=\"630\" fill=\"url(#bsc-glow-a)\"/>\n",
    "<rect width=\"1200\" height=\"630\" fill=\"url(#bsc-glow-b)\"/>\n",
    "<rect x=\"44\" y=\"92\" width=\"1112\" height=\"350\" rx=\"16\" fill=\"#0e1522\"/>\n",
    "<rect x=\"44.75\" y=\"92.75\" width=\"1110.5\" height=\"348.5\" rx=\"16\" fill=\"none\" stroke=\"#1d2a42\" stroke-width=\"1.5\"/>\n",
    // the play button
    "<circle cx=\"194\" cy=\"267\" r=\"104\" fill=\"none\" stroke=\"#5eead4\" stroke-opacity=\"0.25\" stroke-width=\"2\"/>\n",
    "<circle cx=\"194\" cy=\"267\" r=\"86\" fill=\"#5eead4\"/>\n",
    "<path d=\"M172 219V315L254 267Z\" fill=\"#070b12\"/>\n",
    // the wordmark: the PNG's Petri mark (place, transition, place) and the name
    "<g fill=\"none\" stroke=\"#5eead4\" stroke-width=\"2.2\"><circle cx=\"51\" cy=\"58\" r=\"6.5\"/><path d=\"M57.5 58H66M71 58H79.5\"/><circle cx=\"86\" cy=\"58\" r=\"6.5\"/></g>\n",
    "<rect x=\"66\" y=\"49\" width=\"5\" height=\"18\" rx=\"1.5\" fill=\"#5eead4\"/><circle cx=\"51\" cy=\"58\" r=\"2.4\" fill=\"#5eead4\"/>\n",
);

/// Text or an attribute value, XML-escaped; a character XML 1.0 does not allow is dropped.
fn x(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            '\t' | '\n' | '\r' => o.push(' '),
            c if (c as u32) < 0x20 || c == '\u{FFFE}' || c == '\u{FFFF}' => {}
            c => o.push(c),
        }
    }
    o
}

/// A coordinate, to a tenth.
fn n(v: f64) -> String {
    let s = format!("{v:.1}");
    s.strip_suffix(".0").map_or_else(|| s.clone(), str::to_string)
}

/// A character's advance in ems. Nothing is measured here (the reader's own font draws the text), so this errs wide.
fn ems(c: char, bold: bool) -> f64 {
    if (c as u32) >= 0x2E80 {
        1.0
    } else if c.is_ascii_uppercase() || c == 'm' || c == 'w' {
        0.78
    } else if bold {
        0.62
    } else {
        0.57
    }
}

fn width(s: &str, size: f64, bold: bool) -> f64 {
    s.chars().map(|c| ems(c, bold)).sum::<f64>() * size
}

/// `s`, cut with an ellipsis to fit `w` pixels at `size`.
fn fit(s: &str, size: f64, w: f64, bold: bool) -> String {
    if width(s, size, bold) <= w {
        return s.to_string();
    }
    let room = w - width("…", size, bold);
    let mut used = 0.0;
    let head: String = s
        .chars()
        .take_while(|c| {
            used += ems(*c, bold) * size;
            used <= room
        })
        .collect();
    let head = head.trim_end();
    if head.is_empty() {
        String::new()
    } else {
        format!("{head}…")
    }
}

/// The PNG's `Content` of a program (as `render_png` builds it, without the code it draws).
fn content_of(site: &str, p: &Prog, rep: &Report) -> Content {
    let seconds = match (rep.seconds, rep.tempo) {
        (Some(s), _) => s,
        (None, Some(t)) if t > 0.0 && rep.song_bars > 0 => rep.song_bars as f64 * 4.0 * 60.0 / t,
        _ => 0.0,
    };
    Content {
        cid: p.cid.clone(),
        name: p.label(),
        license: p.meta.license.clone(),
        tempo: rep.tempo.unwrap_or(0.0),
        key: rep.key.clone(),
        bars: rep.song_bars,
        seconds,
        sections: rep.sections.clone(),
        site: site.into(),
        ..Content::default()
    }
}

/// The facts row of the PNG's wide card: (value, unit).
fn facts(c: &Content) -> Vec<(String, String)> {
    if c.generic {
        return vec![("a track in a few lines".into(), String::new()), ("same text, same CID".into(), String::new())];
    }
    let mut fs = Vec::new();
    if c.tempo > 0.0 {
        fs.push((lex::fmt_bpm(c.tempo), "BPM".into()));
    }
    let k = lex::key_label(&lex::clean_text(&c.key, 24));
    if !k.is_empty() {
        fs.push((k, String::new()));
    }
    if c.bars > 0 {
        fs.push((c.bars.to_string(), "bars".into()));
        if c.seconds > 0.0 {
            fs.push((lex::clock(c.seconds), String::new()));
        }
    } else {
        fs.push(("loop".into(), String::new()));
    }
    fs
}

/// The timeline's sections, as the PNG's: those with bars, at most 40, and one bar of "loop" when there are none.
fn sections(c: &Content) -> Vec<Section> {
    let mut s: Vec<Section> = c.sections.iter().filter(|s| s.bars > 0).take(40).cloned().collect();
    if s.is_empty() {
        s.push(Section { name: "loop".into(), start: 0, bars: 1 });
    }
    s
}

/// The card of `c` as SVG text, with `line` (the program's description) under the name.
fn draw(c: &Content, line: &str) -> String {
    let fs = facts(c);
    let secs = sections(c);
    let site = if c.site.is_empty() { "btw" } else { c.site.as_str() };
    let said = fs.iter().map(|(v, u)| if u.is_empty() { v.clone() } else { format!("{v} {u}") }).collect::<Vec<_>>().join(" · ");
    let names = secs.iter().map(|s| lex::clean_text(&s.name, 24)).collect::<Vec<_>>().join(", ");
    let mut s = String::with_capacity(8192);
    s.push_str("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1200\" height=\"630\" viewBox=\"0 0 1200 630\" role=\"img\">\n");
    s.push_str(&format!("<title>{} · BeatScript</title>\n<desc>{}. Sections: {}.</desc>\n", x(&c.name), x(&said), x(&names)));
    s.push_str(GROUND);
    s.push_str(&format!("<g font-family=\"{SANS}\">\n"));
    s.push_str(&format!("<text x=\"104\" y=\"66\" font-size=\"24\" font-weight=\"700\" fill=\"{TEXT}\">BeatScript</text>\n"));
    s.push_str(&format!("<text x=\"1156\" y=\"66\" font-size=\"20\" fill=\"{SUB}\" text-anchor=\"end\">{}</text>\n", x(&lex::clean_text(site, 40))));

    // the name, the line under it and what a click does, beside the play button (clipped to the stage)
    const WORDS: f64 = 794.0;
    let (size, name) = [54.0, 44.0, 36.0].into_iter().find(|z| width(&c.name, *z, true) <= WORDS).map_or_else(|| (36.0, fit(&c.name, 36.0, WORDS, true)), |z| (z, c.name.clone()));
    s.push_str("<g clip-path=\"url(#bsc-words)\">\n");
    s.push_str(&format!("<text x=\"338\" y=\"232\" font-size=\"{}\" font-weight=\"700\" fill=\"{TEXT}\">{}</text>\n", n(size), x(&name)));
    let line = fit(&lex::clean_text(line, 200), 24.0, WORDS, false);
    if !line.is_empty() {
        s.push_str(&format!("<text x=\"338\" y=\"284\" font-size=\"24\" fill=\"{SUB}\">{}</text>\n", x(&line)));
    }
    let cta = if c.generic { "Write one in your browser" } else { "Play it in your browser" };
    s.push_str(&format!("<text x=\"338\" y=\"340\" font-size=\"22\" font-weight=\"700\" fill=\"{ACCENT}\">{cta}</text>\n</g>\n"));

    // the section strip: a bar per section, its width its share of the song's bars, named where it fits
    const GAP: f64 = 4.0;
    let total: i64 = secs.iter().map(|s| s.bars).sum();
    let avail = 1112.0 - GAP * (secs.len() as f64 - 1.0);
    let mut at = 44.0;
    for sec in &secs {
        let bw = avail * sec.bars as f64 / total as f64;
        s.push_str(&format!("<rect x=\"{}\" y=\"466\" width=\"{}\" height=\"46\" rx=\"7\" fill=\"{BLOCK}\"/>\n", n(at), n(bw)));
        let label = if bw > 26.0 { fit(&lex::clean_text(&sec.name, 24), 16.0, bw - 14.0, false) } else { String::new() };
        if !label.is_empty() {
            s.push_str(&format!("<text x=\"{}\" y=\"494.6\" font-size=\"16\" fill=\"{BLOCK_T}\" text-anchor=\"middle\">{}</text>\n", n(at + bw / 2.0), x(&label)));
        }
        at += bw + GAP;
    }
    // the playhead, at the top of the song
    s.push_str(&format!("<rect x=\"48.75\" y=\"461\" width=\"2.5\" height=\"56\" rx=\"1.2\" fill=\"{AMBER}\"/>\n"));

    // the footer, right to left: the short CID (the site on the generic card), the licence badge, then the facts
    let id = if c.generic { lex::clean_text(site, 40) } else { lex::short_cid(&c.cid) };
    s.push_str(&format!("<text x=\"1156\" y=\"586\" font-family=\"{MONO}\" font-size=\"19\" fill=\"{SUB}\" text-anchor=\"end\">{}</text>\n", x(&id)));
    let right = 1156.0 - id.chars().count() as f64 * 19.0 * 0.62 - 22.0;
    let badge = lex::license_badge(&c.license);
    let (bw, bh) = (width(&badge, 16.0, true) + 26.0, 30.4);
    s.push_str(&format!(
        "<rect x=\"{}\" y=\"565.2\" width=\"{}\" height=\"{}\" rx=\"{}\" fill=\"{ACCENT}\" fill-opacity=\"0.12\" stroke=\"{ACCENT}\" stroke-opacity=\"0.8\" stroke-width=\"1.5\"/>\n",
        n(right - bw),
        n(bw),
        n(bh),
        n(bh / 2.0)
    ));
    s.push_str(&format!("<text x=\"{}\" y=\"586\" font-size=\"16\" font-weight=\"700\" fill=\"{ACCENT}\" text-anchor=\"middle\">{}</text>\n", n(right - bw / 2.0), x(&badge)));
    let room = right - bw - 22.0 - 44.0;
    let mut used = 0.0;
    s.push_str(&format!("<text x=\"44\" y=\"586\" font-size=\"30\" font-weight=\"700\" fill=\"{TEXT}\">"));
    for (i, (v, u)) in fs.iter().enumerate() {
        let w = if i > 0 { 28.0 + width("·", 30.0, false) } else { 0.0 } + width(v, 30.0, true) + if u.is_empty() { 0.0 } else { 8.0 + width(u, 17.0, false) };
        if used + w > room {
            break;
        }
        used += w;
        if i > 0 {
            s.push_str(&format!("<tspan dx=\"14\" font-weight=\"400\" fill=\"{DIM}\">·</tspan><tspan dx=\"14\">{}</tspan>", x(v)));
        } else {
            s.push_str(&format!("<tspan>{}</tspan>", x(v)));
        }
        if !u.is_empty() {
            s.push_str(&format!("<tspan dx=\"8\" font-size=\"17\" font-weight=\"400\" fill=\"{SUB}\">{}</tspan>", x(u)));
        }
    }
    s.push_str("</text>\n</g>\n</svg>\n");
    s
}

/// The headers every SVG card has.
fn headers(cache: &'static str, etag: &str) -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/svg+xml"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    h.insert(header::ETAG, HeaderValue::from_str(etag).unwrap());
    h.insert("content-security-policy", HeaderValue::from_static(SVG_CSP));
    h
}

/// The generic card, saying why it is the generic one.
fn generic(c: &Cards, req: &content::Req, reason: &str, cache: &'static str, noidx: bool) -> Response<Body> {
    let body = draw(&generic_content(&c.site), GENERIC_DESC);
    let mut h = headers(cache, &format!("\"default-svg-v{SVG_VERSION}\""));
    h.insert("x-btw-card", HeaderValue::from_str(&format!("generic; reason={reason}")).unwrap());
    if noidx {
        h.insert("x-robots-tag", HeaderValue::from_static("noindex"));
    }
    content::serve(h, req, body.len() as u64, Some(Bytes::from(body)))
}

/// `GET|HEAD /card/<name>.svg` from `addr`: `name` is `default`, `<cid>` or `<cid>.<packed>`.
pub(super) async fn serve(c: &Cards, creq: &content::Req<'_>, addr: &str, name: &str) -> Response<Body> {
    if name == "default" {
        return generic(c, creq, "default", "public, max-age=86400", false);
    }
    let (cid, packed) = name.split_once('.').unwrap_or((name, ""));
    let p = match c.resolve(Some(addr), cid, packed).await {
        Ok(p) => p,
        Err(e) => return unavailable(&e),
    };
    let Some(rep) = p.report.clone().filter(|_| embed::embeddable(&p)) else {
        let st = p.state();
        let reason = if st != St::Ok {
            state_name(st)
        } else if !p.known {
            "unstored"
        } else {
            "license"
        };
        return generic(c, creq, reason, CACHE_SHORT, no_index(st));
    };
    let desc = if p.desc.is_empty() { lex::clean_text(&p.ex_desc, 200) } else { p.desc.clone() };
    let body = draw(&content_of(&c.site, &p, &rep), &desc);
    let h = name_hash(&p) + &license_hash(&p);
    let etag = if h.is_empty() { format!("\"{}-svg-v{SVG_VERSION}\"", p.cid) } else { format!("\"{}-svg-v{SVG_VERSION}-{h}\"", p.cid) };
    let mut h = headers(CACHE_IMMUTABLE, &etag);
    h.insert("x-btw-card", HeaderValue::from_static("program"));
    content::serve(h, creq, body.len() as u64, Some(Bytes::from(body)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A start tag: its name and its attributes.
    type Tag = (String, Vec<(String, String)>);

    fn entity_ok(s: &str) -> bool {
        let Some(end) = s.find(';') else { return false };
        let e = &s[1..end];
        matches!(e, "amp" | "lt" | "gt" | "quot" | "apos")
            || e.strip_prefix("#x").is_some_and(|h| !h.is_empty() && h.chars().all(|c| c.is_ascii_hexdigit()))
            || e.strip_prefix('#').is_some_and(|d| !d.is_empty() && d.chars().all(|c| c.is_ascii_digit()))
    }

    fn text_ok(t: &str) -> Result<(), String> {
        match t.match_indices('&').find(|(i, _)| !entity_ok(&t[*i..])) {
            Some(_) => Err(format!("a bare & in {t:?}")),
            None => Ok(()),
        }
    }

    /// A small XML checker: one root, every tag closed in order, attributes double-quoted and unique, no `<` in a value,
    /// and `&` only as one of the five entities or a numeric one. Enough to say an XML parser reads it. Gives the start
    /// tags.
    fn well_formed(s: &str) -> Result<Vec<Tag>, String> {
        let (mut open, mut tags, mut roots) = (Vec::<String>::new(), Vec::new(), 0);
        let mut rest = s;
        while let Some(i) = rest.find('<') {
            let text = &rest[..i];
            if open.is_empty() && !text.trim().is_empty() {
                return Err(format!("text outside the root: {text:?}"));
            }
            text_ok(text)?;
            let t = &rest[i..];
            if let Some(c) = t.strip_prefix("<!--") {
                rest = &c[c.find("-->").ok_or("an unclosed comment")? + 3..];
                continue;
            }
            // the tag runs to the first `>` outside a quoted value
            let mut quoted = false;
            let end = t
                .char_indices()
                .find(|&(_, ch)| {
                    if ch == '"' {
                        quoted = !quoted;
                    }
                    !quoted && ch == '>'
                })
                .ok_or("an unclosed tag")?
                .0;
            let inner = &t[1..end];
            rest = &t[end + 1..];
            if let Some(name) = inner.strip_prefix('/') {
                let want = open.pop().ok_or(format!("</{name}> closes nothing"))?;
                if name.trim() != want {
                    return Err(format!("</{name}> closes <{want}>"));
                }
                continue;
            }
            let (body, empty) = inner.strip_suffix('/').map_or((inner, false), |b| (b, true));
            let name_end = body.find(char::is_whitespace).unwrap_or(body.len());
            let name = &body[..name_end];
            let is_name = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_'));
            if !is_name(name) {
                return Err(format!("a bad tag name in <{inner}>"));
            }
            let mut attrs: Vec<(String, String)> = Vec::new();
            let mut a = body[name_end..].trim_start();
            while !a.is_empty() {
                let eq = a.find('=').ok_or(format!("an attribute with no value in <{inner}>"))?;
                let k = &a[..eq];
                let v = a[eq + 1..].strip_prefix('"').ok_or(format!("an unquoted value in <{inner}>"))?;
                let close = v.find('"').ok_or("an unclosed value")?;
                let val = &v[..close];
                if !is_name(k) || val.contains('<') || attrs.iter().any(|(n, _)| n == k) {
                    return Err(format!("a bad attribute {k:?} in <{inner}>"));
                }
                text_ok(val)?;
                attrs.push((k.to_string(), val.to_string()));
                let after = &v[close + 1..];
                if !after.is_empty() && !after.starts_with(char::is_whitespace) {
                    return Err(format!("no space after {k} in <{inner}>"));
                }
                a = after.trim_start();
            }
            if open.is_empty() {
                roots += 1;
            }
            tags.push((name.to_string(), attrs));
            if !empty {
                open.push(name.to_string());
            }
        }
        if !open.is_empty() || !rest.trim().is_empty() || roots != 1 {
            return Err(format!("unclosed {open:?}, {roots} roots, trailing {rest:?}"));
        }
        Ok(tags)
    }

    /// Well formed, and nothing that runs or reaches out: no script or foreignObject, no on*= attribute, no href, and
    /// every url() a reference to the card's own #id.
    fn assert_inert(svg: &str) {
        let tags = well_formed(svg).unwrap_or_else(|e| panic!("{e}\n{svg}"));
        assert_eq!(tags[0].0, "svg");
        for (name, attrs) in &tags {
            let tag = name.to_ascii_lowercase();
            assert!(!["script", "foreignobject", "a", "image", "use", "style", "iframe"].contains(&tag.as_str()), "<{name}>");
            for (k, v) in attrs {
                let k = k.to_ascii_lowercase();
                assert!(!k.starts_with("on") && !k.ends_with("href"), "{k}= on <{name}>");
                assert!(v.match_indices("url(").all(|(i, _)| v[i + 4..].starts_with('#')), "an outside url on <{name}>: {v}");
                assert!(!v.to_ascii_lowercase().contains("javascript:"), "{v}");
            }
        }
        let low = svg.to_ascii_lowercase();
        assert!(!low.contains("<script") && !low.contains("foreignobject") && !low.contains("@import") && !low.contains("@font-face"), "{svg}");
    }

    fn prog(text: &str, meta: Value) -> Prog {
        let rep: Value = serde_json::from_str(&btwapi::check_json(text)).unwrap();
        let r = Report::from_json(&rep);
        Prog { canonical: r.canonical.clone(), report: Some(Arc::new(r.clone())), meta: Meta::from_json(&meta), known: true, ..Prog::bare(&r.cid, St::Ok) }
    }

    const TEXT: &str = "tempo 128\nkey am\nsection intro 4 bars {\n  deck A: kick x ~ ~ ~\n}\nsection drop 8 bars {\n  deck A: kick x ~ x ~\n}\narrange intro drop\n";
    const HOSTILE: &str = "<script>alert(\"x\")</script> it's & <b onload=\"y\">";

    #[test]
    fn the_card_shows_the_name_facts_sections_badge_and_cid() {
        let p = prog(TEXT, json!({"name": "night bus", "license": "CC-BY-4.0", "visibility": "public"}));
        let s = draw(&content_of("beatscript.app", &p, p.report.as_ref().unwrap()), "a test track");
        assert_inert(&s);
        assert!(s.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1200\" height=\"630\" viewBox=\"0 0 1200 630\""), "{s}");
        let cid = format!(">{}</text>", x(&lex::short_cid(&p.cid)));
        for want in [">night bus</text>", ">a test track</text>", "<tspan>128</tspan>", ">BPM</tspan>", ">A minor</tspan>", ">12</tspan>", ">intro</text>", ">drop</text>", ">CC BY 4.0</text>", &cid, ">BeatScript</text>", ">beatscript.app</text>", "<path d=\"M172 219V315L254 267Z\""] {
            assert!(s.contains(want), "no {want}:\n{s}");
        }
        // the bars are proportional: intro is 4 of the 12 bars, drop 8 (1112 px less one 4 px gap)
        assert!(s.contains("<rect x=\"44\" y=\"466\" width=\"369.3\"") && s.contains("<rect x=\"417.3\" y=\"466\" width=\"738.7\""), "{s}");
        // no font is fetched: the stacks end in the generic families
        assert!(s.contains(&format!("font-family=\"{SANS}\"")) && SANS.ends_with("sans-serif") && MONO.ends_with("monospace"));
    }

    #[test]
    fn a_hostile_name_and_section_are_escaped_and_the_card_runs_nothing() {
        let p = prog(TEXT, json!({"name": HOSTILE}));
        let mut rep = (**p.report.as_ref().unwrap()).clone();
        rep.sections[0].name = "</text><script>x</script>".into();
        rep.key = "\"am\" onload=\"x\"".into();
        let s = draw(&content_of("beatscript.app", &p, &rep), "<desc> & 'more'");
        assert_inert(&s);
        assert!(s.contains("<title>&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt; it&apos;s &amp; &lt;b onload=&quot;y&quot;&gt; · BeatScript</title>"), "{s}");
        assert!(s.contains("&quot;am&quot; onload=&quot;x&quot;") && s.contains("&lt;/text&gt;"), "{s}");
        // capped as the PNG caps it: the name at 60 runes, and cut to fit where it is drawn
        let long = prog(TEXT, json!({"name": "W".repeat(90)}));
        let s = draw(&content_of("beatscript.app", &long, long.report.as_ref().unwrap()), "");
        assert_inert(&s);
        assert!(s.contains(&format!("<title>{}… · BeatScript</title>", "W".repeat(59))), "{s}");
        assert!(s.contains("W…</text>") && !s.contains(&format!(">{}…</text>", "W".repeat(59))), "{s}");
        assert_eq!(x("a\u{1}b\u{FFFF}c\td"), "abc d", "no character XML refuses");
    }

    #[test]
    fn the_generic_card_is_inert_too() {
        let s = draw(&generic_content("beatscript.app"), GENERIC_DESC);
        assert_inert(&s);
        for want in [">write a whole track as text</text>", ">Write one in your browser</text>", ">intro</text>", ">outro</text>", ">CC0</text>", "<tspan>a track in a few lines</tspan>"] {
            assert!(s.contains(want), "no {want}:\n{s}");
        }
        assert_eq!(fit("abcdefghij", 10.0, 40.0, false), "abcdef…");
        assert_eq!(fit("abc", 10.0, 40.0, false), "abc");
    }

    const EXAMPLE: &str = "#! name: <script>alert(\"x\")</script> & 'night bus'\n# a test example\ntempo 128\nkey am\nsection intro 4 bars {\n  deck A: kick x ~ ~ ~\n}\nsection drop 8 bars {\n  deck A: kick x ~ x ~\n}\narrange intro drop\n";

    async fn send(r: &Router, method: &str, path: &str, if_none_match: Option<&str>) -> (u16, HeaderMap, String) {
        let mut b = axum::http::Request::builder().method(method).uri(path);
        if let Some(e) = if_none_match {
            b = b.header(header::IF_NONE_MATCH, e);
        }
        let res = tower::ServiceExt::oneshot(r.clone(), b.body(Body::empty()).unwrap()).await.unwrap();
        let (parts, body) = res.into_parts();
        let body = axum::body::to_bytes(body, 1 << 20).await.unwrap();
        (parts.status.as_u16(), parts.headers, String::from_utf8(body.to_vec()).unwrap())
    }

    /// A program stored as the share route stores it: canonical text and meta, under its CID.
    fn store(dir: &std::path::Path, text: &str, meta: Value) -> String {
        let rep = crate::store::check(text);
        assert_eq!(rep["ok"], true, "{rep}");
        let (cid, canonical) = (rep["cid"].as_str().unwrap().to_string(), rep["canonical"].as_str().unwrap().to_string());
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(format!("{cid}.json")), serde_json::to_vec(&json!({"cid": cid, "text": canonical, "meta": meta})).unwrap()).unwrap();
        cid
    }

    #[tokio::test]
    async fn the_route_answers_an_svg_for_a_stored_example_and_the_generic_card_for_the_rest() {
        let tmp = tempfile::tempdir().unwrap();
        let mut app = crate::test_app(&tmp.path().join("data"));
        let ex = tmp.path().join("examples");
        std::fs::create_dir_all(&ex).unwrap();
        std::fs::write(ex.join("index.json"), r#"[{"name": "night-bus", "description": "a test example"}]"#).unwrap();
        std::fs::write(ex.join("night-bus.btw"), EXAMPLE).unwrap();
        let (examples, _) = crate::site::load_examples(Some(&ex)).unwrap();
        let cid = examples.list[0].cid.clone();
        Arc::get_mut(&mut app).expect("the test's own app").examples = examples;
        let programs = app.cfg.programs.clone();
        let private = store(&programs, "tempo 90\nkey cm\nsection groove 2 bars {\n  deck A: kick x ~ ~ ~\n}\narrange groove\n", json!({"license": "CC0-1.0", "visibility": "private"}));
        let reserved = store(&programs, "tempo 91\nkey cm\nsection groove 2 bars {\n  deck A: kick x ~ ~ ~\n}\narrange groove\n", json!({"license": "all-rights-reserved", "visibility": "public"}));
        let unstored = "tempo 92\nkey dm\nsection groove 2 bars {\n  deck A: kick x x ~ ~\n}\narrange groove\n";
        let unstored_rep = crate::store::check(unstored);
        let unstored_link = format!("{}.{}", unstored_rep["cid"].as_str().unwrap(), pack(unstored_rep["canonical"].as_str().unwrap()));
        let r = routes(&app);

        let (st, h, body) = send(&r, "GET", &format!("/card/{cid}.svg"), None).await;
        assert_eq!(st, 200, "{body}");
        assert_eq!(h["content-type"], "image/svg+xml");
        assert_eq!(h["cache-control"], CACHE_IMMUTABLE);
        assert_eq!(h["content-security-policy"], "default-src 'none'; style-src 'unsafe-inline'");
        assert_eq!(h["x-btw-card"], "program");
        let etag = h["etag"].to_str().unwrap().to_string();
        assert!(etag.starts_with(&format!("\"{cid}-svg-v1-")), "the name is in the tag: {etag}");
        assert_inert(&body);
        assert!(body.contains("<title>&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt; &amp; &apos;night bus&apos; · BeatScript</title>"), "{body}");
        assert!(body.contains(">a test example</text>") && body.contains(">drop</text>") && body.contains(">CC0</text>"), "{body}");
        // HEAD: the headers and no body; a revalidation: 304
        let (st, hh, b) = send(&r, "HEAD", &format!("/card/{cid}.svg"), None).await;
        assert_eq!((st, b.as_str(), hh["content-length"].to_str().unwrap()), (200, "", body.len().to_string().as_str()));
        assert_eq!(send(&r, "GET", &format!("/card/{cid}.svg"), Some(&etag)).await.0, 304);

        // anything that is not an embeddable stored program: the generic card, and why
        let unknown = "z4EBG9jCtKY4ioQ3TEJCQQmTTJZjugJRihCNbBxNJrBaWr1adMi";
        for (path, reason) in [
            (format!("/card/{unknown}.svg"), "unknown"),
            (format!("/card/{private}.svg"), "private"),
            (format!("/card/{reserved}.svg"), "license"),
            (format!("/card/{unstored_link}.svg"), "unstored"),
            ("/card/zabc.svg".to_string(), "invalid"),
            ("/card/default.svg".to_string(), "default"),
        ] {
            let (st, h, body) = send(&r, "GET", &path, None).await;
            assert_eq!(st, 200, "{path}: {body}");
            assert_eq!(h["content-type"], "image/svg+xml", "{path}");
            assert_eq!(h["x-btw-card"], format!("generic; reason={reason}").as_str(), "{path}");
            assert_eq!(h["etag"], "\"default-svg-v1\"", "{path}");
            assert_eq!(h["cache-control"], if reason == "default" { "public, max-age=86400" } else { CACHE_SHORT }, "{path}");
            assert_eq!(h["content-security-policy"], SVG_CSP, "{path}");
            assert_eq!(h.contains_key("x-robots-tag"), reason == "private" || reason == "invalid", "{path}");
            assert_inert(&body);
            assert!(body.contains(">write a whole track as text</text>"), "{path}: {body}");
        }
    }
}
