//! beatscript-api's answers, one by one: goldens for the corpus and the extra programs, error cases (line, column,
//! expected, help; unicode; CRLF), fmt idempotence and respacing, open and share round trips with forged
//! links refused, the asset and rights flags, UTF-16 offsets, highlighting, generation and nets.

mod support;

use djengine::json::JsonValue;
use support::*;

fn check(text: &str) -> JsonValue {
    let r = parse(&beatscript_api::check_json(text));
    assert_report_invariants(text, &r);
    r
}

fn prog(name: &str) -> String {
    all_programs().into_iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("no program {name}")).1
}

fn first_error(r: &JsonValue) -> &JsonValue {
    get(r, "diagnostics").as_array().unwrap().iter().find(|d| s(d, "severity") == "error").expect("an error")
}

// ---------- goldens ----------

#[test]
fn golden_json_per_program() {
    let bless = std::env::var("BEATSCRIPT_BLESS").is_ok();
    let dir = dir("golden");
    std::fs::create_dir_all(&dir).unwrap();
    let mut seen = Vec::new();
    for (name, text) in all_programs() {
        let report = parse(&beatscript_api::check_json(&text));
        let fmt = parse(&beatscript_api::fmt_json(&text));
        let share = parse(&beatscript_api::share_json(&text, None, false, ""));
        let doc = JsonValue::Object(djengine::json::JsonObject::from_pairs([("check", report), ("fmt", fmt), ("share", share)]));
        let want = pretty(&doc);
        let path = dir.join(format!("{name}.json"));
        seen.push(path.clone());
        if bless {
            std::fs::write(&path, &want).unwrap();
            continue;
        }
        let have = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{}: no golden; run with BEATSCRIPT_BLESS=1", path.display()));
        if have != want {
            let (a, b) = (have.lines().collect::<Vec<_>>(), want.lines().collect::<Vec<_>>());
            let at = a.iter().zip(b.iter()).position(|(x, y)| x != y).unwrap_or(a.len().min(b.len()));
            panic!("{name}: the answer changed from {} at line {}:\n  golden: {}\n  now:    {}\n(BEATSCRIPT_BLESS=1 cargo test -p beatscript-api --test api rewrites the goldens)", path.display(), at + 1, a.get(at).unwrap_or(&""), b.get(at).unwrap_or(&""));
        }
    }
    // no stale goldens
    for e in std::fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        assert!(seen.contains(&p), "{} belongs to no program", p.display());
    }
}

#[test]
fn what_the_corpus_says_about_itself() {
    let v2 = check(&prog("hand-dubstep-140-v2"));
    assert_eq!(s(&v2, "cid"), "z4EBG9j9cjqaYU5uVNFH6YKAMoiyPfNkof87KEXjPNPpAhJzDhi");
    assert!(b(&v2, "ok") && b(&v2, "arranged") && b(&v2, "ends") && !b(&v2, "loops"));
    assert_eq!((n(&v2, "song_bars"), n(&v2, "end_bar")), (64, 64));
    assert_eq!(get(&v2, "seconds").as_f64(), Some(109.714));
    assert_eq!(get(&v2, "tempo").as_f64(), Some(140.0));
    assert_eq!(s(&v2, "key"), "fm");
    let names: Vec<&str> = get(&v2, "sections").as_array().unwrap().iter().map(|x| s(x, "name")).collect();
    assert_eq!(names, ["intro", "build", "drop", "breakdown", "drop2", "outro"]);
    assert_eq!((n(&v2, "sections.2.start"), n(&v2, "sections.2.bars")), (16, 16));
    assert_eq!(get(&v2, "decks").js_stringified(), r#"["A","B"]"#);
    // the doc's own numbers: trance-138's CID and its longhand nets
    let t = check(&prog("trance-138"));
    assert_eq!(s(&t, "cid"), "z4EBG9jDzsE2UHZfL8A9ouKLU5vUJsgHC8nVaTE7sWjci7PCukZ");
    assert_eq!(s(&t, "song_nets.A"), "z4EBG9jG8nT2p449McscV7ZSCfmUJDiFxFU26JpZhJ9hfBFgeLD");
    assert_eq!(s(&t, "song_nets.B"), "z4EBG9j8hbXoGqrw8qMseuz7BhibdkMiee4USmUaSyPRgJjiiLJ");
    // an evolve in a section is reported with its tail and period and its lineage, and is still one section
    let m = check(&prog("markers-demo"));
    assert!(s(&m, "evolve.0").starts_with("evolve in drop: tail "), "{}", s(&m, "evolve.0"));
    assert_eq!(get(&m, "lineage").as_array().unwrap().len(), 4);
    assert_eq!((s(&m, "lineage.1.scope"), n(&m, "lineage.1.gen")), ("drop", 1));
    assert_eq!(s(&m, "lineage.1.forkedFrom"), s(&m, "lineage.0.cid"));
    assert!(matches!(get(&m, "lineage.0.forkedFrom"), JsonValue::Null));
    assert_eq!(get(&m, "sections").as_array().unwrap().len(), 2);
    assert_eq!((n(&m, "sections.1.start"), n(&m, "sections.1.bars")), (8, 16));
}

#[test]
fn song_length_follows_end_loop_and_sections() {
    let e = check(&prog("sections-end"));
    assert_eq!((n(&e, "song_bars"), n(&e, "end_bar")), (20, 12), "the timeline is 20 bars, `end after bridge` stops at 12");
    assert_eq!(get(&e, "seconds").as_f64(), Some(28.8));
    assert_eq!(get(&e, "sections").as_array().unwrap().len(), 4, "verse is named twice and is two sections");
    let l = check(&prog("sections-loop"));
    assert!(b(&l, "loops") && !b(&l, "ends"));
    assert_eq!(get(&l, "seconds").as_f64(), Some(21.333), "a loop's length is one lap");
    // an evolve with no sections plays its own orbit and ends; it has no sections to list
    let f = check(&prog("evolve-flat"));
    assert!(b(&f, "arranged") && b(&f, "ends"));
    assert_eq!(get(&f, "sections").as_array().unwrap().len(), 0);
    assert_eq!((n(&f, "song_bars"), get(&f, "seconds").as_f64()), (32, Some(64.0)));
    // a program with no timeline has no length
    let d = check(&prog("do-lines"));
    assert!(!b(&d, "arranged") && matches!(get(&d, "song_bars"), JsonValue::Null) && matches!(get(&d, "seconds"), JsonValue::Null));
    // a program too long for one net chains and says so
    let c = check(&prog("chained"));
    assert!(b(&c, "ok") && b(&c, "loops"));
    assert!(s(&c, "diagnostics.0.message").contains("85 bars"), "{}", s(&c, "diagnostics.0.message"));
    // swing and the key's mode
    let w = check(&prog("swing-settings"));
    assert_eq!((get(&w, "tempo").as_f64(), get(&w, "swing").as_f64(), s(&w, "key")), (Some(87.5), Some(0.15), "d:minpent"));
}

// ---------- errors ----------

#[test]
fn an_error_has_line_col_expected_found_help_and_a_caret() {
    // the example of docs/BEATSCRIPT.md, "Errors"
    let r = check("deck A: kick | snare ~ x ~ x\n");
    assert!(!b(&r, "ok"));
    assert!(matches!(get(&r, "nets"), JsonValue::Object(o) if o.is_empty()));
    let d = first_error(&r);
    assert_eq!((n(d, "line"), n(d, "col"), n(d, "end_col")), (1, 14, 15));
    assert_eq!((n(d, "start"), n(d, "end")), (13, 14));
    assert_eq!(s(d, "message"), "`kick` has no pattern");
    assert!(get(d, "expected").as_array().unwrap().iter().any(|e| e.as_str().is_some_and(|e| e.contains("`x`"))));
    assert_eq!(s(d, "found"), "`|`");
    assert!(get(d, "help").as_str().is_some_and(|h| h.contains("kick x")), "{}", get(d, "help").js_stringified());
    let render = s(d, "render");
    assert!(render.starts_with("error: ") && render.contains("--> 1:14") && render.contains("^") && render.contains("= expected:") && render.contains("= help:"), "{render}");
}

#[test]
fn names_that_are_wrong_come_with_the_nearest_ones() {
    let r = check("tempo 140\ndeck A: bass c2 ~ c2 ~ inst=wobble-lad\n");
    let d = first_error(&r);
    assert_eq!((n(d, "line"), n(d, "col")), (2, 29));
    assert!(s(d, "message").contains("wobble-lad"), "{}", s(d, "message"));
    assert!(get(d, "help").as_str().is_some_and(|h| h.contains("wobble-lead")), "{}", get(d, "help").js_stringified());
    // a family word is accepted, with a warning that says what plays
    let r = check("deck A: bass c2 ~ c2 ~ inst=reese\n");
    assert!(b(&r, "ok"), "{}", r.js_stringified());
    assert_eq!((n(&r, "errors"), n(&r, "warnings")), (0, 1));
    assert!(s(&r, "diagnostics.0.message").contains("reese"));
    // a kit that is not one
    let r = check("deck A: kick x ~ x ~ kit=drums-909\n");
    assert!(!b(&r, "ok") && s(first_error(&r), "message").contains("drums-909"), "{}", r.js_stringified());
}

#[test]
fn do_lines_are_checked_against_the_registry() {
    let r = check("do vocoer on\n");
    assert_eq!(get(first_error(&r), "help").as_str(), Some("did you mean `vocoder`? (`describe` lists the nouns)"));
    let r = check("do vocal create bars=2\n");
    assert!(s(first_error(&r), "message").contains("needs `lyrics`"), "{}", s(first_error(&r), "message"));
    let r = check("do mixer decks=A\n");
    assert_eq!(s(first_error(&r), "message"), "`mixer` has no param `decks`");
    let r = check("do vocal creat lyrics=\"x\"\n");
    assert_eq!(get(first_error(&r), "help").as_str(), Some("did you mean `create`?"));
    let r = check("do vocoder set mix=0 -> 1\n");
    assert!(s(first_error(&r), "message").contains("needs a length"), "{}", s(first_error(&r), "message"));
    // and the right ones are fine
    let r = check("do vocoder set mix=0 -> 1 over 8 bars\nvocal create \"robots on the road\" key=am bars=4\nwheel sync deck=A ratio=1/4\n");
    assert!(b(&r, "ok"), "{}", r.js_stringified());
}

#[test]
fn an_import_says_there_is_no_snippet_store() {
    let text = "import \"night bus\"\ndeck A: kick x ~ x ~\n";
    let r = check(text);
    assert!(!b(&r, "ok"));
    let d = first_error(&r);
    assert_eq!(s(d, "message"), "`import` is not available on the web: there is no snippet store here");
    assert_eq!(s(d, "help"), "paste the snippet's `let` and `def` lines into the program instead");
    assert!(s(d, "render").contains("= help: paste the snippet's"), "{}", s(d, "render"));
    assert_eq!(n(d, "line"), 1);
}

#[test]
fn unicode_in_the_text_moves_columns_in_characters_and_offsets_in_bytes_and_utf16() {
    // 🎵 is four bytes and two UTF-16 units; é two bytes; ✓ three bytes, one unit and the thing the language cannot read
    let text = "# 🎵 voilà é\ndeck A: kick x ~ ✓ ~\n";
    let r = check(text);
    assert!(!b(&r, "ok"));
    let d = first_error(&r);
    assert_eq!((n(d, "line"), n(d, "col")), (2, 18), "{}", d.js_stringified());
    let line1 = "# 🎵 voilà é\n";
    assert_eq!(n(d, "start") as usize, line1.len() + 17);
    assert_eq!(n(d, "start16") as usize, line1.encode_utf16().count() + 17);
    assert!(n(d, "start16") < n(d, "start"), "the UTF-16 offset is smaller than the byte offset once there are multi-byte characters before it");
    assert_eq!(&text[n(d, "start") as usize..n(d, "end") as usize], "✓");
    // an error on the first line, after the astral character
    let r = check("# 🎵\ndeck A: kick x ~ x ~\ndeck B: nonesuch x\n");
    let d = first_error(&r);
    assert_eq!(n(d, "line"), 3);
}

#[test]
fn crlf_text_reads_the_same_and_keeps_the_offsets_of_the_text_given() {
    let lf = "tempo 120\ndeck A: kick x ~ q\n";
    let crlf = lf.replace('\n', "\r\n");
    let (a, c) = (check(lf), check(&crlf));
    let (da, dc) = (first_error(&a), first_error(&c));
    assert_eq!((n(da, "line"), n(da, "col")), (n(dc, "line"), n(dc, "col")));
    assert_eq!(n(dc, "start"), n(da, "start") + 1, "the CR of line one is in the text given");
    assert!(!s(dc, "render").contains('\r'));
    // a good program has the same CID either way
    for (name, text) in all_programs() {
        let crlf = text.replace('\n', "\r\n");
        let (x, y) = (check(&text), check(&crlf));
        assert_eq!(b(&x, "ok"), b(&y, "ok"), "{name}");
        if b(&x, "ok") {
            assert_eq!(s(&x, "cid"), s(&y, "cid"), "{name}: CRLF changed the CID");
            assert_eq!(s(&x, "canonical"), s(&y, "canonical"), "{name}");
        }
    }
}

#[test]
fn odd_texts_are_answers_not_crashes() {
    for text in ["", "\n", "   \n\t\n", "# only a comment\n", "\u{feff}tempo 120\n", "tempo", "deck", "deck A:", "}", "{{{{", "((((", "\"unterminated", "section x 4 bars {", "x".repeat(5000).as_str(), "\0\0\0"] {
        let r = check(text);
        assert_eq!(b(&r, "ok"), n(&r, "errors") == 0);
        let _ = beatscript_api::fmt_json(text);
        let _ = beatscript_api::highlight_json(text);
        let _ = beatscript_api::highlight_json_utf16(text);
        let _ = beatscript_api::share_json(text, None, false, "");
        let _ = beatscript_api::nets_json(text, false);
        let _ = beatscript_api::generation_json(text, None, 3);
    }
    // nothing is a program with nothing in it
    let r = check("# nothing\n");
    assert!(b(&r, "ok") && !b(&r, "arranged"));
}

// ---------- fmt ----------

#[test]
fn fmt_is_the_canonical_text_its_cid_and_the_comments() {
    for (name, text) in all_programs() {
        let f = parse(&beatscript_api::fmt_json(&text));
        let c = check(&text);
        assert_eq!(b(&f, "ok"), b(&c, "ok"), "{name}");
        if !b(&c, "ok") {
            assert!(matches!(get(&f, "canonical"), JsonValue::Null));
            continue;
        }
        // fmt is the program in BeatScript 2 (djlive::version): its canonical text is check's with the version line first, its
        // CID the BeatScript 2 one, and `from_cid` the CID of the text as given when that is BeatScript 1
        assert_eq!(s(&f, "canonical"), djlive::version::upgrade_canonical(s(&c, "canonical")), "{name}");
        assert_eq!(s(&f, "cid"), djlive::compile::program_cid(s(&f, "canonical")), "{name}");
        assert_eq!(get(&f, "version").as_i64(), Some(2), "{name}");
        if s(&c, "canonical").starts_with(djlive::version::CANONICAL_HEAD) {
            assert_eq!(s(&f, "cid"), s(&c, "cid"), "{name}: a BeatScript 2 text formats to its own CID");
            assert!(matches!(get(&f, "from_cid"), JsonValue::Null), "{name}");
        } else {
            assert_eq!(s(&f, "from_cid"), s(&c, "cid"), "{name}");
            assert_eq!(s(&c, "renamed_cid"), s(&f, "cid"), "{name}: check names the CID fmt upgrades to");
        }
        // the formatted text checks as the program fmt says it is
        if let JsonValue::String(t) = get(&f, "text") {
            assert_eq!(s(&check(t), "cid"), s(&f, "cid"), "{name}");
        }
        // a fixed point: formatting the canonical text changes nothing
        let again = parse(&beatscript_api::fmt_json(s(&f, "canonical")));
        assert_eq!(s(&again, "canonical"), s(&f, "canonical"), "{name}");
        assert_eq!(s(&again, "cid"), s(&f, "cid"), "{name}");
        assert_eq!(get(&again, "comments").as_array().unwrap().len(), 0, "{name}: the canonical text has no comments (the version line is not one)");
    }
    let f = parse(&beatscript_api::fmt_json("# one\ntempo 120   # two\n#! license: CC0-1.0\n"));
    assert_eq!(get(&f, "comments").js_stringified(), r#"["one","two","! license: CC0-1.0"]"#);
}

fn respace(text: &str, rng: &mut Rng) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let mut in_str = false;
        let mut in_comment = false;
        let mut l = String::new();
        let trimmed = line.trim_start();
        if !trimmed.is_empty() {
            for _ in 0..rng.below(4) {
                l.push(if rng.below(3) == 0 { '\t' } else { ' ' });
            }
        }
        for c in trimmed.chars() {
            match c {
                '"' if !in_comment => in_str = !in_str,
                '#' if !in_str && l.trim().is_empty() || (c == '#' && !in_str && l.ends_with(' ')) => in_comment = true,
                _ => {}
            }
            if c == ' ' && !in_str && !in_comment {
                for _ in 0..=rng.below(3) {
                    l.push(if rng.below(4) == 0 { '\t' } else { ' ' });
                }
            } else {
                l.push(c);
            }
        }
        if !in_comment && !trimmed.is_empty() && rng.below(3) == 0 {
            l.push_str("   # a comment that is not part of the program");
        }
        out.push_str(&l);
        out.push('\n');
        for _ in 0..rng.below(3) {
            out.push_str(if rng.below(2) == 0 { "\n" } else { "   \t \n" });
        }
    }
    out
}

#[test]
fn respacing_a_program_never_changes_its_cid() {
    let mut rng = Rng(0x9e3779b97f4a7c15);
    for (name, text) in all_programs() {
        let base = check(&text);
        if !b(&base, "ok") {
            continue;
        }
        for round in 0..12 {
            let spaced = respace(&text, &mut rng);
            let r = check(&spaced);
            assert!(b(&r, "ok"), "{name} round {round}: {}\n{spaced}", r.js_stringified());
            assert_eq!(s(&r, "cid"), s(&base, "cid"), "{name} round {round}");
            assert_eq!(s(&r, "canonical"), s(&base, "canonical"), "{name} round {round}");
            assert_eq!(get(&r, "nets").js_stringified(), get(&base, "nets").js_stringified(), "{name} round {round}");
        }
    }
}

#[test]
fn changing_one_note_changes_the_cid() {
    let text = prog("transforms");
    let a = check(&text);
    let b2 = check(&text.replace("d2! ~ d2 f2^", "d2! ~ d2 g2^"));
    assert!(b(&b2, "ok") && s(&a, "cid") != s(&b2, "cid"));
    // a number's spelling does not
    let c = check(&prog("ramps").replace("8k", "8000").replace("0.2 -> 0.9", "0.20 -> 0.90"));
    assert_eq!(s(&c, "cid"), s(&check(&prog("ramps")), "cid"));
}

// ---------- share and open ----------

#[test]
fn a_link_opens_to_the_program_that_made_it_in_every_form() {
    let mut forms = 0;
    for (name, text) in all_programs() {
        let c = check(&text);
        let sh = parse(&beatscript_api::share_json(&text, None, false, ""));
        if !b(&c, "ok") || !b(&sh, "ok") {
            continue;
        }
        assert_eq!(s(&sh, "cid"), s(&c, "cid"), "{name}");
        let url = s(&sh, "url");
        assert!(url.starts_with("https://beatscript.app/#z"), "{url}");
        assert_eq!(n(&sh, "chars") as usize, url.chars().count());
        assert!(s(&sh, "social_url").starts_with("https://beatscript.app/c/z") && !s(&sh, "social_url").contains('#'));
        assert!(s(&sh, "beats_url").starts_with("https://beats.bitwrap.io/p#z"));
        // the site's two links seal the same part: the CID and the formatted text (comments and `#!` headers kept)
        let frag = url.split_once('#').unwrap().1;
        assert_eq!(s(&sh, "social_url").split_once("/c/").unwrap().1, frag);
        let formatted = s(&parse(&beatscript_api::fmt_json(&text)), "text").to_string();
        assert_eq!(unpacked(url), formatted, "{name}");
        // beats' form is djlive's own and packs the canonical text (beats' page hashes what it unpacks)
        let comp = djlive::compile::check_with(&text, &beatscript_api::web_env().env()).compiled.unwrap();
        assert_eq!(s(&sh, "beats_url"), djlive::share::encode(&comp, None, false));
        assert_eq!(unpacked(s(&sh, "beats_url")), s(&c, "canonical"), "{name}");
        for form in [url.to_string(), s(&sh, "social_url").to_string(), s(&sh, "beats_url").to_string(), format!("#{frag}"), frag.to_string(), format!("  {url}\n")] {
            let o = parse(&beatscript_api::open_json(&form));
            assert!(b(&o, "ok") && b(&o, "verified"), "{name} {form}: {}", o.js_stringified());
            assert_eq!(s(&o, "cid"), s(&c, "cid"), "{name}");
            assert_eq!(s(&o, "canonical"), s(&c, "canonical"), "{name}");
            // the editor's text: the program as written (formatted), or the canonical text where the link carries only that
            let want = if form.contains("beats.bitwrap.io") { s(&c, "canonical") } else { formatted.as_str() };
            assert_eq!(s(&o, "text"), want, "{name} {form}");
            assert!(matches!(get(&o, "error"), JsonValue::Null));
            assert_eq!(get(&o, "notes_unverified").as_array().unwrap().len(), 0);
            forms += 1;
        }
    }
    assert!(forms >= 60, "{forms}");
}

#[test]
fn a_custom_origin_and_a_signature_and_notes_travel_as_given() {
    let text = "# a note\ntempo 120\ndeck A: kick x ~ x ~\n";
    let sh = parse(&beatscript_api::share_json(text, Some("sigAAAA_-"), true, "http://127.0.0.1:18097/"));
    // the comment is in the packed text, so the site's link needs no `~notes`
    assert!(s(&sh, "url").starts_with("http://127.0.0.1:18097/#z") && s(&sh, "url").ends_with(".sigAAAA_-"), "{}", s(&sh, "url"));
    assert!(s(&sh, "social_url").starts_with("http://127.0.0.1:18097/c/z") && s(&sh, "social_url").ends_with(".sigAAAA_-") && !s(&sh, "social_url").contains('~'), "{}", s(&sh, "social_url"));
    assert!(get(&sh, "warnings").as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("signature")));
    for u in [s(&sh, "url"), s(&sh, "social_url")] {
        let o = parse(&beatscript_api::open_json(u));
        assert!(b(&o, "ok"), "{}", o.js_stringified());
        assert_eq!(s(&o, "sig"), "sigAAAA_-");
        assert_eq!(s(&o, "text"), text);
        assert!(get(&o, "notes_unverified").as_array().unwrap().is_empty());
        assert!(get(&o, "warnings").as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("unsealed")));
    }
    // beats' link carries them as `~notes`, unverified, beside the canonical text
    assert!(s(&sh, "beats_url").contains('~'), "{}", s(&sh, "beats_url"));
    let o = parse(&beatscript_api::open_json(s(&sh, "beats_url")));
    assert!(b(&o, "ok") && s(&o, "sig") == "sigAAAA_-", "{}", o.js_stringified());
    assert_eq!(get(&o, "notes_unverified").js_stringified(), r#"["a note"]"#);
    assert_eq!(s(&o, "text"), "tempo 120\ndeck A: kick x ~ x ~\n");
    // no notes asked: none in beats' link; the site's link still carries the comment in its text
    let sh = parse(&beatscript_api::share_json(text, None, false, ""));
    assert!(!s(&sh, "url").contains('~') && !s(&sh, "beats_url").contains('~'));
    assert_eq!(unpacked(s(&sh, "url")), text);
}

/// The text a link packs (the part after `<CID>.`, inflated).
fn unpacked(url: &str) -> String {
    let frag = url.split_once('#').map_or(url, |(_, f)| f);
    let frag = frag.split_once("/c/").map_or(frag, |(_, f)| f);
    let packed = frag.split(['~', '*']).next().unwrap().split('.').nth(1).unwrap();
    let raw = miniz_oxide::inflate::decompress_to_vec(&djengine::codec::base64_url_decode(packed).unwrap()).unwrap();
    String::from_utf8(raw).unwrap()
}

fn pack(text: &str) -> String {
    djengine::codec::base64_url(&miniz_oxide::deflate::compress_to_vec(text.as_bytes(), 10))
}

fn rag_rondo() -> String {
    std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/beatscript/rag-rondo.beatscript")).unwrap()
}

#[test]
fn a_link_keeps_the_header_and_the_comments_and_opens_to_them() {
    let text = rag_rondo();
    let fmt = parse(&beatscript_api::fmt_json(&text));
    let formatted = s(&fmt, "text").to_string();
    assert!(formatted.starts_with("#! name: Rag Rondo\n") && formatted.contains("\n# rag-rondo: the reference program"), "{formatted}");
    let sh = parse(&beatscript_api::share_json(&text, None, false, ""));
    assert!(b(&sh, "ok"), "{}", sh.js_stringified());
    assert_eq!(unpacked(s(&sh, "url")), formatted);
    for u in [s(&sh, "url"), s(&sh, "social_url")] {
        let o = parse(&beatscript_api::open_json(u));
        assert!(b(&o, "ok") && b(&o, "verified"), "{}", o.js_stringified());
        // the editor's text keeps the header, the comments and the blank lines; the canonical text is the program's
        assert_eq!(s(&o, "text"), formatted);
        assert_eq!(s(&o, "canonical"), s(&fmt, "canonical"));
        assert_eq!(s(&o, "cid"), s(&fmt, "cid"));
        // opened text formats to itself and shares to the same link: the round trip is a fixed point
        let again = parse(&beatscript_api::share_json(s(&o, "text"), None, false, ""));
        assert_eq!(s(&again, "url"), s(&sh, "url"));
        let w: Vec<&str> = get(&o, "warnings").as_array().unwrap().iter().map(|w| w.as_str().unwrap()).collect();
        assert!(w.iter().any(|w| w.contains("unsealed")) && !w.iter().any(|w| w.contains("not in canonical form")), "{w:?}");
    }
}

#[test]
fn the_cid_is_the_same_with_and_without_the_comments() {
    let text = rag_rondo();
    let with = parse(&beatscript_api::share_json(&text, None, false, ""));
    let canonical = s(&with, "canonical").to_string();
    assert!(!canonical.lines().any(|l| l.trim_start().starts_with('#')), "the canonical text has no comments or headers");
    let without = parse(&beatscript_api::share_json(&canonical, None, false, ""));
    assert_eq!(s(&with, "cid"), s(&without, "cid"));
    assert_eq!(s(&with, "cid"), "z4EBG9izUsbisU2JZtPHUo4hcQpYd8FLyZXLqKUmbtftjj3Lirw", "the CID rag-rondo.beatscript names");
    // the two links differ only in the packed text; each opens to its own text and the same program
    assert_ne!(s(&with, "url"), s(&without, "url"));
    for (sh, text) in [(&with, text.as_str()), (&without, canonical.as_str())] {
        let o = parse(&beatscript_api::open_json(s(sh, "url")));
        assert!(b(&o, "ok") && b(&o, "verified"));
        assert_eq!((s(&o, "cid"), s(&o, "canonical")), (s(&with, "cid"), canonical.as_str()));
        assert_eq!(s(&o, "text"), s(&parse(&beatscript_api::fmt_json(text)), "text"));
    }
    // the size the comments add (examples/beatscript/rag-rondo.beatscript: twelve header and comment lines over a 4.6 KB program)
    let (a, b2) = (n(&without, "chars"), n(&with, "chars"));
    eprintln!("rag-rondo link: {a} chars canonical only, {b2} chars with its header and comments (+{})", b2 - a);
    assert!(b2 > a);
}

#[test]
fn a_link_made_before_links_kept_the_comments_still_opens() {
    // links made by main bc5f1ce2 (before 7ec3fd76): the canonical text only, on btw.bitwrap.io (and two rag-rondo links
    // made there by `beatscript share`, one with `--notes`)
    let fixture = parse(&std::fs::read_to_string(dir("testdata").join("links-before-formatted.json")).unwrap());
    let links = get(&fixture, "links").as_array().unwrap();
    assert!(links.len() >= 10 && links.iter().any(|l| s(l, "program") == "rag-rondo --notes"));
    for l in links {
        let url = s(l, "url");
        assert!(url.starts_with("https://btw.bitwrap.io/#z"), "{url}");
        assert_eq!(unpacked(url), s(l, "canonical"), "an old link packs the canonical text");
        let o = parse(&beatscript_api::open_json(url));
        assert!(b(&o, "ok") && b(&o, "verified"), "{}: {}", s(l, "program"), o.js_stringified());
        assert_eq!(s(&o, "cid"), s(l, "cid"));
        assert_eq!(s(&o, "canonical"), s(l, "canonical"));
        // nothing beside the program came along in the text: the editor gets the canonical text, with no warning about it
        assert_eq!(s(&o, "text"), s(l, "canonical"), "{}", s(l, "program"));
        let w: Vec<&str> = get(&o, "warnings").as_array().unwrap().iter().map(|w| w.as_str().unwrap()).collect();
        assert!(!w.iter().any(|w| w.contains("canonical form") || w.contains("header lines")), "{}: {w:?}", s(l, "program"));
        // and the same program shared today gets the same CID
        let now = parse(&beatscript_api::share_json(s(l, "canonical"), None, false, ""));
        assert_eq!(s(&now, "cid"), s(l, "cid"));
    }
    // the old --notes link's comments are still read, as unverified notes
    let notes = links.iter().find(|l| s(l, "program") == "rag-rondo --notes").unwrap();
    let o = parse(&beatscript_api::open_json(s(notes, "url")));
    let nu = get(&o, "notes_unverified").as_array().unwrap();
    assert!(nu.len() >= 9 && nu.iter().any(|n| n.as_str().unwrap().contains("Scott Joplin")), "{}", o.js_stringified());
}

#[test]
fn a_tampered_link_is_refused_and_its_comments_are_unsealed() {
    let sh = parse(&beatscript_api::share_json(&rag_rondo(), None, false, ""));
    let cid = s(&sh, "cid").to_string();
    let text = unpacked(s(&sh, "url"));
    // a change to the music, under the same CID: refused
    for (from, to) in [("tempo 122", "tempo 123"), ("swing 0.18", "swing 0.2"), ("key c", "key d")] {
        assert!(text.contains(from), "{from}");
        let forged = format!("https://beatscript.app/#{cid}.{}", pack(&text.replacen(from, to, 1)));
        let o = parse(&beatscript_api::open_json(&forged));
        assert!(!b(&o, "ok") && !b(&o, "verified"), "{from} -> {to}: {}", o.js_stringified());
        assert!(s(&o, "error").starts_with("refused: the link says "), "{}", s(&o, "error"));
        assert!(matches!(get(&o, "text"), JsonValue::Null));
    }
    // a change to a comment or a header line is not covered by the CID: it opens, and says the comments are unsealed
    let edited = text.replacen("#! name: Rag Rondo", "#! name: Rag Rondo (edited)", 1);
    let o = parse(&beatscript_api::open_json(&format!("https://beatscript.app/#{cid}.{}", pack(&edited))));
    assert!(b(&o, "ok") && b(&o, "verified") && s(&o, "cid") == cid, "{}", o.js_stringified());
    assert!(s(&o, "text").starts_with("#! name: Rag Rondo (edited)\n"));
    assert!(get(&o, "warnings").as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("unsealed")));
}

#[test]
fn links_are_made_on_beatscript_app_unless_another_site_is_named() {
    assert_eq!(beatscript_api::DEFAULT_ORIGIN, "https://beatscript.app");
    assert_eq!(djlive::share::ORIGIN, "https://beatscript.app");
    for origin in ["", "  ", "https://beatscript.app", "https://beatscript.app/"] {
        let sh = parse(&beatscript_api::share_json("deck A: kick x ~ x ~\n", None, false, origin));
        assert!(s(&sh, "url").starts_with("https://beatscript.app/#z") && s(&sh, "social_url").starts_with("https://beatscript.app/c/z"), "{origin:?}: {}", sh.js_stringified());
        assert!(!sh.js_stringified().contains("btw.bitwrap.io"));
    }
    // a named site is kept (a local server, or the legacy host if someone asks for it)
    let sh = parse(&beatscript_api::share_json("deck A: kick x ~ x ~\n", None, false, "https://btw.bitwrap.io"));
    assert!(s(&sh, "url").starts_with("https://btw.bitwrap.io/#z"));
}

#[test]
fn a_forged_link_is_refused() {
    let (a, b2) = (prog("trance-138"), prog("matrix-130"));
    let (la, lb) = (parse(&beatscript_api::share_json(&a, None, false, "")), parse(&beatscript_api::share_json(&b2, None, false, "")));
    let (cid_a, cid_b) = (s(&la, "cid").to_string(), s(&lb, "cid").to_string());
    let packed = |l: &JsonValue| s(l, "url").split_once('#').unwrap().1.split_once('.').unwrap().1.to_string();
    let (packed_a, packed_b) = (packed(&la), packed(&lb));
    // A's text under B's CID, and the other way round
    for (cid, packed) in [(&cid_b, &packed_a), (&cid_a, &packed_b)] {
        for form in [format!("https://beatscript.app/#{cid}.{packed}"), format!("https://beatscript.app/c/{cid}.{packed}"), format!("{cid}.{packed}")] {
            let o = parse(&beatscript_api::open_json(&form));
            assert!(!b(&o, "ok") && !b(&o, "verified"), "{form}");
            assert!(s(&o, "error").starts_with("refused: the link says "), "{}", s(&o, "error"));
            assert_eq!(s(&o, "claimed_cid"), cid.as_str());
            assert!(matches!(get(&o, "canonical"), JsonValue::Null) && matches!(get(&o, "cid"), JsonValue::Null));
        }
    }
    // a damaged text: not base64, not deflate, cut short, not a program
    let good = s(&la, "url").to_string();
    for bad in [format!("https://beatscript.app/#{cid_a}.!!!!"), format!("https://beatscript.app/#{cid_a}.AAAA"), good[..good.len() - 40].to_string(), format!("https://beatscript.app/#{cid_a}.{}", &packed_a[..packed_a.len() / 2])] {
        let o = parse(&beatscript_api::open_json(&bad));
        assert!(!b(&o, "ok") && !b(&o, "verified"), "{bad}: {}", o.js_stringified());
        assert!(!s(&o, "error").is_empty());
    }
    // a CID with nothing to verify, and things that are not links
    let o = parse(&beatscript_api::open_json(&cid_a));
    assert!(!b(&o, "ok") && !b(&o, "has_text") && s(&o, "claimed_cid") == cid_a, "{}", o.js_stringified());
    assert!(s(&o, "error").contains("CID with no text"), "{}", s(&o, "error"));
    for junk in ["", "not a link at all", "https://example.com/", "#", "z"] {
        let o = parse(&beatscript_api::open_json(junk));
        assert!(!b(&o, "ok"), "{junk}");
    }
}

#[test]
fn a_link_to_a_program_that_no_longer_checks_still_opens_with_a_warning() {
    // the text is the canonical text of a program whose instrument name is not (any more) an engine voice: the
    // link is still verified (the CID is its text's) and the page can put the text in the editor to fix it
    use djlive::{parse::parse_with, print::canonical};
    let src = "deck A: bass c2 ~ c2 ~ inst=wobble-lead\n";
    let canon = canonical(&parse_with(src, &djlive::env::Env::none()).program);
    let bad = canon.replace("wobble-lead", "wobble-gone");
    let cid = djlive::compile::program_cid(&bad);
    // djlive makes links only of programs that check: pack the text by hand the way `encode` does
    let packed = djengine::codec::base64_url(&miniz_oxide::deflate::compress_to_vec(bad.as_bytes(), 10));
    let url = format!("https://beatscript.app/#{cid}.{packed}");
    let o = parse(&beatscript_api::open_json(&url));
    assert!(b(&o, "ok") && b(&o, "verified"), "{}", o.js_stringified());
    assert_eq!(s(&o, "cid"), cid);
    assert!(get(&o, "warnings").as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("does not check")), "{}", o.js_stringified());
}

#[test]
fn share_refuses_what_has_errors_or_is_private() {
    let sh = parse(&beatscript_api::share_json("deck A: kick | snare\n", None, false, ""));
    assert!(!b(&sh, "ok") && matches!(get(&sh, "url"), JsonValue::Null));
    assert!(s(&sh, "error").contains("has errors") && get(&sh, "diagnostics").as_array().unwrap().len() == 1);
    let sh = parse(&beatscript_api::share_json(&prog("assets-restricted"), None, false, ""));
    assert!(!b(&sh, "ok") && s(&sh, "error").starts_with("not shared:"), "{}", sh.js_stringified());
    // open assets are shareable, with a word about what a link carries
    let sh = parse(&beatscript_api::share_json(&prog("assets-open"), None, false, ""));
    assert!(b(&sh, "ok"), "{}", sh.js_stringified());
    assert!(get(&sh, "warnings").as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("hashes")));
}

// ---------- rights and assets ----------

#[test]
fn assets_and_restriction_are_read_from_the_text_with_no_file_lookup() {
    let flags = |name: &str| {
        let r = check(&prog(name));
        (b(&r, "ok"), b(&r, "uses_assets"), b(&r, "restricted"))
    };
    assert_eq!(flags("assets-open"), (true, true, false), "cc0 / cc-by samples and a loop: assets, not private");
    assert_eq!(flags("assets-restricted"), (true, true, true), "`use restricted` with a restricted sample");
    assert_eq!(flags("assets-forgot-restricted"), (false, true, true), "a restricted sample without `use restricted` is an error, and still flagged");
    assert_eq!(flags("transforms"), (true, false, false));
    // the error says how to fix it and where
    let r = check(&prog("assets-forgot-restricted"));
    let d = first_error(&r);
    assert_eq!((n(d, "line"), n(d, "col")), (3, 1));
    assert!(s(d, "message").contains("must declare `use restricted`") && s(d, "help").contains("use restricted"));
    // `use restricted` alone, with no sample, is private
    let r = check("use restricted\ndeck A: kick x ~ x ~\n");
    assert!(b(&r, "ok") && b(&r, "restricted") && !b(&r, "uses_assets"));
    // a file header that says private
    let r = check("#! visibility: private\ndeck A: kick x ~ x ~\n");
    assert!(b(&r, "ok") && b(&r, "restricted"));
    // the canonical text of an asset program names the sample by what the text says (no path was looked up)
    let r = check(&prog("assets-open"));
    assert!(s(&r, "canonical").contains("sha256:9f2c"), "{}", s(&r, "canonical"));
    let r = check(&prog("assets-restricted"));
    assert!(s(&r, "canonical").contains("~/samples/harp.wav"), "no file was looked up, so the path stays as written: {}", s(&r, "canonical"));
}

// ---------- highlight ----------

fn spans(text: &str) -> Vec<(usize, usize, String)> {
    parse(&beatscript_api::highlight_json(text)).as_array().unwrap().iter().map(|x| (x.at(0).unwrap().as_i64().unwrap() as usize, x.at(1).unwrap().as_i64().unwrap() as usize, x.at(2).unwrap().as_str().unwrap().to_string())).collect()
}

fn class_of(text: &str, token: &str) -> String {
    spans(text).into_iter().find(|(s, e, _)| &text[*s..*e] == token).unwrap_or_else(|| panic!("no span for {token}")).2
}

#[test]
fn tokens_get_the_classes_of_the_editor() {
    let t = "deck A: kick e(5,8) | snare ~ x ~ x vel=60..90 # hi\n";
    for (tok, class) in [("deck", "keyword"), ("A", "deck"), ("kick", "role"), ("e", "generator"), ("5", "number"), ("|", "operator"), ("~", "rest"), ("x", "step"), ("vel", "param"), ("# hi", "comment"), ("..", "operator")] {
        assert_eq!(class_of(t, tok), class, "{tok}");
    }
    let t = "let k = x ~\ndef g(deck) {\n  deck $deck: kick k\n}\nsection drop 8 bars {\n  g(A)\n}\narrange drop\nend after drop\nevolve every 4 bars {\n  melody: rotate 1 | transpose 2\n  stop when gen == 3\n}\n";
    for (tok, class) in [("let", "keyword"), ("def", "keyword"), ("section", "keyword"), ("arrange", "keyword"), ("end", "keyword"), ("evolve", "keyword"), ("rotate", "action"), ("transpose", "action"), ("$deck", "param"), ("stop", "action"), ("when", "keyword"), ("gen", "keyword"), ("drop", "word")] {
        assert_eq!(class_of(t, tok), class, "{tok}");
    }
    assert_eq!(class_of("do vocal create \"robots\" bars=2\n", "\"robots\""), "string");
    assert_eq!(class_of("deck A: kick x ~ ✓ ~\n", "✓"), "error");
    let t = "sample harp from \"sha256:ab\" license cc0 else pluck\nloop \"Dub 04\" as cyc\nuse restricted\n";
    for (tok, class) in [("sample", "keyword"), ("from", "keyword"), ("license", "keyword"), ("else", "keyword"), ("use", "keyword"), ("restricted", "keyword"), ("as", "keyword"), ("\"Dub 04\"", "string")] {
        assert_eq!(class_of(t, tok), class, "{tok}");
    }
}

#[test]
fn spans_are_in_order_whole_and_cover_every_visible_character() {
    const CLASSES: [&str; 14] = ["comment", "keyword", "role", "action", "deck", "number", "step", "rest", "generator", "param", "operator", "word", "string", "error"];
    for (name, text) in all_programs() {
        let sp = spans(&text);
        let mut at = 0;
        for (s0, e0, class) in &sp {
            assert!(*s0 >= at && s0 < e0 && *e0 <= text.len(), "{name}: span {s0}..{e0} after {at}");
            assert!(text.is_char_boundary(*s0) && text.is_char_boundary(*e0), "{name}");
            assert!(CLASSES.contains(&class.as_str()), "{name}: class {class}");
            at = *e0;
        }
        // everything that is not white space is in a span
        let mut covered = vec![false; text.len()];
        for (s0, e0, _) in &sp {
            covered[*s0..*e0].iter_mut().for_each(|c| *c = true);
        }
        for (i, c) in text.char_indices() {
            assert!(c.is_whitespace() || covered[i], "{name}: `{c}` at byte {i} has no class");
        }
    }
}

#[test]
fn utf16_highlighting_slices_a_js_string_to_the_same_tokens() {
    let text = "# 🎵 héllo\ndeck A: kick x ~ x ~ inst=wobble-lead # voilà ✓\ndo vocal create \"naïve 🎶\" bars=2\n";
    let bytes = spans(text);
    let u16s: Vec<u16> = text.encode_utf16().collect();
    let sp16 = parse(&beatscript_api::highlight_json_utf16(text));
    let sp16 = sp16.as_array().unwrap();
    assert_eq!(sp16.len(), bytes.len());
    for ((s8, e8, c8), j) in bytes.iter().zip(sp16) {
        let (s16, e16) = (n(j, "0") as usize, n(j, "1") as usize);
        assert_eq!(s(j, "2"), c8);
        assert_eq!(String::from_utf16(&u16s[s16..e16]).unwrap(), &text[*s8..*e8]);
    }
    assert_eq!(sp16.iter().filter(|j| s(j, "2") == "string").count(), 1);
    // in ASCII text the two agree
    assert_eq!(beatscript_api::highlight_json("deck A: kick x\n"), beatscript_api::highlight_json_utf16("deck A: kick x\n"));
}

// ---------- generation and nets ----------

#[test]
fn any_generation_of_an_evolve_by_cid() {
    let text = prog("markers-demo");
    let lineage = check(&text);
    for g in 0..4 {
        let j = parse(&beatscript_api::generation_json(&text, None, g));
        assert!(b(&j, "ok"), "{}", j.js_stringified());
        assert_eq!(s(&j, "cid"), s(&lineage, &format!("lineage.{g}.cid")), "generation {g}");
        assert_eq!((s(&j, "scope"), n(&j, "n")), ("drop", g as i64));
        let named = parse(&beatscript_api::generation_json(&text, Some("drop"), g));
        assert_eq!(s(&named, "cid"), s(&j, "cid"));
    }
    // far past the end it costs the orbit, not the number
    let far = parse(&beatscript_api::generation_json(&text, None, 1_000_000));
    assert!(b(&far, "ok"), "{}", far.js_stringified());
    // the flat program's own evolve
    let flat = parse(&beatscript_api::generation_json(&prog("evolve-flat"), None, 2));
    assert!(b(&flat, "ok") && s(&flat, "scope") == "evolve", "{}", flat.js_stringified());
    // no evolve, an unknown scope, a program with errors
    let none = parse(&beatscript_api::generation_json(&prog("ramps"), None, 1));
    assert!(!b(&none, "ok") && s(&none, "error").contains("no `evolve`"), "{}", none.js_stringified());
    let nope = parse(&beatscript_api::generation_json(&text, Some("intro"), 1));
    assert!(!b(&nope, "ok") && s(&nope, "error").contains("intro"), "{}", nope.js_stringified());
    let bad = parse(&beatscript_api::generation_json("deck A: kick |\n", None, 1));
    assert!(!b(&bad, "ok") && !get(&bad, "diagnostics").as_array().unwrap().is_empty());
}

#[test]
fn nets_are_the_check_reports_nets_with_their_envelopes() {
    for (name, text) in all_programs() {
        let c = check(&text);
        let j = parse(&beatscript_api::nets_json(&text, false));
        assert_eq!(b(&j, "ok"), b(&c, "ok"), "{name}");
        if !b(&c, "ok") {
            continue;
        }
        assert_eq!(s(&j, "cid"), s(&c, "cid"));
        // every net the check names is a deck's or a section's here
        let mut cids: Vec<String> = Vec::new();
        for (_, d) in get(&j, "decks").as_object().unwrap().iter() {
            cids.push(s(d, "cid").to_string());
        }
        for seg in get(&j, "segments").as_array().unwrap() {
            for (deck, d) in get(seg, "decks").as_object().unwrap().iter() {
                let key = format!("{}@{}:{}", s(seg, "name"), n(seg, "start"), deck);
                assert_eq!(s(&c, &format!("nets.{key}")), s(d, "cid"), "{name} {key}");
            }
        }
        if !b(&c, "arranged") {
            for (deck, d) in get(&j, "decks").as_object().unwrap().iter() {
                assert_eq!(s(&c, &format!("nets.{deck}")), s(d, "cid"), "{name}");
            }
            assert!(!cids.is_empty() || get(&j, "decks").as_object().unwrap().is_empty());
        }
    }
    let j = parse(&beatscript_api::nets_json("deck A: kick x ~ x ~\n", true));
    assert!(get(&j, "decks.A.envelope.nets.kick.places").as_object().is_some_and(|p| p.len() == 16), "{}", j.js_stringified().chars().take(300).collect::<String>());
}

#[test]
fn check_reports_the_flow_of_named_arrangements_and_only_then() {
    let sections = "tempo 120\nsection intro 1 bars { deck A: kick x ~ ~ ~ }\nsection drop 2 bars { deck A: kick x x x x }\n";
    let r = check(&format!("{sections}arrange open: intro then set\narrange set: drop loop\n"));
    assert_eq!(get(&r, "loops").as_bool(), Some(true));
    assert_eq!(get(&r, "song_bars").as_f64(), Some(3.0));
    let flow = get(&r, "flow");
    assert_eq!(get(flow, "loop_from_bar").as_f64(), Some(1.0));
    assert_eq!(get(flow, "finite").as_bool(), Some(false));
    assert_eq!(get(flow, "order").as_array().unwrap().len(), 2);
    // a finite one has a length, and a plain arrange has no `flow` key at all
    let r = check(&format!("{sections}arrange open: intro then set\narrange set: drop loop 2\n"));
    assert_eq!(get(&r, "song_bars").as_f64(), Some(5.0));
    assert_eq!(get(&r, "seconds").as_f64(), Some(10.0));
    assert_eq!(get(get(&r, "flow"), "finite").as_bool(), Some(true));
    let r = check(&format!("{sections}arrange intro drop\n"));
    assert!(r.as_object().unwrap().get("flow").is_none());
}

#[test]
fn check_reports_a_named_arrangement_with_mutations_as_both_a_flow_and_an_arrange() {
    let r = check(include_str!("../../../examples/beatscript/named-arrange-mutations.beatscript"));
    assert_eq!(get(&r, "ok").as_bool(), Some(true));
    assert_eq!(get(&r, "song_bars").as_f64(), Some(16.0));
    let flow = get(&r, "flow");
    assert_eq!(get(flow, "finite").as_bool(), Some(true));
    let order: Vec<_> = get(flow, "order").as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    assert_eq!(order, ["open", "set", "set", "close"]);
    let arrange = get(&r, "arrange");
    assert_eq!(get(arrange, "arrangement").as_str(), Some("set"));
    // the block's two plays, placed where the flow first plays `set`: bars 4 and 6 (after open's intro and drop)
    let plays = get(arrange, "plays").as_array().unwrap();
    let at: Vec<(String, f64)> = plays.iter().map(|p| (get(p, "name").as_str().unwrap().to_string(), get(p, "start_bar").as_f64().unwrap())).collect();
    assert_eq!(at, [("drop".to_string(), 4.0), ("drop@2".to_string(), 6.0)]);
    assert_eq!(get(arrange, "patterns.acid").as_array().unwrap().len(), 2);
}

/// `expand` (fmt --expand): a seed written out is a program of its own, with its own CID; the one it came from is
/// `from_cid`, and with lineage the text says so in a `#! forked-from:` header. A program with no seed is as it was.
#[test]
fn expand_writes_out_seeds_and_says_where_they_came_from() {
    let src = "tempo 128\ndeck A: kick x x x x | bass seed(techno, 42)[17..20]\n";
    let r = parse(&beatscript_api::expand_json(src, true));
    assert_eq!(r.get("ok").and_then(JsonValue::as_bool), Some(true), "{}", r.js_stringified());
    let text = r.get("text").and_then(JsonValue::as_str).unwrap();
    let from = check(src).get("cid").and_then(JsonValue::as_str).unwrap().to_string();
    assert_eq!(r.get("from_cid").and_then(JsonValue::as_str), Some(from.as_str()));
    assert!(text.starts_with(&format!("#! forked-from: {from}\n")), "{text}");
    assert!(text.contains("# from seed(techno, 42)") || text.contains("bass <"), "{text}");
    assert_eq!(check(text).get("cid").and_then(JsonValue::as_str), r.get("cid").and_then(JsonValue::as_str));
    assert_ne!(r.get("cid").and_then(JsonValue::as_str), Some(from.as_str()));
    // nothing to write out
    let plain = parse(&beatscript_api::expand_json("deck A: kick x x x x\n", true));
    assert_eq!(plain.get("text").and_then(JsonValue::as_str), Some("deck A: kick x x x x\n"));
    // an error is check's
    let bad = parse(&beatscript_api::expand_json("arrange from seed(tecno, 42)\n", false));
    assert_eq!(bad.get("ok").and_then(JsonValue::as_bool), Some(false));
}
