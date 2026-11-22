//! Key detection, Camelot and compatibility (docs/KEY.md). The audio fixtures are synthesised
//! here, so the expected key is known by construction: a I–IV–V–I cadence (major) or
//! i–iv–V–i in harmonic minor (the raised leading tone, as tonal music writes it), triads of
//! sines with two overtones over the chord's root in the bass, in every one of the 24 keys.

use djengine::key::{self, compat, suggest_transpose, Compat, Key, Profile};

const SR: f64 = 44_100.0;

fn hz(midi: i32) -> f64 {
    440.0 * 2f64.powf((midi as f64 - 69.0) / 12.0)
}

/// One chord: the triad on `root` (MIDI) in the middle register and its root an octave down.
fn chord(out: &mut Vec<f32>, root: i32, third: i32, seconds: f64) {
    let n = (seconds * SR) as usize;
    let start = out.len();
    out.resize(start + n, 0.0);
    let notes = [root - 12, root, root + third, root + 7];
    for (i, s) in out[start..].iter_mut().enumerate() {
        let t = i as f64 / SR;
        let env = (1.0 - t / seconds).max(0.0) * (t * 200.0).min(1.0);
        let mut v = 0.0;
        for &m in &notes {
            let f = hz(m);
            v += (2.0 * std::f64::consts::PI * f * t).sin() + 0.4 * (4.0 * std::f64::consts::PI * f * t).sin()
                + 0.2 * (6.0 * std::f64::consts::PI * f * t).sin();
        }
        *s = (v * env * 0.1) as f32;
    }
}

/// The cadence in `k`, tonic around middle C.
fn cadence(k: Key) -> Vec<f32> {
    let tonic = 60 + k.root as i32;
    let tonic = if tonic > 66 { tonic - 12 } else { tonic };
    let third = if k.minor { 3 } else { 4 };
    let mut out = Vec::new();
    chord(&mut out, tonic, third, 1.5); // I / i
    chord(&mut out, tonic + 5, third, 1.5); // IV / iv
    chord(&mut out, tonic + 7, 4, 1.5); // V (major in both: the leading tone)
    chord(&mut out, tonic, third, 2.0); // I / i
    out
}

fn all_keys() -> Vec<Key> {
    (0..24).map(|i| Key::new(i % 12, i >= 12)).collect()
}

#[test]
fn the_ensemble_finds_every_key_of_a_cadence() {
    for k in all_keys() {
        let d = key::detect(&cadence(k), SR, Profile::Ensemble).expect("audio");
        assert_eq!(d.key, k, "{}: detected {} (votes {:?})", k.name(), d.key.name(), d.votes);
        assert!(d.tonal, "{}: a cadence is tonal", k.name());
        assert_eq!(d.votes.len(), 3);
    }
}

#[test]
fn each_published_profile_gets_most_keys_on_its_own() {
    for p in Profile::ALL {
        let right = all_keys().into_iter().filter(|k| key::detect(&cadence(*k), SR, p).unwrap().key == *k).count();
        assert!(right >= 22, "{}: {right}/24", p.name());
    }
}

#[test]
fn noise_and_drums_are_not_tonal() {
    // a deterministic noise burst train: no pitch to speak of
    let mut x = vec![0.0f32; (SR * 4.0) as usize];
    let mut s = 12345u32;
    for (i, v) in x.iter_mut().enumerate() {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        let env = (1.0 - (i % 11_025) as f32 / 2000.0).max(0.0);
        *v = ((s as f32 / u32::MAX as f32) * 2.0 - 1.0) * env;
    }
    let d = key::detect(&x, SR, Profile::Ensemble).unwrap();
    assert!(!d.tonal, "noise read as {} tonal", d.key.name());
    assert!(key::detect(&[0.0; 100], SR, Profile::Ensemble).is_none(), "too short");
    assert!(key::detect(&vec![0.0; 20_000], SR, Profile::Ensemble).is_none(), "silence");
}

#[test]
fn camelot_codes_match_the_wheel() {
    let cases = [
        ("C major", "8B"), ("A minor", "8A"), ("G major", "9B"), ("E minor", "9A"), ("F major", "7B"),
        ("D minor", "7A"), ("B major", "1B"), ("Ab minor", "1A"), ("F# major", "2B"), ("Eb minor", "2A"),
        ("Db major", "3B"), ("Bb minor", "3A"), ("Ab major", "4B"), ("F minor", "4A"), ("Eb major", "5B"),
        ("C minor", "5A"), ("Bb major", "6B"), ("G minor", "6A"), ("D major", "10B"), ("B minor", "10A"),
        ("A major", "11B"), ("F# minor", "11A"), ("E major", "12B"), ("C# minor", "12A"),
    ];
    for (name, code) in cases {
        let k = Key::parse(name).unwrap_or_else(|| panic!("parse {name}"));
        assert_eq!(k.camelot(), code, "{name}");
        assert_eq!(Key::from_camelot(code), Some(k), "{code}");
        assert_eq!(Key::parse(code), Some(k), "{code} parses as a key");
    }
}

#[test]
fn keys_parse_as_people_and_the_generator_write_them() {
    let am = Key::new(9, true);
    for t in ["A minor", "Am", "a min", "A_minor", "8A", "A aeolian", "A dorian", "A minor pentatonic", "A MinorScale"] {
        assert_eq!(Key::parse(t), Some(am), "{t}");
    }
    assert_eq!(Key::parse("Bbm"), Some(Key::new(10, true)));
    assert_eq!(Key::parse("Bm"), Some(Key::new(11, true)));
    assert_eq!(Key::parse("F#"), Some(Key::new(6, false)));
    assert_eq!(Key::parse("C mixolydian"), Some(Key::new(0, false)));
    assert_eq!(Key::parse("Eb major"), Some(Key::new(3, false)));
    for bad in ["", "H minor", "13A", "C weird", "0B"] {
        assert_eq!(Key::parse(bad), None, "{bad}");
    }
    // the generator's own names
    assert_eq!(Key::from_root_scale(54, "Major"), Key::new(6, false));
    assert_eq!(Key::from_root_scale(45, "MinorScale"), am);
    assert_eq!(Key::from_root_scale(43, "Minor").camelot(), "6A");
}

#[test]
fn a_generated_tracks_key_is_the_one_it_was_composed_in() {
    // the loop sets write it ("A minor"); the project carries it as rootNote + scaleName
    for (g, seed) in [("techno", 7u32), ("dnb", 2602), ("house", 42)] {
        let p = djengine::loopsets::played(g, seed);
        let k = Key::from_project(&p).expect("a generated track has a root");
        let label = djengine::loopsets::key_label(p.root_note.unwrap(), &p.scale_name);
        assert_eq!(Key::parse(&label), Some(k), "{g}:{seed} {label}");
    }
}

#[test]
fn compatibility_follows_the_wheel() {
    let k = |s: &str| Key::parse(s).unwrap();
    assert_eq!(compat(k("8A"), k("8A")), Compat::Same);
    assert_eq!(compat(k("8A"), k("8B")), Compat::Relative);
    assert_eq!(compat(k("8A"), k("9A")), Compat::Adjacent);
    assert_eq!(compat(k("8A"), k("7A")), Compat::Adjacent);
    assert_eq!(compat(k("12A"), k("1A")), Compat::Adjacent, "the wheel wraps");
    assert_eq!(compat(k("8A"), k("9B")), Compat::Diagonal);
    assert_eq!(compat(k("8A"), k("10A")), Compat::Boost);
    assert_eq!(compat(k("A minor"), k("A major")), Compat::Parallel);
    assert_eq!(compat(k("8A"), k("2A")), Compat::Clash);
    assert!(compat(k("8A"), k("9A")).harmonic() && !compat(k("8A"), k("3B")).harmonic());
}

#[test]
fn the_suggested_transpose_is_the_smallest_that_mixes() {
    let a = Key::parse("A minor").unwrap();
    // already harmonic: leave it
    assert_eq!(suggest_transpose(a, Key::parse("E minor").unwrap()).semitones, 0);
    // a semitone off is a semitone back
    let t = suggest_transpose(a, Key::parse("Bb minor").unwrap());
    assert_eq!((t.semitones, t.compat), (-1, Compat::Same));
    // every key can be brought into harmony within a tritone
    for b in (0..24).map(|i| Key::new(i % 12, i >= 12)) {
        let t = suggest_transpose(a, b);
        assert!(t.compat.harmonic(), "{} → {}", b.name(), t.key.name());
        assert!(t.semitones.abs() <= 6);
        assert_eq!(b.transposed(t.semitones), t.key);
    }
    let j = key::pair_json(Some(a), Some(Key::parse("2A").unwrap()));
    assert_eq!(j.get("compat").and_then(|v| v.as_str()), Some("clash"));
    assert!(j.get("suggest").is_some());
}
