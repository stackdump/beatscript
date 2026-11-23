//! A program's references in the editor (roadmap 6668): hover says what a reference is, what it resolves to and its
//! expansion; `beatscript/expand` answers the references with their ranges; a code lens over each says `show expansion` and `eject`;
//! the eject code action is offered on a reference and resolves to exactly what `djlive::refs::eject` writes, proved (the same
//! nets); `workspace/executeCommand` runs the lenses' commands. (Completion in a `gen@2` call: gen_complete.rs.)

mod common;

use beatscriptls::{Analysis, Encoding, LineIndex, Server};
use common::*;
use djengine::json::JsonValue;
use djlive::refs;

fn env() -> djlive::env::Env<'static> {
    beatscript_api::web_env().env()
}

fn sites(text: &str) -> Vec<refs::Site> {
    refs::sites(text, &env()).unwrap_or_else(|e| panic!("{text}\n{:?}", e.iter().map(|d| &d.message).collect::<Vec<_>>()))
}

/// A `let` of one part of a generated section.
const LET: &str = "tempo 128\nkey c\nlet bed = gen@2 techno seed=42 section=drop part=bass\nsection groove 4 bars {\n  deck A: kick x x x x | bass bed\n}\narrange groove groove\n";
/// A part line's generator call, and a seed.
const PARTS: &str = "tempo 128\nkey c\nsection groove 4 bars {\n  deck A: kick x x x x\n        | bass gen@2 techno seed=42 section=drop inst=reese\n  deck B: arp seed(trance, 42)[17..20]\n}\narrange groove\n";
/// Two sections of a generated program.
const SECTIONS: &str = "tempo 126\nkey am\nsection one from gen@2 house seed=42 section=intro\nsection two from gen@2 house seed=42 section=drop\narrange one two\n";
/// The whole generated program.
const WHOLE: &str = "arrange from gen@2 techno seed=42 bpm=132\n";

/// A stored program the host knows, borrowed from by its CID: a part as a let, and a section's part on a part line.
fn stored() -> String {
    beatscript_api::generate::install();
    let source = "tempo 120\nkey am\nsection drop 4 bars {\n  deck A: kick x ~ x ~ | bass a1 ~ ~ ~ c2 ~ ~ ~ e2 ~ ~ ~ a1 ~ ~ ~\n}\narrange drop\n";
    let canon = djlive::print::canonical(&djlive::parse::parse(source).program);
    let cid = djlive::compile::program_cid(&canon);
    beatscript_api::known::know(&cid, &canon).unwrap();
    format!("tempo 120\nkey am\nlet riff = {cid}.drop.bass\nsection grv 4 bars {{\n  deck A: kick x x x x | bass riff\n  deck B: bass {cid}.drop\n}}\narrange grv\n")
}

/// The params of a request at byte `byte` of `text` (UTF-16 positions).
fn at_byte(text: &str, byte: usize) -> String {
    let p = LineIndex::new(text).position(text, byte, Encoding::Utf16);
    at(p.line, p.character)
}

/// A server whose client takes `workspace/applyEdit`.
fn server_applying(text: &str) -> Server {
    let mut s = Server::new();
    let init = JsonValue::parse(r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"capabilities":{"general":{"positionEncodings":["utf-16"]},"workspace":{"applyEdit":true}}}}"#).unwrap();
    s.handle(&init);
    s.open(URI, text, 1);
    s
}

/// A WorkspaceEdit's edits on the test document, made to its text.
fn apply(text: &str, edit: &JsonValue) -> String {
    let ix = LineIndex::new(text);
    let edits = edit.get("changes").and_then(|c| c.get(URI)).and_then(JsonValue::as_array).unwrap();
    let mut spans: Vec<(usize, usize, String)> = edits
        .iter()
        .map(|e| {
            let r = beatscriptls::json::get_range(e.get("range").unwrap()).unwrap();
            let sp = ix.span(text, r, Encoding::Utf16);
            (sp.start, sp.end, e.get("newText").unwrap().as_str().unwrap().to_string())
        })
        .collect();
    spans.sort_by_key(|s| std::cmp::Reverse(s.0));
    let mut t = text.to_string();
    for (a, b, n) in spans {
        t.replace_range(a..b, &n);
    }
    t
}

/// A range as a byte span of `text`.
fn span_of(text: &str, range: &JsonValue, enc: Encoding) -> (usize, usize) {
    let sp = LineIndex::new(text).span(text, beatscriptls::json::get_range(range).unwrap(), enc);
    (sp.start, sp.end)
}

fn hover_text(s: &mut Server, text: &str, at: usize) -> String {
    let h = req(s, "textDocument/hover", &at_byte(text, at));
    h.get("contents").and_then(|c| c.get("value")).and_then(JsonValue::as_str).unwrap_or_else(|| panic!("no hover at {at}: {}", h.js_stringified())).to_string()
}

#[test]
fn hover_on_a_gen_reference_says_its_call_cid_generator_fit_and_expansion() {
    beatscript_api::generate::install();
    let site = &sites(LET)[0];
    let mut s = server(LET);
    let md = hover_text(&mut s, LET, LET.find("gen@2").unwrap() + 3);
    let cid = site.resolved.as_deref().unwrap();
    assert!(md.contains("gen@2 techno seed=42 section=drop part=bass"), "{md}");
    assert!(md.contains("**engine call**") && md.contains(&format!("resolves to `{cid}`")) && md.contains("generator `seeded@2`"), "{md}");
    assert!(md.contains("fitted to the program's key `c` and its tempo 128"), "{md}");
    assert!(md.contains("```beatscript\nlet bed_1 = "), "{md}");
    assert!(md.contains("the `let bed`"), "{md}");
    golden("hover-gen-reference", &req(&mut s, "textDocument/hover", &at_byte(LET, LET.find("gen@2").unwrap())));
    // the let's name is the let's own hover; the rest of its line is the reference's
    let name = hover_text(&mut s, LET, LET.find("bed =").unwrap() + 1);
    assert!(name.contains("let bed = gen@2"), "{name}");
}

#[test]
fn hover_on_a_part_lines_call_and_a_seed() {
    beatscript_api::generate::install();
    let ss = sites(PARTS);
    assert_eq!(ss.iter().map(|s| s.kind).collect::<Vec<_>>(), ["gen", "seed"]);
    let mut s = server(PARTS);
    let gen = hover_text(&mut s, PARTS, PARTS.find("gen@2").unwrap());
    assert!(gen.contains(ss[0].resolved.as_deref().unwrap()) && gen.contains("a `bass` part") && gen.contains("**Expansion**"), "{gen}");
    let seed = hover_text(&mut s, PARTS, PARTS.find("seed(").unwrap() + 1);
    assert!(seed.contains(&format!("resolves to `{}`", ss[1].resolved.as_deref().unwrap())), "{seed}");
    assert!(seed.contains("`seed`: a track of the beats generator") && seed.contains("**engine call**") && seed.contains("track *"), "{seed}");
    assert!(seed.contains(&format!("```beatscript\n{}", refs_expansion(&ss[1]).lines().next().unwrap())), "{seed}");
}

fn refs_expansion(s: &refs::Site) -> String {
    beatscriptls::refs::expansion(s).to_string()
}

#[test]
fn hover_on_a_stored_reference_is_a_structural_import() {
    let text = stored();
    let ss = sites(&text);
    assert_eq!(ss.len(), 2, "{ss:?}");
    let mut s = server(&text);
    let md = hover_text(&mut s, &text, text.find("let riff = ").unwrap() + "let riff = ".len() + 2);
    let cid = ss[0].resolved.as_deref().unwrap();
    assert!(text.contains(&format!("{cid}.drop.bass")));
    assert!(md.contains("**structural import**") && md.contains("`stored`") && md.contains(&format!("resolves to `{cid}`")), "{md}");
    assert!(md.contains("```beatscript\n"), "{md}");
}

#[test]
fn a_long_expansion_is_cut_short_and_says_how_much_was_left_out() {
    beatscript_api::generate::install();
    let site = &sites(WHOLE)[0];
    let total = beatscriptls::refs::expansion(site).split('\n').count();
    assert!(total > beatscriptls::refs::HOVER_LINES, "{total}");
    let mut s = server(WHOLE);
    let md = hover_text(&mut s, WHOLE, WHOLE.find("gen@2").unwrap());
    assert!(md.contains(&format!("({total} lines,")), "{md}");
    assert!(md.contains("more lines not shown") && md.contains(&format!("({total} in all)")), "{md}");
    assert!(md.lines().count() < total, "the hover is cut short");
}

#[test]
fn expand_answers_every_reference_with_its_ranges_in_utf16() {
    beatscript_api::generate::install();
    // a comment of characters that count differently in UTF-8 and UTF-16 before the references
    let text = format!("# é𝄞 — a bed from the generator\n{PARTS}");
    let ss = sites(&text);
    let mut s = server(&text);
    let got = req(&mut s, "beatscript/expand", &doc());
    let list = got.get("references").and_then(JsonValue::as_array).unwrap();
    assert_eq!(list.len(), ss.len());
    for (r, site) in list.iter().zip(&ss) {
        assert_eq!(span_of(&text, r.get("range").unwrap(), Encoding::Utf16), (site.span.start, site.span.end));
        assert_eq!(span_of(&text, r.get("replaced").unwrap(), Encoding::Utf16), (site.replaced.start, site.replaced.end));
        let f = |k: &str| r.get(k).and_then(JsonValue::as_str).map(str::to_string);
        assert_eq!(f("kind").as_deref(), Some(site.kind));
        assert_eq!(f("half").as_deref(), Some(site.half));
        assert_eq!(f("call"), Some(site.call.clone()));
        assert_eq!(f("resolved"), site.resolved);
        assert_eq!(f("generator"), site.generator);
        assert_eq!(f("place").as_deref(), Some(site.place));
        assert_eq!(f("name"), site.name);
        assert_eq!(f("text").as_deref(), Some(beatscriptls::refs::expansion(site)));
        assert_eq!(f("from"), Some(refs::from_line(site)));
        assert_eq!(r.get("fitted").and_then(JsonValue::strings), Some(site.fitted.clone()));
    }
    golden("expand-parts", &got);
    // one: the reference at a position
    let one = req(&mut s, "beatscript/expand", &at_byte(&text, text.find("seed(").unwrap()));
    let one = one.get("references").and_then(JsonValue::as_array).unwrap();
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].get("kind").and_then(JsonValue::as_str), Some("seed"));
    // nothing at a plain part
    let none = req(&mut s, "beatscript/expand", &at_byte(&text, text.find("kick x").unwrap()));
    assert_eq!(none.get("references").and_then(JsonValue::as_array).map(Vec::len), Some(0));
}

/// The range's characters are UTF-16 units: a quoted phrase with a character of two UTF-8 bytes in the reference itself.
#[test]
fn a_reference_with_non_ascii_text_in_it_has_a_utf16_range() {
    let text = "arrange from template(deep-house, \"the café is open\")\n";
    let ss = sites(text);
    let mut s = server(text);
    let got = req(&mut s, "beatscript/expand", &doc());
    let r = got.get("references").and_then(|l| l.at(0)).unwrap();
    let end = r.get("range").and_then(|g| g.get("end")).and_then(|e| e.get("character")).and_then(JsonValue::as_i64).unwrap();
    let line = text.lines().next().unwrap();
    let in_bytes = ss[0].span.end as i64;
    assert_eq!(end, line[..ss[0].span.end].encode_utf16().count() as i64);
    assert_eq!(end, in_bytes - 1, "é is two bytes and one unit");
}

#[test]
fn expand_says_what_is_wrong_and_a_program_without_references_has_none() {
    beatscript_api::generate::install();
    let bad = "let b = gen@2 tecno seed=1 section=drop part=bass\n";
    let mut s = server(bad);
    let got = req(&mut s, "beatscript/expand", &doc());
    assert_eq!(got.get("references").and_then(JsonValue::as_array).map(Vec::len), Some(0));
    let errs = got.get("errors").and_then(JsonValue::as_array).unwrap();
    assert!(errs.iter().any(|e| e.get("message").and_then(JsonValue::as_str).is_some_and(|m| m.contains("tecno"))), "{}", got.js_stringified());
    assert_eq!(req(&mut s, "textDocument/codeLens", &doc()).as_array().map(Vec::len), Some(0));
    let plain = "deck A: kick x ~ x ~ | bass c2 ~ c2 ~\n";
    let mut p = server(plain);
    assert!(!beatscriptls::refs::may_have(&Analysis::new(plain)));
    assert_eq!(req(&mut p, "beatscript/expand", &doc()).js_stringified(), r#"{"references":[]}"#);
}

/// The eject code action of the reference at `at`: offered unresolved, resolved to an edit.
fn eject_action(s: &mut Server, text: &str, at: usize) -> (JsonValue, JsonValue) {
    let p = LineIndex::new(text).position(text, at, Encoding::Utf16);
    let params = format!(r#"{{"textDocument":{{"uri":"{URI}"}},"range":{{"start":{{"line":{l},"character":{c}}},"end":{{"line":{l},"character":{c}}}}},"context":{{"diagnostics":[],"only":["refactor.inline"]}}}}"#, l = p.line, c = p.character);
    let list = req(s, "textDocument/codeAction", &params);
    let acts = list.as_array().unwrap();
    let eject = acts.iter().find(|a| a.get("data").and_then(|d| d.get("refactor")).and_then(JsonValue::as_str) == Some("eject")).unwrap_or_else(|| panic!("no eject at {at}: {}", list.js_stringified())).clone();
    assert!(eject.get("edit").is_none(), "made on resolve");
    assert_eq!(eject.get("kind").and_then(JsonValue::as_str), Some("refactor.inline"));
    let copy = acts.iter().find(|a| a.get("command").and_then(|c| c.get("command")).and_then(JsonValue::as_str) == Some(beatscriptls::refs::COPY));
    assert!(copy.is_some(), "a copy action: {}", list.js_stringified());
    let resolved = req(s, "codeAction/resolve", &eject.js_stringified());
    (eject, resolved)
}

#[test]
fn eject_is_offered_and_resolves_to_what_refs_eject_writes_proved() {
    beatscript_api::generate::install();
    let st = stored();
    for text in [LET, PARTS, SECTIONS, WHOLE, st.as_str()] {
        let ss = sites(text);
        assert!(!ss.is_empty(), "{text}");
        let mut s = server(text);
        for site in &ss {
            let (_, resolved) = eject_action(&mut s, text, site.span.start);
            let got = apply(text, resolved.get("edit").unwrap());
            let want = refs::eject(text, &env(), site.span.start).unwrap().unwrap();
            assert_eq!(got, want, "{text}\n--- ejected {}", site.call);
            // proved: the same nets (resolve refuses otherwise); and the provenance line says the call and its CID
            let a = Analysis::checked(text);
            let made = beatscriptls::refs::eject_made(&a, site.span.start).unwrap();
            beatscriptls::refactor::verify(&a, &made).unwrap();
            assert!(got.contains(&refs::from_line(site)), "{got}");
        }
    }
    // the title names the call; a CID is shortened
    let mut s = server(&st);
    let (eject, _) = eject_action(&mut s, &st, st.find("let riff").unwrap() + 12);
    let title = eject.get("title").and_then(JsonValue::as_str).unwrap();
    assert!(title.starts_with("Eject `z…") && title.ends_with(".drop.bass` into local text") && title.len() < 60, "{title}");
}

#[test]
fn nothing_to_eject_off_a_reference() {
    beatscript_api::generate::install();
    let mut s = server(LET);
    let at = LET.find("kick x").unwrap();
    let p = LineIndex::new(LET).position(LET, at, Encoding::Utf16);
    let params = format!(r#"{{"textDocument":{{"uri":"{URI}"}},"range":{{"start":{{"line":{l},"character":{c}}},"end":{{"line":{l},"character":{c}}}}},"context":{{"diagnostics":[]}}}}"#, l = p.line, c = p.character);
    let list = req(&mut s, "textDocument/codeAction", &params);
    assert!(!list.js_stringified().contains("\"eject\""), "{}", list.js_stringified());
}

#[test]
fn the_ejected_from_line_hovers_as_provenance() {
    beatscript_api::generate::install();
    let site = &sites(LET)[0];
    let out = refs::eject(LET, &env(), site.span.start).unwrap().unwrap();
    let mut s = server(&out);
    let md = hover_text(&mut s, &out, out.find("# from:").unwrap() + 4);
    assert!(md.contains(&site.call) && md.contains(site.resolved.as_deref().unwrap()) && md.contains("ejected"), "{md}");
}

#[test]
fn code_lenses_and_their_commands_round_trip() {
    beatscript_api::generate::install();
    let ss = sites(SECTIONS);
    let mut s = server_applying(SECTIONS);
    let lenses = req(&mut s, "textDocument/codeLens", &doc());
    golden("codelens-sections", &lenses);
    let lenses = lenses.as_array().unwrap().clone();
    assert_eq!(lenses.len(), 2 * ss.len());
    let titles: Vec<&str> = lenses.iter().filter_map(|l| l.get("command").and_then(|c| c.get("title")).and_then(JsonValue::as_str)).collect();
    assert_eq!(titles, ["show expansion", "eject", "show expansion", "eject"]);
    for (k, site) in ss.iter().enumerate() {
        let toggle = lenses[2 * k].get("command").unwrap();
        assert_eq!(toggle.get("command").and_then(JsonValue::as_str), Some(beatscriptls::refs::TOGGLE));
        assert_eq!(span_of(SECTIONS, lenses[2 * k].get("range").unwrap(), Encoding::Utf16), (site.span.start, site.span.end));
        // show: the reference, as beatscript/expand has it at that place
        let shown = req(&mut s, "workspace/executeCommand", &toggle.js_stringified());
        let at = req(&mut s, "beatscript/expand", &at_byte(SECTIONS, site.span.start));
        assert_eq!(shown.js_stringified(), at.get("references").and_then(|r| r.at(0)).unwrap().js_stringified());
        // copy: what eject writes, its provenance line first
        let copy = format!(r#"{{"command":"{}","arguments":{}}}"#, beatscriptls::refs::COPY, toggle.get("arguments").unwrap().js_stringified());
        let copied = req(&mut s, "workspace/executeCommand", &copy);
        assert_eq!(copied.get("text").and_then(JsonValue::as_str), Some(format!("{}\n{}", refs::from_line(site), beatscriptls::refs::expansion(site)).as_str()));
    }
    // eject over the wire: the edit is sent as workspace/applyEdit, then the answer
    let eject = lenses[1].get("command").unwrap();
    assert_eq!(eject.get("command").and_then(JsonValue::as_str), Some(beatscriptls::refs::EJECT));
    let msg = JsonValue::parse(&format!(r#"{{"jsonrpc":"2.0","id":9,"method":"workspace/executeCommand","params":{}}}"#, eject.js_stringified())).unwrap();
    let out = s.handle(&msg);
    assert_eq!(out.len(), 2, "{:?}", out.iter().map(JsonValue::js_stringified).collect::<Vec<_>>());
    assert_eq!(out[0].get("method").and_then(JsonValue::as_str), Some("workspace/applyEdit"));
    assert_eq!(out[0].get("id").and_then(JsonValue::as_str), Some("beatscript/1"));
    let edit = out[0].get("params").and_then(|p| p.get("edit")).unwrap();
    let want = refs::eject(SECTIONS, &env(), ss[0].span.start).unwrap().unwrap();
    assert_eq!(apply(SECTIONS, edit), want);
    assert_eq!(out[1].get("id").and_then(JsonValue::as_i64), Some(9));
    assert_eq!(apply(SECTIONS, out[1].get("result").and_then(|r| r.get("edit")).unwrap()), want);
    // the client's answer to our request is taken quietly
    assert!(s.handle(&JsonValue::parse(r#"{"jsonrpc":"2.0","id":"beatscript/1","result":{"applied":true}}"#).unwrap()).is_empty());
    // a show over the wire also tells a client with no show of its own
    let msg = JsonValue::parse(&format!(r#"{{"jsonrpc":"2.0","id":10,"method":"workspace/executeCommand","params":{}}}"#, lenses[0].get("command").unwrap().js_stringified())).unwrap();
    let out = s.handle(&msg);
    assert_eq!(out[0].get("method").and_then(JsonValue::as_str), Some("window/showMessage"));
    assert!(out[0].get("params").and_then(|p| p.get("message")).and_then(JsonValue::as_str).unwrap().starts_with(&ss[0].call));
    // a lens made for another text is refused
    let mut stale = eject.clone();
    if let JsonValue::Object(o) = &mut stale {
        let mut arg = eject.get("arguments").and_then(|a| a.at(0)).unwrap().clone();
        if let JsonValue::Object(x) = &mut arg {
            x.set("length", JsonValue::int(SECTIONS.len() as i64 + 1));
        }
        o.set("arguments", JsonValue::Array(vec![arg]));
    }
    let Err((_, m)) = s.request("workspace/executeCommand", &stale) else { panic!("ejected into a changed text") };
    assert!(m.contains("changed"), "{m}");
    let Err(_) = s.request("workspace/executeCommand", &JsonValue::parse(r#"{"command":"beatscript.nope","arguments":[]}"#).unwrap()) else { panic!("an unknown command") };
}

#[test]
fn the_server_says_it_has_lenses_commands_and_expand() {
    let s = server("deck A: kick x\n");
    let c = s.capabilities();
    assert_eq!(c.get("codeLensProvider").map(JsonValue::js_stringified).as_deref(), Some(r#"{"resolveProvider":false}"#));
    assert_eq!(c.get("executeCommandProvider").and_then(|e| e.get("commands")).and_then(JsonValue::strings), Some(beatscriptls::refs::COMMANDS.iter().map(|c| c.to_string()).collect()));
    assert_eq!(c.get("experimental").and_then(|e| e.get("beatscript")).and_then(|b| b.get("expand")).and_then(JsonValue::as_bool), Some(true));
}
