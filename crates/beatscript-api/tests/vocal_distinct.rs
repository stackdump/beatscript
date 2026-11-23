//! Vowel- and word-distinctness of the sung voice, measured (project-datum 6627: "sung words are hard to understand").
//!
//! Nobody can listen in CI, so the voice is measured instead, headless, through the engine the page runs: a `sing` line
//! is compiled by `beatscript_api::run::Run` exactly as the browser does, its sung part (`vox`, `voxc`) is rendered offline by
//! `host::beats::render_offline` (no audio device, deterministic), and the render is read back:
//!
//! * **F1 and F2 of a held vowel** (LPC, 14 poles at 12 kHz, a 40 ms window in the steady part, the first two peaks of
//!   the envelope above 150 Hz), then the distance between two vowels in the (F1, F2) plane in **Bark** (Traunmuller).
//! * **A-weighted RMS per word** over `we see the street` dry on `voice-robot`, one held note a word, against the loudest.
//!
//! The tests pin what was measured to be wrong and was changed in the sung voice (the vowel table and the rounding of
//! `oh`, the held-and-released envelope, the preset levels and the loudness probe: the public engine crate `djrust`), and
//! what held before and must go on holding: the render keeps the vowel table (so the measuring is sound), the front pair
//! `/ii/` vs `/aa/` (which Matt hears as clearly different) sets the bar, and `decay=` on the part a line is sung on does
//! not touch the sung part. `-- --ignored --nocapture report` prints the numbers.

use beatscript_api::run::Run;
use djengine::json::{JsonObject, JsonValue};

const SR: f64 = 48000.0;

fn load_vox(text: &str) -> JsonValue {
    load_nets(text, "vox")
}

/// The first deck's envelope with only the parts whose name starts with `prefix` left in it.
fn load_nets(text: &str, prefix: &str) -> JsonValue {
    let mut run = Run::new();
    let ev = run.eval(text);
    assert_eq!(ev.get("ok").and_then(JsonValue::as_bool), Some(true), "{}", ev.js_stringified());
    let out = run.service(None);
    let env = out
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c.get("op").and_then(JsonValue::as_str) == Some("load_net"))
        .map(|c| c.get("envelope").unwrap().clone())
        .expect("a net loads");
    // the sung parts alone: the deck's other parts out of the envelope
    let mut solo = env.clone();
    if let JsonValue::Object(o) = &mut solo {
        if let Some(JsonValue::Object(nets)) = o.get("nets").cloned().as_ref() {
            let mut keep = JsonObject::new();
            for (id, n) in nets.iter() {
                if id.starts_with(prefix) {
                    keep.set(id, n.clone());
                }
            }
            o.set("nets", JsonValue::Object(keep));
        }
    }
    solo
}

fn render(env: &JsonValue, seconds: f64) -> Vec<f32> {
    use djengine::host::deck::engine::EngineConfig;
    use djengine::host::deck::host::host_engine;
    let st = djengine::host::beats::render_offline(
        || {
            let mut cfg = EngineConfig::new(SR);
            cfg.history_beats = None;
            host_engine(cfg, false, true)
        },
        &env.js_stringified(),
        seconds,
    )
    .unwrap();
    st.chunks(2).map(|c| 0.5 * (c[0] + c[1])).collect()
}

fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

fn db(x: f64) -> f64 {
    20.0 * x.max(1e-12).log10()
}

/// Decimate by 4 with a windowed-sinc lowpass.
fn decimate4(x: &[f32]) -> Vec<f64> {
    let taps = 61usize;
    let fc = 0.5 / 4.0 * 0.9;
    let mid = (taps / 2) as f64;
    let h: Vec<f64> = (0..taps)
        .map(|i| {
            let t = i as f64 - mid;
            let s = if t == 0.0 { 2.0 * fc } else { (2.0 * std::f64::consts::PI * fc * t).sin() / (std::f64::consts::PI * t) };
            let w = 0.54 - 0.46 * (2.0 * std::f64::consts::PI * i as f64 / (taps - 1) as f64).cos();
            s * w
        })
        .collect();
    (0..x.len().saturating_sub(taps) / 4).map(|n| (0..taps).map(|k| h[k] * f64::from(x[n * 4 + k])).sum()).collect()
}

/// LPC formants of the 40 ms around `centre` seconds (peaks of the LPC envelope).
fn formants(x: &[f32], centre: f64, order: usize) -> Vec<f64> {
    let d = decimate4(x);
    let fs = SR / 4.0;
    let n = (0.040 * fs) as usize;
    let c = (centre * fs) as usize;
    let a = c.saturating_sub(n / 2);
    let seg: Vec<f64> = d[a..(a + n).min(d.len())].to_vec();
    let mut w: Vec<f64> = seg.iter().enumerate().map(|(i, v)| v * (0.54 - 0.46 * (2.0 * std::f64::consts::PI * i as f64 / (n - 1) as f64).cos())).collect();
    for i in (1..w.len()).rev() {
        w[i] -= 0.97 * w[i - 1];
    }
    let r: Vec<f64> = (0..=order).map(|k| (0..w.len() - k).map(|i| w[i] * w[i + k]).sum()).collect();
    // Levinson
    let mut al = vec![0.0; order + 1];
    al[0] = 1.0;
    let mut e = r[0] * (1.0 + 1e-9) + 1e-30;
    for i in 1..=order {
        let mut acc = r[i];
        for j in 1..i {
            acc += al[j] * r[i - j];
        }
        let k = -acc / e;
        let prev = al.clone();
        for j in 1..i {
            al[j] = prev[j] + k * prev[i - j];
        }
        al[i] = k;
        e *= 1.0 - k * k;
    }
    let spec = |f: f64| -> f64 {
        let wv = 2.0 * std::f64::consts::PI * f / fs;
        let (mut re, mut im) = (0.0, 0.0);
        for (k, a) in al.iter().enumerate() {
            re += a * (wv * k as f64).cos();
            im -= a * (wv * k as f64).sin();
        }
        1.0 / (re * re + im * im).max(1e-30)
    };
    let step = 5.0;
    let grid: Vec<f64> = (0..(4500.0 / step) as usize).map(|i| spec(i as f64 * step)).collect();
    let mut out = Vec::new();
    for i in 2..grid.len() - 1 {
        if grid[i] > grid[i - 1] && grid[i] >= grid[i + 1] && i as f64 * step > 150.0 {
            out.push(i as f64 * step);
        }
    }
    out
}


// ---- measuring

/// Second-order sections of the IEC 61672 A-weighting at `fs`, by the bilinear transform, 0 dB at 1 kHz.
fn a_weight(x: &[f32], fs: f64) -> Vec<f64> {
    let (f1, f2, f3, f4) = (20.598997, 107.65265, 737.86223, 12194.217);
    let w = |f: f64| 2.0 * std::f64::consts::PI * f;
    // (b2 s^2 + b1 s + b0) / (a2 s^2 + a1 s + a0), each as [s^2, s, 1]
    let sections = [
        ([1.0, 0.0, 0.0], [1.0, 2.0 * w(f1), w(f1).powi(2)]),
        ([1.0, 0.0, 0.0], [1.0, w(f2) + w(f3), w(f2) * w(f3)]),
        ([0.0, 0.0, w(f4).powi(2)], [1.0, 2.0 * w(f4), w(f4).powi(2)]),
    ];
    let c = 2.0 * fs;
    let z = |p: [f64; 3]| [p[0] * c * c + p[1] * c + p[2], -2.0 * p[0] * c * c + 2.0 * p[2], p[0] * c * c - p[1] * c + p[2]];
    let bq: Vec<([f64; 3], [f64; 3])> = sections
        .iter()
        .map(|(b, a)| {
            let (nb, na) = (z(*b), z(*a));
            ([nb[0] / na[0], nb[1] / na[0], nb[2] / na[0]], [1.0, na[1] / na[0], na[2] / na[0]])
        })
        .collect();
    let run = |input: &[f64]| -> Vec<f64> {
        let mut y: Vec<f64> = input.to_vec();
        for (b, a) in &bq {
            let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
            for v in y.iter_mut() {
                let x0 = *v;
                let o = b[0] * x0 + b[1] * x1 + b[2] * x2 - a[1] * y1 - a[2] * y2;
                (x2, x1, y2, y1) = (x1, x0, y1, o);
                *v = o;
            }
        }
        y
    };
    // the gain at 1 kHz, from a tone
    let tone: Vec<f64> = (0..(fs as usize)).map(|n| (2.0 * std::f64::consts::PI * 1000.0 * n as f64 / fs).sin()).collect();
    let t = run(&tone);
    let g = (t[t.len() / 2..].iter().map(|v| v * v).sum::<f64>() / (t.len() / 2) as f64 * 2.0).sqrt();
    run(&x.iter().map(|v| f64::from(*v)).collect::<Vec<_>>()).iter().map(|v| v / g).collect()
}

/// A-weighted RMS, dB, of the render between two times (seconds).
fn a_rms_db(pcm: &[f32], from: f64, to: f64) -> f64 {
    let w = a_weight(pcm, SR);
    let (a, b) = ((from.max(0.0) * SR) as usize, ((to * SR) as usize).min(w.len()));
    db((w[a..b].iter().map(|v| v * v).sum::<f64>() / (b - a) as f64).sqrt())
}

/// Traunmuller's Bark.
fn bark(f: f64) -> f64 {
    26.81 * f / (1960.0 + f) - 0.53
}

/// Distance of two vowels in the (F1, F2) plane, Bark.
fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((bark(a.0) - bark(b.0)).powi(2) + (bark(a.1) - bark(b.1)).powi(2)).sqrt()
}

/// `sing "/v/"` on one note of A2 held through two seconds on `voice-robot`; its (F1, F2), the median of three windows of the
/// steady part (a diphthong has settled on its second vowel by then).
fn held(v: &str) -> (f64, f64) {
    let text = format!("tempo 120\ndeck A: melody [a2 ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~]\ndeck A: sing \"/{v}/\" on melody voice=robot\n");
    let pcm = render(&load_vox(&text), 2.0);
    let med = |k: usize| {
        let mut x: Vec<f64> = [0.45, 0.6, 0.75].iter().map(|&c| formants(&pcm, c, 14)[k]).collect();
        x.sort_by(|a, b| a.partial_cmp(b).unwrap());
        x[1]
    };
    (med(0), med(1))
}

/// `we see the street` dry on `voice-robot`, one held note a word (a second each), each word's A-weighted level over the
/// first 0.75 s of its note and the 0.15 s of consonant before it, dB.
fn words(lyric: &str) -> Vec<f64> {
    let text = format!("tempo 120\ndeck A: melody <[a3 ~ ~ ~ ~ ~ ~ ~ c4 ~ ~ ~ ~ ~ ~ ~] [e4 ~ ~ ~ ~ ~ ~ ~ g4 ~ ~ ~ ~ ~ ~ ~]>\ndeck A: sing \"{lyric}\" on melody voice=robot\n");
    let pcm = render(&load_vox(&text), 4.2);
    (0..4).map(|i| a_rms_db(&pcm, i as f64 - 0.15, i as f64 + 0.75)).collect()
}

/// The engine's own vowel table (`djrust::fx::formant::VOWELS`), by the phoneme that sings each entry.
fn table() -> Vec<(&'static str, (f64, f64))> {
    let v = &djengine::fx::formant::VOWELS;
    [("aa", 0), ("eh", 1), ("ee", 2), ("aw", 3), ("oo", 4), ("uh", 5), ("uu", 6), ("ae", 7), ("ii", 8), ("er", 9)].iter().map(|(n, i)| (*n, (v[*i].f[0], v[*i].f[1]))).collect()
}

#[test]
fn the_render_keeps_the_vowel_table_so_the_measuring_is_sound() {
    for (v, (f1, f2)) in table() {
        let (m1, m2) = held(v);
        assert!((m1 / f1 - 1.0).abs() < 0.06 && (m2 / f2 - 1.0).abs() < 0.06, "/{v}/ measures F1 {m1} F2 {m2}, the table says {f1} {f2}");
    }
}

#[test]
#[ignore = "prints the measurements: cargo test -p beatscript-api --test vocal_distinct -- --ignored --nocapture report"]
fn vowel_report() {
    // not an assertion: what is measured, for whoever changes the voice (cargo test ... vowel_report -- --nocapture)
    let vs = ["aa", "uh", "aw", "oh", "ow", "uu", "oo", "ii", "ee"];
    let m: Vec<(f64, f64)> = vs.iter().map(|v| held(v)).collect();
    for (v, f) in vs.iter().zip(&m) {
        println!("/{v:2}/ held: F1 {:4.0} F2 {:4.0} Hz", f.0, f.1);
    }
    print!("Bark distance:      ");
    for v in vs {
        print!("{v:>6}");
    }
    println!();
    for (i, a) in vs.iter().enumerate() {
        print!("{a:>3}                 ");
        for b in &m {
            print!("{:6.2}", dist(m[i], *b));
        }
        println!();
    }
    let w = words("we see the street");
    println!("we see the street, A-weighted dB: {:?}", w.iter().map(|x| (x * 10.0).round() / 10.0).collect::<Vec<_>>());
}

/// One note (`/aa/`, 7 sixteenths = 0.875 s at 120 bpm) and then a rest: the level of the note's middle and of the two
/// 0.2 s stretches after its end, dB, for `voice`.
fn ring(voice: &str) -> (f64, f64, f64) {
    let text = format!("tempo 120\ndeck A: melody <[a3 ~ ~ ~ ~ ~ ~ ~ c4 ~ ~ ~ ~ ~ ~ ~] [a3 ~ ~ ~ ~ ~ ~ ~ c4 ~ ~ ~ ~ ~ ~ ~]>\ndeck A: sing \"/aa _/\" on melody voice={voice}\n");
    let pcm = render(&load_vox(&text), 2.0);
    (a_rms_db(&pcm, 0.3, 0.5), a_rms_db(&pcm, 0.95, 1.15), a_rms_db(&pcm, 1.15, 1.35))
}

#[test]
#[ignore = "prints the measurements: cargo test -p beatscript-api --test vocal_distinct -- --ignored --nocapture report"]
fn ring_report() {
    for v in ["robot", "robot-choir", "choir-aah", "soprano", "bass-choir"] {
        let (mid, a, b) = ring(v);
        println!("voice-{v:12} note middle {mid:6.1} dB, 0.1-0.3 s after its end {:5.1} dB re the middle, 0.3-0.5 s {:5.1}", a - mid, b - mid);
    }
}

// ---- what held before the changes and holds still

#[test]
fn the_front_pair_that_is_heard_as_clearly_different_sets_the_bar() {
    // Matt: /ii/ and /aa/ held on one pitch sound clearly different. In Bark that is the distance the back vowels are held to.
    let (a, i) = (held("aa"), held("ii"));
    let d = dist(a, i);
    assert!((4.5..6.0).contains(&d), "/aa/ {a:?} /ii/ {i:?}: {d} Bark");
}

#[test]
fn decay_on_the_part_a_line_is_sung_on_does_not_cut_what_is_sung() {
    // `decay=` is the part's own tail (the melody's, 160 ms here): the sung part is another part with the voice's own
    // envelope, so the vowel is the same whatever the melody's decay says. (A short decay on the sung part itself would cut the
    // vowel and leave the consonant: that is what the record's `decay=160` heard, and a `sing` line has no `decay=` of its own.)
    let sung = |part: &str| render(&load_vox(&format!("tempo 120\ndeck A: melody [a3 c4 e4 g4]{part}\ndeck A: sing \"we see the street\" on melody voice=robot\n")), 2.0);
    assert_eq!(sung(""), sung(" decay=160"));
}

// ---- what the voice was changed to do (project-datum 6627)

#[test]
fn the_back_vowels_are_clearly_apart() {
    // Thresholds, from the engine's own source (Peterson & Barney) and the pair Matt hears as clearly different: in natural speech
    // the closest two back vowels (/aa/ and /uh/) are 1.0 Bark apart, so no pair of back vowels may be nearer than that (a held
    // /oh/ and /uu/ were 0.00); /aa/ and /uh/ (the schwa) are held to 1.5, 50 % more than speech, because a held note has no
    // context to tell them apart; and the pair Matt hears as alike, /oh/ and /aa/, is held to 0.55 of the front pair /aa/-/ii/.
    let front = dist(held("aa"), held("ii"));
    let back = ["aa", "uh", "aw", "oh", "uu", "oo"];
    let m: Vec<(f64, f64)> = back.iter().map(|v| held(v)).collect();
    let mut close = Vec::new();
    for i in 0..back.len() {
        for j in i + 1..back.len() {
            let d = dist(m[i], m[j]);
            if d < 1.0 {
                close.push(format!("/{}/ and /{}/ are {d:.2} Bark apart", back[i], back[j]));
            }
        }
    }
    assert!(close.is_empty(), "no pair of back vowels nearer than 1.0 Bark: {close:?}");
    assert!(dist(m[0], m[1]) >= 1.5, "/aa/ and /uh/: {:.2} Bark", dist(m[0], m[1]));
    assert!(dist(m[3], m[0]) >= 0.55 * front, "/oh/ and /aa/: {:.2} Bark, the front pair is {front:.2}", dist(m[3], m[0]));
}

#[test]
fn every_word_of_we_see_the_street_is_heard_and_the_function_word_is_not_lost() {
    // Words differ in level by their vowel (Lehiste and Peterson: /i/ is the weakest vowel, 5 to 7 dB under /a/), so the bar for
    // every word is 8 dB under the loudest (A-weighted); "the" (/dh uh/, the schwa Matt thought lost) is held to 3 dB.
    let w = words("we see the street");
    let top = w.iter().cloned().fold(f64::MIN, f64::max);
    for (word, l) in ["we", "see", "the", "street"].iter().zip(&w) {
        assert!(top - l <= 8.0, "`{word}` is {:.1} dB under the loudest ({w:?})", top - l);
    }
    assert!(top - w[2] <= 3.0, "`the` is {:.1} dB under the loudest ({w:?})", top - w[2]);
}

#[test]
fn a_syllable_does_not_ring_into_the_next() {
    // 0.1 to 0.3 s after a note ends it must be 12 dB under the note's middle: the words are then apart (a note was only
    // 1 to 7 dB down there, on every preset, when its envelope was a decay of 1.1 times its length)
    for v in ["robot", "robot-choir", "choir-aah", "soprano", "bass-choir"] {
        let (mid, after, _) = ring(v);
        assert!(after - mid <= -12.0, "voice-{v} is {:.1} dB under its middle 0.1 to 0.3 s after the note ends", mid - after);
    }
}

/// `n` notes of A3, a part on `voice-robot` that sings no syllable (its vowel), 8 sixteenths long, with `extra` written on it:
/// the A-weighted level between two times, dB.
fn voice_part(extra: &str, from: f64, to: f64) -> f64 {
    let text = format!("tempo 120\ndeck A: melody [a3 ~ ~ ~ ~ ~ ~ ~] len=8 inst=voice-robot{extra}\n");
    a_rms_db(&render(&load_nets(&text, "melody"), 1.6), from, to)
}

#[test]
fn a_held_vowel_keeps_its_level_through_its_note() {
    // the note is 8 sixteenths (1 s): its last quarter is within 2.5 dB of its first (it drooped 4.3 dB between those
    // windows, and 8 dB over the note, as a decay of 1.1 times its length)
    let early = voice_part("", 0.15, 0.25);
    let late = voice_part("", 0.70, 0.80);
    assert!(late - early >= -2.5, "the vowel falls {:.1} dB over its note", early - late);
}

#[test]
fn decay_on_a_voice_part_is_its_tail_and_never_cuts_the_vowel() {
    // `decay=160` was the vowel's own time constant: 27 dB down by the middle of a one-second note, so a sung part was its
    // consonants. It is now the length of the release after the note, and the vowel holds as it does without it.
    let early = voice_part(" decay=160", 0.15, 0.25);
    let late = voice_part(" decay=160", 0.70, 0.80);
    assert!(late - early >= -4.0, "the vowel falls {:.1} dB over its note with decay=160", early - late);
    assert!((late - voice_part("", 0.70, 0.80)).abs() < 1.0, "decay=160 changes the held vowel");
    // and it is heard as the tail: a longer decay rings longer after the note ends (at 1.0 s), 0.15 to 0.25 s after it
    let tail = |extra: &str| voice_part(extra, 1.15, 1.25) - voice_part(extra, 0.70, 0.80);
    assert!(tail(" decay=400") > tail(" decay=60") + 6.0, "{} against {}", tail(" decay=400"), tail(" decay=60"));
}

#[test]
fn the_voices_play_at_about_the_same_level() {
    // the same vowel on the same note through the page's engine, A-weighted: no preset more than 10 dB under the loudest. It was
    // 19 dB for voice-soprano (the loudness probe heard it at 8 kHz, where its upper formants do not exist), 18 and 21 dB for
    // voice-android and voice-talkbox (their parallel formants sum quiet).
    let all = ["robot", "robot-choir", "android", "choir-aah", "choir-ooh", "soprano", "bass-choir", "talkbox"];
    let l: Vec<(&str, f64)> = all.iter().map(|v| (*v, ring(v).0)).collect();
    let top = l.iter().map(|x| x.1).fold(f64::MIN, f64::max);
    for (v, x) in &l {
        assert!(top - x <= 10.0, "voice-{v} is {:.1} dB under the loudest preset ({l:?})", top - x);
    }
}

#[test]
fn a_final_consonant_is_heard() {
    // The syllable's coda (the `t` of "bat", the `v` of "rave", the `d` of "hands") closes the note. In the net the lane never
    // began it, so `/b ae t/` and `/b ae/` were the same samples to the bit and every word lost its last sound.
    let r = |syl: &str| {
        let text = format!("tempo 120\ndeck A: melody <[a3 ~ ~ ~ ~ ~ ~ ~ c4 ~ ~ ~ ~ ~ ~ ~] [a3 ~ ~ ~ ~ ~ ~ ~ c4 ~ ~ ~ ~ ~ ~ ~]>\ndeck A: sing \"/{syl} _/\" on melody voice=robot\n");
        render(&load_vox(&text), 1.5)
    };
    for coda in ["t", "k", "s", "d", "v", "n"] {
        let (with, without) = (r(&format!("b ae {coda}")), r("b ae"));
        let diff: Vec<f32> = with.iter().zip(&without).map(|(x, y)| x - y).collect();
        let (a, b) = ((0.80 * SR) as usize, (0.97 * SR) as usize);
        let (d, vowel) = (db(rms(&diff[a..b])), db(rms(&with[(0.3 * SR) as usize..(0.5 * SR) as usize])));
        println!("coda {coda}: differs from none by {d:.1} dB at the end, the vowel is {vowel:.1}");
        assert!(d > vowel - 15.0, "the coda `{coda}` is {:.1} dB under the vowel, i.e. not there", vowel - d);
    }
}

