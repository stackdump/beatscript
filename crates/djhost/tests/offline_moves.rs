//! The DJ moves in the desktop's offline render (`djhost::beatscript_render::render`, with `djhost::offline_moves`): a `move` line
//! the control net leaves beside it is armed on the booth's scheduler and serviced on the render's clock. The pins were taken
//! before the change: a program with no move, and one whose `filter_exit` the control net plays, render byte for byte as
//! they did.

use djhost::beatscript_render;

const SR: f64 = 22_050.0;

fn compile(src: &str) -> djlive::compile::Compiled {
    let c = djlive::check(src);
    assert!(c.compiled.is_some(), "{src}: {:?}", c.diagnostics);
    c.compiled.unwrap()
}

fn hash(a: &djengine::session::wav::RenderedAudio) -> String {
    let mut b = Vec::with_capacity(a.left.len() * 8);
    for (l, r) in a.left.iter().zip(&a.right) {
        b.extend_from_slice(&l.to_le_bytes());
        b.extend_from_slice(&r.to_le_bytes());
    }
    djengine::cid::sha256(&b).iter().map(|x| format!("{x:02x}")).collect()
}

fn example(name: &str) -> String {
    std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/beatscript").join(name)).unwrap()
}

/// `examples/beatscript/mix.beatscript` (no move), rendered with its control net at 22.05 kHz.
const PIN_NO_MOVE: &str = "8fe55bde26ded6b86c0ebce47b9e75ce0a8ffbedea896395602a49e997b79176";
/// The program of `nets_render.rs`'s move test: the control net plays its filter exit, before the change and after.
const PIN_NET_FILTER_EXIT: &str = "f7951d198e78ea1cfee00b29854383d4405362c0168eaee7c0bf7993e5b7da6f";
const NET_FILTER_EXIT: &str = "tempo 120\ndeck A: hihat x*16 | bass c2 c2 c2 c2 | kick x ~ x ~\nat bar 2: move filter_exit on A over 2 bars\n";

#[test]
fn pin_a_program_with_no_move_renders_as_it_did() {
    let r = beatscript_render::render(&compile(&example("mix.beatscript")), SR, true).expect("render");
    assert_eq!(hash(&r.audio), PIN_NO_MOVE);
}

#[test]
fn pin_a_filter_exit_the_net_plays_renders_as_it_did() {
    let r = beatscript_render::render(&compile(NET_FILTER_EXIT), SR, true).expect("render");
    assert_eq!(hash(&r.audio), PIN_NET_FILTER_EXIT);
    assert!(r.beside.is_empty(), "{:?}", r.beside);
}

/// The roll plays on the scheduler (the net leaves it beside): the render is the same twice, differs from the program without
/// it only from the bar it lands on, and nothing says it is not heard.
#[test]
fn a_loop_roll_is_heard_from_the_bar_after_its_line() {
    let src = "tempo 120\ndeck A: melody c4 e4 g4 c5 len=4\nat bar 2: move loop_roll on A over 1 bars\nend at bar 8\n";
    let c = compile(src);
    let (a, b) = (beatscript_render::render(&c, SR, true).unwrap(), beatscript_render::render(&c, SR, true).unwrap());
    assert_eq!(hash(&a.audio), hash(&b.audio), "the same text is the same audio");
    assert!(a.beside.is_empty(), "the roll is heard: {:?}", a.beside);
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    let plain = beatscript_render::render(&compile(&src.replace("at bar 2: move loop_roll on A over 1 bars\n", "")), SR, true).unwrap();
    let bar = |k: f64| (k * 2.0 * SR).round() as usize;
    // armed on bar 2, it lands on bar 3
    assert!(a.audio.left[..bar(3.0) - 64] == plain.audio.left[..bar(3.0) - 64], "nothing moves before the bar it lands on");
    assert!(a.audio.left[bar(3.0)..bar(4.0)] != plain.audio.left[bar(3.0)..bar(4.0)], "the roll's first bar");
    // without the control lines, no move either
    let off = beatscript_render::render(&c, SR, false).unwrap();
    assert_eq!(hash(&off.audio), hash(&beatscript_render::render(&compile(&src.replace("at bar 2: move loop_roll on A over 1 bars\n", "")), SR, false).unwrap().audio));
}

/// A move that cannot act offline is beside the render, with why.
#[test]
fn a_vocoder_handoff_is_beside_with_why() {
    let r = beatscript_render::render(&compile("tempo 120\ndeck A: kick x ~ x ~\nmove vocoder_handoff on A\nend at bar 2\n"), SR, true).unwrap();
    assert_eq!(r.beside.len(), 1, "{:?}", r.beside);
    assert!(r.beside[0].1.contains("no vocoder"), "{:?}", r.beside);
}
