//! `beatscript/numberAt`: the number, note or numeral at a position, its limits, step and unit; the limits proved against the
//! checker; a scrub that edits only its literal keeps the program checkable and canonical.

mod common;

use beatscriptls::number::{self, number_at, LIMITS};
use beatscriptls::Analysis;
use common::*;
use djengine::json::JsonValue;

fn reg() -> &'static dyn djlive::env::Registry {
    &beatscript_api::web_env().registry
}

/// The number at the `nth` occurrence of `needle` in `text`, `skip` bytes into it.
fn at_needle(text: &str, needle: &str, nth: usize, skip: usize) -> Option<number::NumberAt> {
    let start = text.match_indices(needle).nth(nth).unwrap_or_else(|| panic!("no {needle:?} #{nth}")).0 + skip;
    number_at(&Analysis::new(text), start, reg())
}

fn kind(text: &str, needle: &str, skip: usize) -> &'static str {
    at_needle(text, needle, 0, skip).map_or("none", |n| n.limit.kind)
}

fn ok(text: &str) -> bool {
    beatscriptls::analysis::full_check(text).ok()
}

const PROGRAM: &str = "\
tempo 124
swing 0.1
key am
deck A: kick e(5, 8, 2) vel=90..110 | hihat x*8?0.7 len=2 | snare ~ x ~ x
deck B: bass 1 ~ 3 5 oct=2 | melody c4 e4 g4 ~ | harmony i VI III VII
at bar 16: master: filter lp 2000 -> 8000 over 8 bars
every 16 bars for 4 bars: deck B: filter hp 400
master: delay 0.3
deck A: level 0.8
crossfader 0.5
section drop 16 bars {
  deck A: kick x ~ x ~
}
repeat 4 {
  deck B: clap x when $i == 2
}
";

#[test]
fn each_kind_of_number_says_what_it_is() {
    assert!(ok(PROGRAM), "{:?}", beatscriptls::analysis::full_check(PROGRAM).diagnostics);
    for (needle, skip, want) in [
        ("tempo 124", 6, "tempo"),
        ("swing 0.1", 6, "swing"),
        ("e(5", 2, "hits"),
        ("8, 2)", 0, "steps"),
        ("2) vel", 0, "rotation"),
        ("vel=90", 4, "vel"),
        ("..110", 2, "vel"),
        ("x*8", 2, "times"),
        ("?0.7", 1, "chance"),
        ("len=2", 4, "len"),
        ("bass 1", 5, "degree"),
        ("oct=2", 4, "oct"),
        ("bar 16", 4, "bar"),
        ("lp 2000", 3, "cutoff-lp"),
        ("-> 8000", 3, "cutoff-lp"),
        ("over 8 bars", 5, "bars"),
        ("every 16", 6, "bars"),
        ("for 4", 4, "bars"),
        ("hp 400", 3, "cutoff-hp"),
        ("delay 0.3", 6, "mix"),
        ("level 0.8", 6, "level"),
        ("crossfader 0.5", 11, "crossfader"),
        ("drop 16", 5, "section-bars"),
        ("repeat 4", 7, "repeat"),
        ("c4 e4", 0, "note"),
        ("VI III", 0, "chord"),
    ] {
        assert_eq!(kind(PROGRAM, needle, skip), want, "{needle:?}+{skip}");
    }
    // a word that is not a number, note or numeral; a comment
    assert!(at_needle(PROGRAM, "kick e", 0, 1).is_none());
    assert!(at_needle("# tempo 120\ntempo 120\n", "120", 0, 0).is_none());
}

#[test]
fn the_request_answers_span_limits_step_and_unit() {
    let mut s = server(PROGRAM);
    let (l, c) = find(PROGRAM, "tempo 124", 0, 6);
    let r = req(&mut s, "beatscript/numberAt", &at(l, c));
    assert_eq!(r.get("kind").and_then(JsonValue::as_str), Some("tempo"));
    assert_eq!(r.get("value").and_then(JsonValue::as_f64), Some(124.0));
    assert_eq!(r.get("min").and_then(JsonValue::as_f64), Some(40.0));
    assert_eq!(r.get("max").and_then(JsonValue::as_f64), Some(300.0));
    assert_eq!(r.get("unit").and_then(JsonValue::as_str), Some("bpm"));
    assert_eq!(
        pretty(r.get("range").unwrap()),
        pretty(&JsonValue::parse(r#"{"start":{"line":0,"character":6},"end":{"line":0,"character":9}}"#).unwrap())
    );
    let (l, c) = find(PROGRAM, "lp 2000", 0, 3);
    let r = req(&mut s, "beatscript/numberAt", &at(l, c));
    assert_eq!(r.get("scale").and_then(JsonValue::as_str), Some("log"));
    let span = r.get("span").expect("a ramp's end says the other end");
    assert_eq!(span.get("op").and_then(JsonValue::as_str), Some("->"));
    assert_eq!(span.get("role").and_then(JsonValue::as_str), Some("start"));
    assert_eq!(span.get("endValue").and_then(JsonValue::as_f64), Some(8000.0));
    assert_eq!(span.get("ordered"), Some(&JsonValue::Bool(false)));
    // nothing there: null
    let (l, c) = find(PROGRAM, "deck A", 0, 0);
    assert_eq!(req(&mut s, "beatscript/numberAt", &at(l, c)), JsonValue::Null);
    // advertised
    let caps = s.capabilities();
    assert_eq!(
        caps.get("experimental").and_then(|e| e.get("beatscript")).and_then(|b| b.get("numberAt")),
        Some(&JsonValue::Bool(true))
    );
}

#[test]
fn a_range_says_both_ends_in_order() {
    let n = at_needle(PROGRAM, "vel=90", 0, 4).unwrap();
    let r = n.range.expect("vel=90..110 is a range");
    assert_eq!((r.op, r.role, r.start_value, r.end_value), ("..", "start", 90.0, 110.0));
    let m = at_needle(PROGRAM, "..110", 0, 2).unwrap();
    let r2 = m.range.unwrap();
    assert_eq!((r2.role, r2.start, r2.end), ("end", r.start, r.end));
    assert_eq!(m.limit.kind, "vel", "the far end has the near end's limits");
    // a negative end keeps its sign in the span
    let t = "deck B: bass 1 oct=-1..2\n";
    let n = at_needle(t, "-1", 0, 1).unwrap();
    assert_eq!((n.value, n.text.as_str()), (-1.0, "-1"));
}

#[test]
fn arithmetic_is_not_exact() {
    let t = "tempo 120 + 6\ndeck A: kick x vel=(60 + 4)..100\n";
    assert!(!at_needle(t, "120", 0, 0).unwrap().exact);
    assert!(!at_needle(t, "6\n", 0, 0).unwrap().exact);
    assert!(!at_needle(t, "60", 0, 0).unwrap().exact);
    assert!(at_needle(t, "100", 0, 0).unwrap().exact);
    assert!(at_needle(PROGRAM, "tempo 124", 0, 6).unwrap().exact);
}

#[test]
fn a_note_moves_through_the_key_or_every_semitone() {
    // in A minor: a b c d e f g, from octave -1 to 9
    let n = at_needle(PROGRAM, "c4 e4", 0, 0).unwrap();
    let i = n.index.unwrap();
    assert_eq!(n.choices[i], "c4");
    assert_eq!((n.choices[i - 1].as_str(), n.choices[i + 1].as_str()), ("b3", "d4"));
    assert!(n.choices.iter().all(|c| !c.contains('#')), "{:?}", n.choices);
    // no key: every semitone, sharps as fmt spells them
    let t = "deck B: melody c4 ~\n";
    let n = at_needle(t, "c4", 0, 0).unwrap();
    let i = n.index.unwrap();
    assert_eq!((n.choices[i - 1].as_str(), n.choices[i + 1].as_str()), ("b3", "c#4"));
    // a note out of the key stays where it is among them
    let t = "key c\ndeck B: melody f#4 ~\n";
    let n = at_needle(t, "f#4", 0, 0).unwrap();
    let i = n.index.unwrap();
    assert_eq!((n.choices[i - 1].as_str(), n.choices[i].as_str(), n.choices[i + 1].as_str()), ("f4", "f#4", "g4"));
}

#[test]
fn a_numeral_moves_through_the_degrees_of_the_key() {
    let n = at_needle(PROGRAM, "VI III", 0, 0).unwrap();
    // A minor: i ii° III iv v VI VII
    assert_eq!(n.index, Some(5));
    assert_eq!(n.choices[0], "i");
    assert_eq!(n.choices[2], "III");
    assert_eq!(n.choices[6], "VII");
    // every choice reads back as a numeral in the program
    for c in &n.choices {
        let t = format!("key am\ndeck B: harmony {c} ~\n");
        assert!(ok(&t), "{c}: {:?}", beatscriptls::analysis::full_check(&t).diagnostics);
    }
}

#[test]
fn a_do_param_takes_its_range_from_the_registry() {
    // the registry's schema bounds some params (heos level 0..100); a param with none is free
    let reg = beatscript_api::web_env();
    let bounded = djlive::env::Registry::noun_names(&reg.registry)
        .into_iter()
        .filter_map(|n| djlive::env::Registry::noun(&reg.registry, &n))
        .flat_map(|n| n.params.into_iter().map(move |p| (n.name.clone(), p)))
        .find(|(_, p)| p.minimum.is_some() && p.maximum.is_some() && p.types.iter().any(|t| t == "number" || t == "integer"));
    let Some((noun, p)) = bounded else {
        panic!("the snapshot carries no minimum/maximum: regenerate it (gen_snapshot)")
    };
    let t = format!("do {noun} {}={}\n", p.name, p.minimum.unwrap());
    let n = at_needle(&t, "=", 0, 1).unwrap_or_else(|| panic!("{t}"));
    assert_eq!((n.limit.min, n.limit.max), (p.minimum, p.maximum), "{t}");
    assert!(n.label.contains(&p.name));
}

/// One template per row of LIMITS: `@` is where the number goes.
fn template(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "tempo" => "tempo @\ndeck A: kick x\n",
        "swing" => "swing @\ndeck A: kick x\n",
        "vel" => "deck A: kick x vel=@\n",
        "oct" => "deck B: bass 1 oct=@\n",
        "len" => "deck B: bass 1 len=@\n",
        "glide" => "deck B: bass 1^ 3 glide=@\n",
        "decay" => "deck B: bass 1 decay=@\n",
        "pitch-drop" => "deck B: bass 1 pitch-drop=@\n",
        "ratchet" => "deck A: kick x:@\n",
        "seed" => "deck A: kick x?0.5 seed=@\n",
        "times" => "deck A: hihat x*@\n",
        "chance" => "deck A: hihat x?@\n",
        "cycle" => "deck A: hihat/@ x x\n",
        "hits" => "deck A: kick e(@, 64)\n",
        "steps" => "deck A: kick e(1, @)\n",
        "density" => "key am\ndeck B: melody markov(am, @)\n",
        "level" => "deck A: kick x\nmaster: level @\n",
        "eq" => "deck A: kick x\ndeck A: low @\n",
        "crossfader" => "deck A: kick x\ncrossfader @\n",
        "mix" => "deck A: kick x\nmaster: delay @\n",
        "resonance" => "deck A: kick x\nmaster: resonance @\n",
        "cutoff-lp" => "deck A: kick x\nmaster: filter lp @\n",
        "cutoff-hp" => "deck A: kick x\nmaster: filter hp @\n",
        "repeat" => "repeat @ {\n  deck A: kick x when $i == 1\n}\n",
        "section-bars" => "section sec @ bars {\n  deck A: kick x\n}\narrange sec\n",
        "beat" => "deck A: kick x\nat bar 4 beat @: master: level 0.5\n",
        "mut-int" => "let p = x ~ x ~\nsection sec 4 bars {\n  deck A: kick p\n}\narrange {\n  sec\n  ~ p rot @\n  sec\n}\n",
        "passes" => "section sec 4 bars {\n  deck A: kick x\n}\narrange {\n  sec\n} loop @\n",
        "share" => "let p = x ~ x ~\nsection sec 4 bars {\n  deck A: kick p\n}\narrange {\n  sec\n  ~ p thin @\n  sec\n}\n",
        _ => return None,
    })
}

#[test]
fn every_limit_is_what_the_checker_accepts() {
    let mut proved = Vec::new();
    for l in LIMITS {
        let Some(t) = template(l.kind) else { continue };
        let with = |v: f64| t.replace('@', &number::spell(v, number::decimals(l.step)));
        let lo = l.min.unwrap();
        assert!(
            ok(&with(lo)),
            "{}: the minimum {lo} is refused: {:?}",
            l.kind,
            beatscriptls::analysis::full_check(&with(lo)).diagnostics
        );
        // the template's literal is classified as this row
        let n = number_at(&Analysis::new(with(lo)), t.find('@').unwrap() + usize::from(lo < 0.0), reg());
        assert_eq!(n.map(|n| n.limit.kind), Some(l.kind), "{}", with(lo));
        let below = lo - l.step;
        assert!(!ok(&with(below)), "{}: one step under the minimum ({below}) checks clean", l.kind);
        if let Some(hi) = l.max {
            assert!(
                ok(&with(hi)),
                "{}: the maximum {hi} is refused: {:?}",
                l.kind,
                beatscriptls::analysis::full_check(&with(hi)).diagnostics
            );
            assert!(!ok(&with(hi + l.step)), "{}: one step over the maximum ({}) checks clean", l.kind, hi + l.step);
        }
        proved.push(l.kind);
    }
    // the rows with no template are the ones bounded by context (a bar, a length, a degree) or by another number
    assert!(proved.len() >= 20, "{proved:?}");
}

/// Every number of a program moved one step up and one down (clamped) keeps it checkable and canonical, and changes only that
/// literal.
#[test]
fn a_scrub_edits_only_its_literal_and_keeps_the_program_canonical() {
    let canon = |t: &str| beatscriptls::analysis::full_check(t).compiled.map(|c| c.text);
    let base = canon(PROGRAM).expect("checks");
    let text = base.clone();
    let a = Analysis::new(text.clone());
    let mut scrubbed = 0;
    for t in a.tokens.iter() {
        if !matches!(t.tok, djlive::lex::Tok::Num { .. } | djlive::lex::Tok::Word(_)) {
            continue;
        }
        let Some(n) = number_at(&a, t.span.start, reg()) else {
            continue;
        };
        if n.span.start != t.span.start && !(n.span.start + 1 == t.span.start) {
            continue;
        }
        for dir in [-1.0, 1.0] {
            let new = if n.choices.is_empty() {
                let v = n.value + dir * n.step;
                let v = v.max(n.limit.min.unwrap_or(f64::MIN)).min(n.limit.max.unwrap_or(f64::MAX));
                // a range's ends do not cross
                let v = match &n.range {
                    Some(r) if r.op == ".." && r.role == "start" => v.min(r.end_value),
                    Some(r) if r.op == ".." && r.role == "end" => v.max(r.start_value),
                    _ => v,
                };
                number::spell(v, number::decimals(n.step))
            } else {
                let k = (n.index.unwrap() as i64 + dir as i64).clamp(0, n.choices.len() as i64 - 1) as usize;
                n.choices[k].clone()
            };
            let edited = format!("{}{}{}", &text[..n.span.start], new, &text[n.span.end..]);
            let c = canon(&edited).unwrap_or_else(|| panic!("{} -> {new} refused:\n{edited}\n{:?}", n.text, beatscriptls::analysis::full_check(&edited).diagnostics));
            assert_eq!(c, edited, "{} -> {new}: the text is no longer canonical", n.text);
            // only that literal changed
            assert_eq!(&edited[..n.span.start], &text[..n.span.start]);
            assert_eq!(&edited[n.span.start + new.len()..], &text[n.span.end..]);
            scrubbed += 1;
        }
    }
    assert!(scrubbed >= 40, "{scrubbed}");
}

#[test]
fn the_expression_numbers_say_what_they_are() {
    // link 01's tonal params and the ratchet's count; a colon glued to a word that is no step is a spec word's
    let t = "deck A: kick x:3 x&:4 x.:2 | hihat [x x]:2 ~\ndeck B: bass 1^ 3:2 c3!:4 glide=60 decay=300 pitch-drop=12\n";
    assert!(ok(t), "{:?}", beatscriptls::analysis::full_check(t).diagnostics);
    for (needle, skip, want) in [
        ("x:3", 2, "ratchet"),
        ("&:4", 2, "ratchet"),
        (".:2", 2, "ratchet"),
        ("]:2", 2, "ratchet"),
        ("3:2", 2, "ratchet"),
        ("3:2", 0, "degree"),
        ("!:4", 2, "ratchet"),
        ("glide=60", 6, "glide"),
        ("decay=300", 6, "decay"),
        ("pitch-drop=12", 11, "pitch-drop"),
    ] {
        assert_eq!(kind(t, needle, skip), want, "{needle:?}+{skip}");
    }
    assert_ne!(kind("do loops load dnb:2602\n", "2602", 0), "ratchet");
}
