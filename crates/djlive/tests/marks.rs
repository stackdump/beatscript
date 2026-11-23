//! Seeing it play (docs/BEATSCRIPT.md): spans through expansion, the hits that fire on a fake clock, the marks.
//! (BeatScript on a fake host and a fake clock: an arranged song lands each section's nets on its bar line,
//! `for N bars` and moves put things back, per-role sounds are set once a net has landed.
//! (The hot reload on a fake host and a fake clock: a change lands on the bar and not before, an
//! error keeps the last good program, what did not change is not touched, `every` and ramps keep
//! their time through an edit.)

use djengine::json::JsonValue;
use djlive::runtime::{Grid, Live, LiveHost};

/// Four beats a bar. The fake deck plays when told, and "installs" a net on the next bar line like
/// `Engine::load_prepared`.
#[derive(Default)]
struct Fake {
    beat: f64,
    playing: [bool; 2],
    /// (deck, net cid, the bar it lands on)
    loads: Vec<(String, String, i64)>,
    /// (beat when called, noun, params)
    calls: Vec<(f64, String, String)>,
    fail_load: bool,
    pauses: Vec<String>,
    /// (beat when called, deck, role, option)
    sounds: Vec<(f64, String, String, String)>,
}

fn ix(d: &str) -> usize {
    usize::from(d == "B")
}

impl LiveHost for Fake {
    fn grid(&self) -> Option<Grid> {
        let d = if self.playing[0] { "A" } else if self.playing[1] { "B" } else { return None };
        Some(Grid { deck: d.into(), beat: self.beat, bpm: 120.0 })
    }

    fn playing(&self, deck: &str) -> bool {
        self.playing[ix(deck)]
    }

    fn load_net(&mut self, deck: &str, env: &JsonValue) -> Result<(), String> {
        if self.fail_load {
            return Err("no such luck".into());
        }
        let land = if self.playing[ix(deck)] { (self.beat / 4.0).floor() as i64 + 1 } else { (self.beat / 4.0).floor() as i64 };
        self.loads.push((deck.into(), djengine::cid::share_cid(env), land));
        Ok(())
    }

    fn set_playing(&mut self, deck: &str, on: bool) {
        self.playing[ix(deck)] = on;
        if !on {
            self.pauses.push(deck.into());
        }
    }

    fn set_sound(&mut self, deck: &str, role: &str, option: &str) -> Result<(), String> {
        self.sounds.push((self.beat, deck.into(), role.into(), option.into()));
        Ok(())
    }

    fn call(&mut self, noun: &str, params: &JsonValue) -> Result<JsonValue, String> {
        self.calls.push((self.beat, noun.into(), params.canonical_js()));
        Ok(JsonValue::Null)
    }
}

impl Fake {
    /// Advance `beats` in sixteenth-bar steps, servicing each.
    fn run(&mut self, live: &mut Live, beats: f64) {
        let steps = (beats * 4.0).round() as usize;
        for _ in 0..steps {
            self.beat += 0.25;
            live.service(self);
        }
    }

    fn calls_of(&self, noun: &str) -> Vec<(f64, String)> {
        self.calls.iter().filter(|c| c.1 == noun).map(|c| (c.0, c.2.clone())).collect()
    }
}


fn playing() -> (Fake, Live) {
    let mut h = Fake::default();
    h.playing[0] = true;
    h.beat = 8.0;
    (h, Live::new())
}

fn bar_of(beat: f64) -> i64 {
    (beat / 4.0).floor() as i64
}

#[test]
fn a_move_from_a_program_puts_its_strip_back_at_the_next_phrase() {
    let (mut h, mut live) = playing();
    assert!(live.eval("every 16 bars: move filter_exit on B over 4 bars\n").ok);
    h.run(&mut live, 4.0 * 45.0);
    let moves = h.calls_of("moves");
    let unmutes: Vec<(i64, String)> = h
        .calls_of("mixer")
        .into_iter()
        .map(|(b, p)| (bar_of(b), p))
        .collect();
    assert!(moves.len() >= 2, "{moves:?}");
    // the first move fires on program bar 16, ends on 20, and the strip is back on the phrase line 24
    let first = bar_of(moves[0].0);
    assert!(unmutes.iter().all(|(_, p)| p.contains("\"mute\":false") && p.contains("\"deck\":\"B\"")), "{unmutes:?}");
    let back = unmutes[0].0;
    assert!(back > first && back - first <= 8, "move on {first}, strip back on {back}");
    // never past the next phrase after the move ends: and again for the next one
    assert!(unmutes.len() >= 2, "{unmutes:?}");
}

#[test]
fn stopping_a_program_puts_back_what_is_still_held() {
    let (mut h, mut live) = playing();
    assert!(live.eval("at bar 2: move echo_out on B over 4 bars\n").ok);
    h.run(&mut live, 4.0 * 4.0);
    assert_eq!(h.calls_of("moves").len(), 1);
    assert!(h.calls_of("mixer").is_empty());
    live.stop_with(&mut h);
    assert_eq!(h.calls_of("mixer").len(), 1, "the strip is put back when the program stops");
}

#[test]
fn for_n_bars_undoes_it_on_the_bar() {
    let (mut h, mut live) = playing();
    assert!(live.eval("at bar 4 for 2 bars: master: filter lp 800\n").ok);
    h.run(&mut live, 4.0 * 10.0);
    let ons: Vec<(i64, String)> = h.calls_of("inserts").into_iter().filter(|(_, p)| p.contains("\"action\":\"on\"")).map(|(b, p)| (bar_of(b), p)).collect();
    assert_eq!(ons.len(), 2, "{ons:?}");
    assert!(ons[0].1.contains("\"on\":true") && ons[1].1.contains("\"on\":false"), "{ons:?}");
    assert_eq!(ons[1].0 - ons[0].0, 2);
}


use djlive::compile::check;
use djlive::diag::Span;

fn texts<'a>(src: &'a str, chain: &[Span]) -> Vec<&'a str> {
    chain.iter().map(|s| s.text(src)).collect()
}

fn first_bass_hit_chain(src: &str) -> Vec<String> {
    let c = check(src).compiled.unwrap();
    let ir = &c.decks[&djlive::ast::Deck::A];
    let part = ir.parts.iter().find(|p| p.role == "bass").unwrap();
    let hit = part.cycles.iter().flatten().next().unwrap();
    texts(src, &part.spans[hit.sid as usize]).iter().map(|s| s.to_string()).collect()
}

#[test]
fn a_hit_knows_every_place_in_the_text_it_is_in_through_let_def_and_transform() {
    let src = "let riff = c2 e2\ndef groove(deck) {\n  deck $deck: bass transpose(riff, 5) | kick x ~ x ~\n}\ngroove(A)\n";
    let c = check(src).compiled.unwrap();
    let ir = &c.decks[&djlive::ast::Deck::A];
    // the bass: c2 + 5 = 41
    let part = ir.parts.iter().find(|p| p.role == "bass").unwrap();
    let hit = part.cycles.iter().flatten().next().unwrap();
    assert_eq!(hit.note, 41);
    let chain = texts(src, &part.spans[hit.sid as usize]);
    assert_eq!(chain[0], "c2", "the token in the let: {chain:?}");
    assert!(chain.contains(&"transpose(riff, 5)"), "the transform that moved it: {chain:?}");
    assert!(chain.iter().any(|t| t.starts_with("bass transpose(riff, 5)")), "the part: {chain:?}");
    assert!(chain.iter().any(|t| t.starts_with("deck $deck: bass")), "the line in the def body: {chain:?}");
    assert_eq!(*chain.last().unwrap(), "groove(A)", "and the call site: {chain:?}");
    // the kick's x
    let kick = ir.parts.iter().find(|p| p.role == "kick").unwrap();
    let k = kick.cycles.iter().flatten().next().unwrap();
    let kc = texts(src, &kick.spans[k.sid as usize]);
    assert_eq!(kc[0], "x");
    assert_eq!(*kc.last().unwrap(), "groove(A)");
    // none of it is in what a program is: the same music written another way has the same hits and CIDs
    let other = "let   riff = c2   e2 # a\ndef groove(deck) {\n deck $deck:   bass transpose(riff, 5)|kick x ~ x ~\n}\ngroove(A)\n";
    let o = check(other).compiled.unwrap();
    assert_eq!(o.cid, c.cid);
    assert_eq!(o.decks[&djlive::ast::Deck::A].parts[0].cycles, ir.parts[0].cycles, "hits are equal whatever text wrote them");
}

#[test]
fn a_let_used_twice_lights_its_own_use_and_its_definition() {
    let src = "let riff = c2 e2\ndeck A: bass riff riff\n";
    let chain = first_bass_hit_chain(src);
    assert_eq!(chain[0], "c2");
    assert!(chain.iter().any(|t| t == "riff"), "the use site: {chain:?}");
    let c = check(src).compiled.unwrap();
    let part = &c.decks[&djlive::ast::Deck::A].parts[0];
    // the second riff's hits are the same let tokens, with the other use site
    let uses: Vec<usize> = part.spans.iter().filter_map(|ch| ch.iter().find(|s| s.text(src) == "riff").map(|s| s.start)).collect();
    assert_eq!(uses.iter().collect::<std::collections::BTreeSet<_>>().len(), 2, "two use sites: {uses:?}");
}

#[test]
fn a_repeat_and_a_call_are_in_the_via_of_what_they_expand() {
    let src = "def dd(x) {\n  repeat 2 every 1 bars {\n    master: delay 0.3\n  }\n}\ndd(1)\n";
    let c = check(src).compiled.unwrap();
    assert_eq!(c.controls.len(), 2);
    let v: Vec<&str> = texts(src, &c.controls[0].via);
    assert!(v[0].starts_with("repeat 2"), "{v:?}");
    assert_eq!(*v.last().unwrap(), "dd(1)");
    assert_eq!(c.controls[0].span.text(src), "master: delay 0.3");
}

#[test]
fn an_evolved_hit_keeps_its_token() {
    let src = "section drop 8 bars {\n  deck A: bass [a2 e2]\n  evolve every 4 bars {\n    bass: rotate 1 | transpose 3\n  }\n}\narrange drop\n";
    let c = check(src).compiled.unwrap();
    let g1 = &c.segments[1].decks[&djlive::ast::Deck::A].parts[0];
    let h = g1.cycles.iter().flatten().next().unwrap();
    assert_eq!(h.note, 45 + 3, "a2 up three");
    assert_eq!(texts(src, &g1.spans[h.sid as usize])[0], "a2", "the generation's hit is still the token a2");
    // chain segments (a net cut in stretches) keep them too
}

#[test]
fn the_hits_fire_on_the_clock_with_their_spans() {
    let (mut h, mut live) = playing();
    let src = "deck A: kick x ~ x ~ | snare ~ x ~ x\n";
    assert!(live.eval(src).ok);
    let mut seen: Vec<(String, f64, String)> = Vec::new();
    for _ in 0..(4 * 4 * 4) {
        h.run(&mut live, 0.25);
        for f in live.fires() {
            let key = (f.role.clone(), f.at, f.spans[0].text(src).to_string());
            if !seen.contains(&key) {
                seen.push(key);
            }
        }
    }
    // four bars: a kick on beats 1 and 3, a snare on 2 and 4: eight a bar after the net has landed
    let kicks: Vec<f64> = seen.iter().filter(|s| s.0 == "kick").map(|s| s.1).collect();
    let snares: Vec<f64> = seen.iter().filter(|s| s.0 == "snare").map(|s| s.1).collect();
    assert!(kicks.len() >= 6 && snares.len() >= 6, "{} kicks {} snares", kicks.len(), snares.len());
    // on the eighth: kick steps 0 and 8 of 16, snare 4 and 12
    assert!(kicks.iter().all(|t| (t.fract() * 16.0).round() as i64 % 8 == 0), "{kicks:?}");
    assert!(snares.iter().all(|t| ((t.fract() * 16.0).round() as i64 - 4) % 8 == 0), "{snares:?}");
    assert!(seen.iter().all(|s| s.2 == "x"), "the token that fired is the `x` of its pattern");
    // and the spans say which x: the kick's are on the kick part of the line
    let first_kick = live.fires().into_iter().find(|f| f.role == "kick").unwrap();
    assert!(first_kick.spans.iter().any(|s| s.text(src).starts_with("kick x ~ x ~")), "{:?}", first_kick.spans.iter().map(|s| s.text(src)).collect::<Vec<_>>());
    // nothing fires when the clock does not move, and a jump is not a firing of the whole bar
    let before = live.fires().len();
    live.service(&mut h);
    assert_eq!(live.fires().len(), before);
}

#[test]
fn the_marks_say_what_is_playing_where_the_playhead_is_and_how_far_a_ramp_has_got() {
    let (mut h, mut live) = playing();
    let src = "section drop 8 bars {\n  deck A: kick x ~ x ~\n  master: filter lp 300 -> 2800 over 4 bars\n  at bar 2: master: delay 0.3\n}\narrange drop\n";
    assert!(live.eval(src).ok);
    h.run(&mut live, 4.0 * 3.0 + 1.0);
    let m = live.marks();
    assert!(m.running);
    // the header: the section statement, with the playhead on it
    let (hs, p1) = m.header.clone().expect("a section header");
    assert!(hs.text(&m.source).starts_with("section drop 8 bars"));
    // the lines that are playing: the section's, including the kick line
    assert!(m.active_lines.iter().any(|s| s.text(&m.source).starts_with("deck A: kick")));
    // the ramp is moving, and so is the playhead
    let r1 = m.controls.iter().find(|c| c.key.contains("filter lp")).and_then(|c| c.ramp).expect("a ramp");
    assert!(r1 > 0.0 && r1 < 1.0, "{r1}");
    // the timed line fired on bar 2 of the section: it is glowing
    let at = m.controls.iter().find(|c| c.key.contains("delay")).expect("it fired");
    assert!(at.age_beats < 8.0, "{}", at.age_beats);
    h.run(&mut live, 4.0);
    let m2 = live.marks();
    let (_, p2) = m2.header.clone().unwrap();
    let r2 = m2.controls.iter().find(|c| c.key.contains("filter lp")).and_then(|c| c.ramp).expect("a ramp");
    assert!(p2 > p1 + 0.1, "the playhead moved: {p1} -> {p2}");
    assert!((r2 - r1 - 0.25).abs() < 0.03, "a bar of a four-bar ramp is a quarter: {r1} -> {r2}");
    let at2 = m2.controls.iter().find(|c| c.key.contains("delay")).unwrap();
    assert!(at2.age_beats > at.age_beats + 3.0, "it has aged a bar: {} -> {}", at.age_beats, at2.age_beats);
    let m = m2;
    // as JSON, for a reader that is not the window
    let j = m.to_json();
    assert_eq!(j.get("running").and_then(|v| v.as_bool()), Some(true));
    let fires = j.get("fires").and_then(|v| v.as_array()).unwrap();
    assert!(!fires.is_empty());
    let first = &fires[0].get("spans").and_then(|v| v.as_array()).unwrap()[0];
    assert_eq!(first.get("line").and_then(|v| v.as_f64()), Some(2.0));
    assert_eq!(first.get("len").and_then(|v| v.as_f64()), Some(1.0));
}

#[test]
fn an_evolving_section_flashes_its_rules_when_a_generation_begins() {
    let (mut h, mut live) = playing();
    let src = "section drop 12 bars {\n  deck A: bass [a2 e2 c2 g2]\n  evolve every 4 bars {\n    bass: transpose 2\n    stop when gen == 2\n  }\n}\narrange drop\n";
    assert!(live.eval(src).ok);
    h.run(&mut live, 4.0 * 2.0);
    assert_eq!(live.marks().generation, Some(0));
    assert!(live.marks().gen_flash_age.is_none(), "no generation has begun but the first");
    // run to a little after generation 1 begins
    let mut flashed = None;
    for _ in 0..(4 * 4 * 3) {
        h.run(&mut live, 0.25);
        let m = live.marks();
        if m.generation == Some(1) && m.gen_flash_age.is_some() && flashed.is_none() {
            flashed = Some(m);
        }
    }
    let m = flashed.expect("generation 1 flashed");
    assert!(m.gen_flash_age.unwrap() < 2.0);
    assert!(m.evolve_spans.iter().any(|s| s.text(&m.source).starts_with("evolve every 4 bars")));
    assert!(m.evolve_spans.iter().any(|s| s.text(&m.source) == "bass: transpose 2"));
}
