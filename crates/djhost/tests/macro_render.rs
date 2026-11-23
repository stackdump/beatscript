//! A `macro` line in the desktop's offline render (`djctl btw render`, `djhost::beatscript_render::render`): the control net's `effects
//! fire` played on the engine through `fx_view::fire`, deterministic, heard from the bar its line names (docs/beatscript/ext-control.md).

use djhost::beatscript_render::render;

const SR: f64 = 22_050.0;
const PLAIN: &str = "tempo 120\ndeck A: kick x ~ x ~ x ~ x ~ | bass c2 ~ c2 ~ c2 ~ c2 ~ | hihat x x x x x x x x\nend at bar 8\n";

fn rendered(src: &str) -> djhost::beatscript_render::Render {
    let c = djlive::check(src).compiled.unwrap_or_else(|| panic!("does not compile: {src}"));
    render(&c, SR, true).unwrap()
}

/// The first frame where two renders differ, in bars (120 BPM: two seconds a bar).
fn first_diff_bars(a: &[f32], b: &[f32]) -> Option<f64> {
    a.iter().zip(b).position(|(x, y)| (x - y).abs() > 1e-6).map(|i| i as f64 / (2.0 * SR))
}

#[test]
fn a_macro_renders_the_same_samples_every_time_and_from_its_bar() {
    let with = PLAIN.replace("end at bar 8\n", "at bar 3: macro wash on A over 2 bars\nend at bar 8\n");
    let (a, b, p) = (rendered(&with), rendered(&with), rendered(PLAIN));
    assert!(a.audio.left == b.audio.left && a.audio.right == b.audio.right, "two renders differ");
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    assert!(a.beside.is_empty(), "{:?}", a.beside);
    let at = first_diff_bars(&a.audio.left, &p.audio.left).expect("the macro is heard");
    assert!((3.0..3.25).contains(&at), "the wash starts on bar 3, not {at:.3}");
}

/// The riser's length is its sweep (`over 8 bars`), a held one-shot is let go, and a song macro on a deck with no net is the engine's
/// refusal, counted with its beat.
#[test]
fn a_riser_a_held_tape_stop_and_a_refused_breakdown() {
    let p = rendered(PLAIN);
    let riser = rendered(&PLAIN.replace("end at bar 8\n", "at bar 2: macro riser on A over 4 bars\nend at bar 8\n"));
    assert!((2.0..2.25).contains(&first_diff_bars(&riser.audio.left, &p.audio.left).unwrap()));
    let stop = rendered(&PLAIN.replace("end at bar 8\n", "at bar 4: macro tape_stop on A over 1 bar\nend at bar 8\n"));
    assert!(stop.refused.is_empty(), "{:?}", stop.refused);
    assert!((4.0..4.25).contains(&first_diff_bars(&stop.audio.left, &p.audio.left).unwrap()));
    let bd = rendered(&PLAIN.replace("end at bar 8\n", "at bar 2: macro breakdown on B over 2 bars\nend at bar 8\n"));
    assert!(bd.refused.iter().any(|(_, why)| why.contains("net deck")), "{:?}", bd.refused);
    assert!(bd.audio.left == p.audio.left, "a refused macro changes nothing");
}

/// A macro with no time word, at the program's first bar: there is no beat before it to make the call in, so the engine starts it on
/// its next bar line (docs/beatscript/ext-control.md, Decisions).
#[test]
fn a_macro_at_the_first_bar_starts_where_the_engine_can() {
    let p = rendered(PLAIN);
    let now = rendered(&PLAIN.replace("end at bar 8\n", "macro wash on A over 2 bars\nend at bar 8\n"));
    let at = first_diff_bars(&now.audio.left, &p.audio.left).expect("heard");
    assert!((1.0..1.25).contains(&at), "a macro with no time word starts on bar 1 (the engine's next bar line), not {at:.3}");
}
