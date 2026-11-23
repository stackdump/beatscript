//! `beatscript render` in-process: the streaming renderer's contract on short programs (a few bars, so the
//! whole file runs in well under a minute). The CLI itself (stdin, exit codes, the first byte before the
//! end) is in `render_cli.rs`.
//!
//! The renderer is included by path: `beatscript` is a binary crate, and the test needs `render_to` with
//! its sinks in its own hands.

#![allow(dead_code)]

#[path = "../src/render.rs"]
mod render;

use std::io::Write;
use std::time::Instant;

use djengine::json::JsonValue;
use render::{Events, Format, Options};

/// Four bars: a kick, then a second section that adds a hat and a deck B bass. 140 bpm: 6.857 s.
const SONG: &str = "tempo 140\nsection one 2 bars {\n  deck A: kick x ~ x ~ x ~ x ~ x ~ x ~ x ~ x ~\n}\nsection two 2 bars {\n  deck A: kick x ~ x ~ x ~ x ~ x ~ x ~ x ~ x ~ | hihat ~ x ~ x ~ x ~ x ~ x ~ x ~ x ~ x\n  deck B: bass c2 ~ ~ c2 ~ ~ c2 ~ ~ c2 ~ ~ c2 ~ ~ c2 ~\n}\narrange one two\n";
/// Three sections, so decks come and go: B only in the middle one.
const THREE: &str = "tempo 128\nsection intro 1 bars {\n  deck A: kick x ~ x ~ x ~ x ~ x ~ x ~ x ~ x ~\n}\nsection build 1 bars {\n  deck A: kick x ~ x ~ x ~ x ~ x ~ x ~ x ~ x ~\n  deck B: bass c2 ~ ~ c2 ~ ~ c2 ~ ~ c2 ~ ~ c2 ~ ~ c2 ~\n}\nsection outro 2 bars {\n  deck A: kick x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~\n}\narrange intro build outro\n";
/// No sections and no end: 16 bars at 150 bpm is 25.6 s, so tests cut it with --max-bars.
const OPEN: &str = "tempo 150\ndeck A: kick x ~ x ~ x ~ x ~ x ~ x ~ x ~ x ~ | hihat ~ x ~ x ~ x ~ x ~ x ~ x ~ x ~ x\n";

struct Run {
    pcm: Vec<u8>,
    /// Sizes of the writes the PCM arrived in.
    writes: Vec<usize>,
    lines: Vec<JsonValue>,
    result: Result<render::Summary, render::RenderError>,
}

struct Recorder<'a>(&'a mut Run2);
#[derive(Default)]
struct Run2 {
    pcm: Vec<u8>,
    writes: Vec<usize>,
}
impl Write for Recorder<'_> {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.pcm.extend_from_slice(b);
        self.0.writes.push(b.len());
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn run(text: &str, o: &Options) -> Run {
    let mut pcm = Run2::default();
    let mut err: Vec<u8> = Vec::new();
    let result = {
        let mut ev = Events::new(&mut err);
        let mut rec = Recorder(&mut pcm);
        let r = render::render_to(text, o, &mut rec, &mut ev, Instant::now());
        if let Err(e) = &r {
            ev.error(e);
        }
        r
    };
    let lines = String::from_utf8(err).unwrap().lines().map(|l| JsonValue::parse(l).unwrap_or_else(|e| panic!("stderr line is not JSON: {l}: {e:?}"))).collect();
    Run { pcm: pcm.pcm, writes: pcm.writes, lines, result }
}

fn opts() -> Options {
    Options::default()
}

fn event<'a>(r: &'a Run, name: &str) -> Vec<&'a JsonValue> {
    r.lines.iter().filter(|l| l.get("event").and_then(JsonValue::as_str) == Some(name)).collect()
}

fn f(v: &JsonValue, k: &str) -> f64 {
    v.get(k).and_then(JsonValue::as_f64).unwrap_or_else(|| panic!("no number {k} in {}", v.js_stringified()))
}

/// The frames of `bars` bars of `bpm` at `sr`, as the renderer counts them.
fn bar_frames(bars: u32, bpm: f64, sr: u32) -> usize {
    (f64::from(bars) * 4.0 * 60.0 / bpm * f64::from(sr)).round() as usize
}

#[test]
fn the_same_text_and_rate_give_the_same_bytes_every_time() {
    let a = run(SONG, &opts());
    let b = run(SONG, &opts());
    assert!(a.result.is_ok() && b.result.is_ok());
    assert_eq!(a.pcm.len(), bar_frames(4, 140.0, 44_100) * 4);
    assert!(a.pcm == b.pcm, "two renders of one program differ");
    // and it is sound: not silence, and not clipped flat
    let peak = a.pcm.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]).unsigned_abs()).max().unwrap();
    assert!(peak > 1000, "the render is nearly silent: peak {peak}");
    // another rate is another render, of the right length
    let o = Options { sr: 22_050, ..opts() };
    let c = run(SONG, &o);
    assert_eq!(c.pcm.len(), bar_frames(4, 140.0, 22_050) * 4);
    assert!(c.pcm != a.pcm);
}

#[test]
fn the_chunk_size_does_not_change_a_byte() {
    let base = run(SONG, &Options { chunk_ms: 250, ..opts() });
    for ms in [1, 64, 1000, 60_000] {
        let r = run(SONG, &Options { chunk_ms: ms, ..opts() });
        assert!(r.pcm == base.pcm, "--chunk-ms {ms} changed the bytes");
    }
    // the chunks are the size asked: every write but the last is a whole number of chunks' worth
    let r = run(SONG, &Options { chunk_ms: 64, ..opts() });
    let chunk = 64 * 44_100 / 1000 * 4;
    let first = 60.min(64) * 44_100 / 1000 * 4;
    assert!(r.writes.len() > 100, "{} writes for 6.8 s at 64 ms", r.writes.len());
    for (i, w) in r.writes.iter().enumerate() {
        let last = i + 1 == r.writes.len();
        // a chunk is cut at the end of the block that fills it: at most one block (128 frames) over
        let (lo, hi) = if i == 0 { (first, first + 128 * 4) } else { (if last { 4 } else { chunk }, chunk + 128 * 4) };
        assert!(*w >= lo && *w <= hi, "write {i} is {w} bytes (want {lo}..={hi})");
    }
    // one shot: the first small chunk, then the rest in one write
    let one = run(SONG, &Options { chunk_ms: 60_000, ..opts() });
    assert_eq!(one.writes.len(), 2);
}

#[test]
fn the_first_chunk_is_small_and_leaves_before_the_song_is_rendered() {
    // the sink sees the first write while the engine has rendered about 60 ms, not the song
    struct First(Option<Instant>, Vec<(f64, usize)>, Instant);
    impl Write for First {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            if self.0.is_none() {
                self.0 = Some(Instant::now());
            }
            self.1.push((self.2.elapsed().as_secs_f64(), b.len()));
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let t0 = Instant::now();
    let mut w = First(None, Vec::new(), t0);
    let mut err = Vec::new();
    render::render_to(SONG, &opts(), &mut w, &mut Events::new(&mut err), t0).unwrap();
    let first = w.1.first().unwrap();
    let last = w.1.last().unwrap();
    assert!(first.1 <= (0.061 * 44_100.0) as usize * 4 + 128 * 4, "the first chunk is {} bytes", first.1);
    assert!(first.0 < last.0, "first write at {:.3}s, last at {:.3}s", first.0, last.0);
    assert!(w.1.len() > 20);
}

#[test]
fn stderr_is_json_lines_start_progress_end() {
    let r = run(SONG, &opts());
    assert!(r.result.is_ok());
    let first = &r.lines[0];
    assert_eq!(first.get("event").and_then(JsonValue::as_str), Some("start"), "start comes first");
    assert_eq!(f(first, "sample_rate"), 44_100.0);
    assert_eq!(f(first, "channels"), 2.0);
    assert_eq!(first.get("format").and_then(JsonValue::as_str), Some("s16le"));
    assert_eq!(f(first, "bars"), 4.0);
    assert_eq!(f(first, "bpm"), 140.0);
    assert!((f(first, "seconds") - 4.0 * 4.0 * 60.0 / 140.0).abs() < 1e-3);
    assert!(first.get("notes").and_then(JsonValue::as_array).is_some());
    assert!(first.get("cid").and_then(JsonValue::as_str).unwrap().starts_with('z'));
    let last = r.lines.last().unwrap();
    assert_eq!(last.get("event").and_then(JsonValue::as_str), Some("end"), "end comes last");
    assert!((f(last, "seconds") - f(first, "seconds")).abs() < 1e-3);
    assert_eq!(f(last, "frames") as usize * 4, r.pcm.len());
    for k in ["check_ms", "ready_ms", "ttfb_ms", "render_ms"] {
        assert!(f(last, k) >= 0.0);
    }
    assert!(f(last, "ttfb_ms") <= f(last, "render_ms"));
    // everything between is progress, rising, and never more than 4 a second of wall time
    let progress = event(&r, "progress");
    let mut prev = 0.0;
    for p in &progress {
        let s = f(p, "seconds_rendered");
        assert!(s > prev && s <= f(first, "seconds") + 1e-3, "progress {s} after {prev}");
        prev = s;
    }
    let wall = f(last, "render_ms") / 1000.0;
    assert!(progress.len() as f64 <= 4.0 * wall + 1.0, "{} progress lines in {wall:.2} s", progress.len());
    assert_eq!(r.lines.len(), 2 + progress.len(), "nothing but start, progress and end");
}

#[test]
fn a_program_with_a_mistake_is_refused_with_its_line_and_column() {
    let r = run("tempo 140\ndeck A: kick x ~ x ~\ndeck B bass c2\n", &opts());
    let e = r.result.as_ref().err().expect("an error");
    assert_eq!(e.code, "check_failed");
    assert!(r.pcm.is_empty());
    assert_eq!(r.lines.len(), 1);
    let line = &r.lines[0];
    assert_eq!(line.get("event").and_then(JsonValue::as_str), Some("error"));
    let m = line.get("message").and_then(JsonValue::as_str).unwrap();
    assert!(m.contains("error") && m.contains("3:"), "{m}");
}

#[test]
fn a_program_that_uses_assets_or_is_restricted_is_refused() {
    let assets = "sample harp from \"/no/such/file.wav\" license cc0 else bright-pluck\ntempo 120\ndeck A: kick x ~ x ~\n";
    let r = run(assets, &opts());
    match &r.result {
        Err(e) => assert_eq!(e.code, "uses_assets", "{}", e.message),
        Ok(_) => panic!("a program with a sample was rendered"),
    }
    assert!(r.pcm.is_empty());
    let restricted = "use restricted\ntempo 120\ndeck A: kick x ~ x ~\n";
    let r = run(restricted, &opts());
    match &r.result {
        Err(e) => assert_eq!(e.code, "restricted", "{}", e.message),
        Ok(_) => panic!("a restricted program was rendered"),
    }
    assert!(r.pcm.is_empty());
    assert_eq!(event(&r, "error").len(), 1);
    // the `#!` header says private too
    let header = "#! visibility: private\ntempo 120\ndeck A: kick x ~ x ~\n";
    let r = run(header, &opts());
    assert_eq!(r.result.err().map(|e| e.code), Some("restricted"));
}

#[test]
fn a_program_with_no_end_is_sixteen_bars_and_max_bars_cuts_it() {
    let r = run(OPEN, &opts());
    assert!(r.result.is_ok());
    assert_eq!(r.pcm.len(), bar_frames(16, 150.0, 44_100) * 4);
    let start = event(&r, "start")[0];
    assert_eq!(f(start, "bars"), 16.0);
    assert!(start.get("notes").and_then(JsonValue::as_array).unwrap().iter().any(|n| n.as_str().is_some_and(|s| s.contains("no end"))));
    // --max-bars lower than that wins, and the notes say it was cut
    let r = run(OPEN, &Options { max_bars: 3, ..opts() });
    assert_eq!(r.pcm.len(), bar_frames(3, 150.0, 44_100) * 4);
    let start = event(&r, "start")[0];
    assert_eq!(f(start, "bars"), 3.0);
    assert!((f(start, "seconds") - 3.0 * 4.0 * 60.0 / 150.0).abs() < 1e-3);
    assert!(start.get("notes").and_then(JsonValue::as_array).unwrap().iter().any(|n| n.as_str().is_some_and(|s| s.contains("--max-bars"))));
    // a song longer than max-bars is cut the same way, and a higher max-bars is not a longer song
    let r = run(SONG, &Options { max_bars: 3, ..opts() });
    assert_eq!(r.pcm.len(), bar_frames(3, 140.0, 44_100) * 4);
    let r = run(SONG, &Options { max_bars: 100, ..opts() });
    assert_eq!(r.pcm.len(), bar_frames(4, 140.0, 44_100) * 4);
    // and the cut render is the head of the whole one
    let whole = run(SONG, &opts());
    let cut = run(SONG, &Options { max_bars: 3, ..opts() });
    // (the closing `cue` shortens the last block of the whole render, so compare short of the last bar)
    let n = bar_frames(2, 140.0, 44_100) * 4;
    assert!(whole.pcm[..n] == cut.pcm[..n]);
}

#[test]
fn end_at_bar_is_honoured_for_a_program_with_no_sections() {
    // `djctl btw render` ignores `end at bar N` for a program with no timeline and renders 16 bars; the
    // site's render is as long as the program says it is, and the bars it has in common are the same
    let text = format!("{OPEN}end at bar 3\n");
    let r = run(&text, &opts());
    assert!(r.result.is_ok(), "{:?}", r.result.err().map(|e| e.message));
    assert_eq!(r.pcm.len(), bar_frames(3, 150.0, 44_100) * 4);
    assert_eq!(f(event(&r, "start")[0], "bars"), 3.0);
    // an `end` beyond 16 bars is that long: 20 bars at 150 bpm, rendered at a low rate to keep this short
    let long = format!("{OPEN}end at bar 20\n");
    let r = run(&long, &Options { sr: 8_000, ..opts() });
    assert_eq!(r.pcm.len(), bar_frames(20, 150.0, 8_000) * 4);
}

#[test]
fn an_end_cuts_an_arranged_song_short_as_the_runtime_does() {
    // `end after two` / `end at bar 2` on a 4-bar arrange: the live runtime stops after bar 2, so the render is 2 bars
    // (it used to render every section: site/test/parity heard the engine go silent where the stream played on)
    for end in ["end after one", "end at bar 2"] {
        let text = format!("{SONG}{end}\n");
        let r = run(&text, &opts());
        assert!(r.result.is_ok(), "{end}: {:?}", r.result.err().map(|e| e.message));
        assert_eq!(f(event(&r, "start")[0], "bars"), 2.0, "{end}");
        assert_eq!(r.pcm.len(), bar_frames(2, 140.0, 44_100) * 4, "{end}");
        // the bars it has are the song's first two
        let full = run(SONG, &opts());
        assert_eq!(&full.pcm[..r.pcm.len()], &r.pcm[..], "{end}: the same bars as the uncut song");
    }
    // an end past the song changes nothing
    let r = run(&format!("{SONG}end at bar 9\n"), &opts());
    assert_eq!(f(event(&r, "start")[0], "bars"), 4.0);
}

#[test]
fn from_bar_is_the_tail_of_the_full_render() {
    for text in [SONG, THREE] {
        let whole = run(text, &opts());
        let bars = f(event(&whole, "start")[0], "bars") as u32;
        let bpm = f(event(&whole, "start")[0], "bpm");
        assert!(bars >= 4);
        for from in [1, 2, bars - 1] {
            let r = run(text, &Options { from_bar: from, chunk_ms: 100, ..opts() });
            let skip = bar_frames(from, bpm, 44_100) * 4;
            assert_eq!(r.pcm.len(), whole.pcm.len() - skip, "from bar {from}");
            assert!(r.pcm[..] == whole.pcm[skip..], "--from-bar {from} is not the tail of the full render");
            let start = event(&r, "start")[0];
            assert_eq!(f(start, "bars"), f64::from(bars - from));
            assert_eq!(f(start, "from_bar"), f64::from(from));
            assert!((f(start, "seconds") - r.pcm.len() as f64 / 4.0 / 44_100.0).abs() < 1e-3);
        }
    }
    // past the end is a usage error, not an empty stream
    let r = run(SONG, &Options { from_bar: 4, ..opts() });
    assert_eq!(r.result.err().map(|e| e.code), Some("usage"));
    assert!(r.pcm.is_empty());
}

#[test]
fn f32le_is_the_same_signal_as_s16le() {
    let s = run(SONG, &opts());
    let f32r = run(SONG, &Options { format: Format::F32le, ..opts() });
    assert_eq!(f32r.pcm.len(), s.pcm.len() * 2);
    let mut max_diff = 0i32;
    for (a, b) in s.pcm.chunks_exact(2).zip(f32r.pcm.chunks_exact(4)) {
        let x = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        let want = {
            let c = x.clamp(-1.0, 1.0);
            (if c < 0.0 { (c * 32768.0).round() } else { (c * 32767.0).round() }) as i16
        };
        max_diff = max_diff.max((i32::from(i16::from_le_bytes([a[0], a[1]])) - i32::from(want)).abs());
    }
    assert_eq!(max_diff, 0, "the s16 stream is the f32 stream converted the way the WAV writer converts");
    assert_eq!(event(&f32r, "start")[0].get("format").and_then(JsonValue::as_str), Some("f32le"));
}

/// The sections' events, made lazily, are the arrangement `djhost::beatscript_render::arrangement` makes.
#[test]
fn the_lazy_arrangement_is_the_one_the_desktop_renders() {
    let env = djhost::beatscript_env::BeatscriptEnv::new(None);
    let corpus = concat!(env!("CARGO_MANIFEST_DIR"), "/../beatscript-api/tests/corpus");
    let mut texts: Vec<(String, String)> = vec![("song".into(), SONG.into()), ("three".into(), THREE.into()), ("open".into(), OPEN.into())];
    for e in std::fs::read_dir(corpus).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "beatscript") {
            texts.push((p.file_name().unwrap().to_string_lossy().into(), std::fs::read_to_string(&p).unwrap()));
        }
    }
    assert!(texts.len() >= 9);
    let (mut same, mut differing) = (0usize, Vec::<String>::new());
    for (name, text) in texts {
        let c = djlive::compile::check_with(&text, &env.env()).compiled.unwrap_or_else(|| panic!("{name} does not check"));
        let (official, official_beats) = djhost::beatscript_render::arrangement(&c);
        let (mine, mine_beats) = render::full_arrangement(&c);
        assert_eq!(mine_beats, official_beats, "{name}: length in beats");
        if mine.json().js_stringified() == official.js_stringified() {
            same += 1;
            continue;
        }
        // The desktop's render now carries the mixer's control lines (filter and level ramps, `do` calls, timed moves, a tempo
        // curve: djhost::beatscript_render, ext-render.md); beatscript's lazy render does not yet (its plan says so in `notes`), so a program
        // with such lines is the one place the two differ. A program with none must still be equal.
        let carries_controls = !c.controls.is_empty() || c.tempo_map.drives();
        assert!(carries_controls, "{name}: the arrangement differs and the program has no control line or tempo curve");
        differing.push(name);
    }
    println!("lazy arrangement: {same} equal, {} differ only by control lines / tempo curve: {differing:?}", differing.len());
    assert!(same >= 5, "only {same} programs have the desktop's arrangement");
}

/// The stream is `djctl btw render`'s output: the same engine, the same blocks, the same arrangement.
/// (`djctl btw render FILE OUT.wav --sr N` is `render_arrangement` of `beatscript_render::arrangement` with
/// `beats` set to the song's length: that is called here, and its WAV samples are compared.)
#[test]
fn the_stream_equals_what_djctl_btw_render_writes() {
    use djengine::session::arrangement::Arrangement;
    use djhost::deck::offline::{render_arrangement, ArrangeRenderOptions};
    let env = djhost::beatscript_env::BeatscriptEnv::new(None);
    for text in [SONG, THREE, OPEN] {
        let c = djlive::compile::check_with(text, &env.env()).compiled.unwrap();
        let (j, beats) = djhost::beatscript_render::arrangement(&c);
        let (arr, _) = Arrangement::from_json(&j).unwrap();
        let sr = 44_100.0;
        let official = render_arrangement(&arr, ArrangeRenderOptions { sample_rate: sr, beats: Some(beats), ..Default::default() });
        let wav = djengine::session::wav::encode_wav(&official.audio, false);
        let want = &wav[44..];
        let ours = run(text, &opts());
        assert!(ours.result.is_ok());
        assert_eq!(ours.pcm.len(), want.len());
        assert!(ours.pcm[..] == *want, "the stream is not djctl btw render's output");
    }
}

// ---- control lines: deck and master levels, ramps, inserts (the desktop's control events + the control net)

/// The same deck A part in both sections; the intro sets deck A to half, the drop to full.
const LEVELS: &str = "tempo 120\nsection intro 4 bars {\n  deck A: kick x ~ x ~ x ~ x ~ x ~ x ~ x ~ | hihat x x x x x x x x x x x x x x x x\n  deck A: level 0.5\n}\nsection drop 4 bars {\n  deck A: kick x ~ x ~ x ~ x ~ x ~ x ~ x ~ | hihat x x x x x x x x x x x x x x x x\n  deck A: level 1.0\n}\narrange intro drop\n";

/// RMS (dB full scale) of the s16le stream from bar `a` to bar `b`.
fn rms_db(pcm: &[u8], bpm: f64, sr: u32, a: u32, b: u32) -> f64 {
    let (i, j) = (bar_frames(a, bpm, sr) * 4, bar_frames(b, bpm, sr) * 4);
    let s: &[u8] = &pcm[i..j.min(pcm.len())];
    let mut acc = 0.0f64;
    let mut n = 0usize;
    for ch in s.chunks_exact(2) {
        let v = f64::from(i16::from_le_bytes([ch[0], ch[1]])) / 32768.0;
        acc += v * v;
        n += 1;
    }
    10.0 * (acc / n.max(1) as f64).max(1e-20).log10()
}

/// `deck A: level 0.5` in the intro and `deck A: level 1.0` in the drop: the drop is louder by about 20·log10(2) ≈ 6 dB. Before
/// beatscript rendered control lines both sections came out at the same level.
#[test]
fn deck_levels_set_per_section_are_heard() {
    let r = run(LEVELS, &opts());
    assert!(r.result.is_ok(), "{:?}", r.result.err());
    let start = event(&r, "start")[0];
    let notes = start.get("notes").unwrap().js_stringified();
    assert!(!notes.contains("not rendered") && !notes.contains("not in this render"), "levels are rendered: {notes}");
    // skip each section's first bar: the level moves on the bar line, the tail of the bar before rings on
    let intro = rms_db(&r.pcm, 120.0, 44_100, 1, 4);
    let drop = rms_db(&r.pcm, 120.0, 44_100, 5, 8);
    let diff = drop - intro;
    println!("intro {intro:.2} dB, drop {drop:.2} dB, diff {diff:.2} dB");
    assert!((diff - 6.02).abs() < 1.5, "the drop is ~6 dB louder than the intro: intro {intro:.2} dB, drop {drop:.2} dB");
    // and without the level lines the two sections are the same
    let flat = run(&LEVELS.replace("  deck A: level 0.5\n", "").replace("  deck A: level 1.0\n", ""), &opts());
    let d0 = rms_db(&flat.pcm, 120.0, 44_100, 5, 8) - rms_db(&flat.pcm, 120.0, 44_100, 1, 4);
    assert!(d0.abs() < 0.5, "no level lines: no difference ({d0:.2} dB)");
}

/// A ramp (`level 0.1 -> 1 over 4 bars`) and the master level: the first bar is far quieter than the last, the master halves all.
#[test]
fn level_ramps_and_the_master_level_are_heard() {
    let ramp = "tempo 120\ndeck A: hihat x x x x x x x x x x x x x x x x\ndeck A: level 0.1 -> 1 over 4 bars\nend at bar 4\n";
    let r = run(ramp, &opts());
    assert!(r.result.is_ok(), "{:?}", r.result.err());
    let (first, last) = (rms_db(&r.pcm, 120.0, 44_100, 0, 1), rms_db(&r.pcm, 120.0, 44_100, 3, 4));
    assert!(last - first > 8.0, "the ramp rises: first bar {first:.2} dB, last {last:.2} dB");
    let plain = "tempo 120\ndeck A: hihat x x x x x x x x x x x x x x x x\nend at bar 2\n";
    let full = run(plain, &opts());
    let half = run(&plain.replace("end at bar 2\n", "master: level 0.5\nend at bar 2\n"), &opts());
    let d = rms_db(&full.pcm, 120.0, 44_100, 0, 2) - rms_db(&half.pcm, 120.0, 44_100, 0, 2);
    assert!(d > 3.0, "master level 0.5 is quieter: {d:.2} dB");
}

/// An insert (a master low-pass) is heard: the hats lose most of their energy.
#[test]
fn a_master_filter_is_heard() {
    let plain = "tempo 120\ndeck A: hihat x x x x x x x x x x x x x x x x\nend at bar 2\n";
    let full = run(plain, &opts());
    let lp = run(&plain.replace("end at bar 2\n", "master: filter lp 200\nend at bar 2\n"), &opts());
    assert!(lp.result.is_ok(), "{:?}", lp.result.err());
    let d = rms_db(&full.pcm, 120.0, 44_100, 0, 2) - rms_db(&lp.pcm, 120.0, 44_100, 0, 2);
    assert!(d > 3.0, "a low-pass at 200 Hz takes the hats down: {d:.2} dB");
}

/// With control lines the render is still deterministic, chunk-size invariant, and a seek is the tail of the full render.
#[test]
fn control_lines_keep_the_render_deterministic_and_seekable() {
    let a = run(LEVELS, &opts());
    let b = run(LEVELS, &Options { chunk_ms: 7, ..opts() });
    assert!(a.pcm == b.pcm, "same bytes whatever the chunk size");
    let tail = run(LEVELS, &Options { from_bar: 5, ..opts() });
    let off = bar_frames(5, 120.0, 44_100) * 4;
    assert!(tail.pcm[..] == a.pcm[off..], "--from-bar 5 is the tail of the full render");
}

/// Both behaviours at once (fix/render-honor-levels + the parity branch's `end` fix): an arranged song with per-section
/// levels and an `end after` is cut where the runtime stops, and the levels before the cut are still heard.
#[test]
fn an_end_and_section_levels_work_together() {
    let text = LEVELS.replace("arrange intro drop\n", "section outro 4 bars {\n  deck A: kick x ~ x ~ x ~ x ~ x ~ x ~ x ~\n}\narrange intro drop outro\nend after drop\n");
    let r = run(&text, &opts());
    assert!(r.result.is_ok(), "{:?}", r.result.err().map(|e| e.message));
    assert_eq!(f(event(&r, "start")[0], "bars"), 8.0, "the outro is cut by `end after drop`");
    assert_eq!(r.pcm.len(), bar_frames(8, 120.0, 44_100) * 4);
    let diff = rms_db(&r.pcm, 120.0, 44_100, 5, 8) - rms_db(&r.pcm, 120.0, 44_100, 1, 4);
    assert!((diff - 6.02).abs() < 1.5, "the drop is still ~6 dB louder than the intro ({diff:.2} dB)");
}

/// A render sings a `sing` line as the browser does, in its deck's net: the voice is in the PCM (so `listen` hears it).
#[test]
fn a_render_sings_a_sing_line() {
    let sing = "tempo 120\nkey am\ndeck A: kick x ~ x ~ | melody <[a3 c4 e4 c4] [a3 c4 e4 g4]>\ndeck A: sing \"we are the rave to-night\" on melody voice=robot-choir\nend at bar 4\n";
    let plain = sing.replace("deck A: sing \"we are the rave to-night\" on melody voice=robot-choir\n", "");
    let a = run(sing, &opts());
    let b = run(&plain, &opts());
    assert!(a.result.is_ok() && b.result.is_ok());
    let s16 = |p: &[u8]| p.chunks_exact(2).map(|c| f64::from(i16::from_le_bytes([c[0], c[1]])) / 32768.0).collect::<Vec<f64>>();
    let (x, y) = (s16(&a.pcm), s16(&b.pcm));
    let rms = |v: &[f64]| (v.iter().map(|s| s * s).sum::<f64>() / v.len().max(1) as f64).sqrt();
    let n = x.len().min(y.len());
    let diff: Vec<f64> = (0..n).map(|i| x[i] - y[i]).collect();
    assert!(rms(&diff) > 0.1 * rms(&y[..n]), "the voice is not in the render: diff {} vs {}", rms(&diff), rms(&y[..n]));
}
