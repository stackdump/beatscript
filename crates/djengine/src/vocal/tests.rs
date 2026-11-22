use super::*;

fn hook(lyrics: &str, key: &str, bpm: f64, bars: u32) -> VocalLine {
    VocalLine::create(Params::new(lyrics, Key::parse(key).unwrap(), bpm, bars)).unwrap().0
}

#[test]
fn a_line_is_canonical_json_with_a_cid_and_round_trips() {
    let v = hook("I P O, Claude is on the road", "Bb minor", 124.0, 2);
    assert_eq!(v.notes.len(), 8);
    assert_eq!(v.rights.license.id(), "CC0-1.0");
    let back = VocalLine::from_json(&JsonValue::parse(&v.canonical()).unwrap()).unwrap();
    assert_eq!(back, v);
    assert_eq!(back.cid(), v.cid());
    assert!(v.cid().starts_with('z'));
    // the same text, key and tempo make the same line
    assert_eq!(hook("I P O, Claude is on the road", "Bb minor", 124.0, 2).cid(), v.cid());
    // another seed, another tune, another CID
    let mut p = Params::new("I P O, Claude is on the road", Key::parse("Bb minor").unwrap(), 124.0, 2);
    p.seed = 5;
    assert_ne!(VocalLine::create(p).unwrap().0.cid(), v.cid());
}

#[test]
fn the_license_is_in_the_cid_and_a_fork_credits_its_parent() {
    let v = hook("buy the macs", "A minor", 118.0, 1);
    let mut by = v.clone();
    by.rights.license = crate::rights::License::CcBy4;
    by.rights.author = Some("did:key:zAlice".into());
    assert_ne!(by.cid(), v.cid());
    let (f, warn) = by.fork(None, Some("did:key:zBob"));
    assert!(warn.is_empty());
    assert_eq!(f.rights.forked_from.as_deref(), Some(by.cid().as_str()));
    assert_eq!(f.rights.credits.len(), 1);
    assert_eq!(f.name, format!("{} (remix)", by.name));
    assert_eq!(f.notes, by.notes);
    // private is never shared
    let mut p = v.clone();
    p.rights.visibility = crate::rights::Visibility::Private;
    assert!(p.share().is_err());
    assert!(v.share().is_ok());
}

#[test]
fn refusals() {
    let k = Key::parse("C major").unwrap();
    assert!(VocalLine::create(Params::new("   ", k, 120.0, 2)).is_err());
    assert!(VocalLine::create(Params::new("la", k, 5.0, 2)).is_err());
    assert!(VocalLine::create(Params::new("la", k, 120.0, 0)).is_err());
    assert!(VocalLine::create(Params::new(&"la ".repeat(40), k, 120.0, 2)).is_err());
    let mut p = Params::new("la", k, 120.0, 2);
    p.preset = "nope".into();
    assert!(VocalLine::create(p).is_err());
    let bad = JsonValue::parse(r#"{"type":"djrust.track"}"#).unwrap();
    assert!(VocalLine::from_json(&bad).is_err());
}

#[test]
fn unknown_words_are_sounded_out_and_reported() {
    let (v, w) = VocalLine::create(Params::new("zorbl the road", Key::parse("C major").unwrap(), 120.0, 1)).unwrap();
    assert!(w.iter().any(|m| m.contains("zorbl")), "{w:?}");
    assert!(!v.notes.is_empty());
}

#[test]
fn given_notes_are_sung_in_order_and_surplus_is_reported() {
    let k = Key::parse("A minor").unwrap();
    let mut p = Params::new("la la la", k, 100.0, 1);
    p.places = Some(vec![
        fit::Place { start: 0.0, dur: 1.0, midi: 57 },
        fit::Place { start: 2.0, dur: 1.0, midi: 60 },
    ]);
    let (v, w) = VocalLine::create(p).unwrap();
    assert_eq!(v.notes.len(), 2);
    assert_eq!((v.notes[1].start, v.notes[1].midi), (2.0, 60));
    assert!(w.iter().any(|m| m.contains("not sung")), "{w:?}");
}

#[test]
fn a_render_is_exactly_the_loop_long_audible_and_not_clipping() {
    for (lyrics, key, bpm) in [
        ("I P O, Claude is on the road", "Bb minor", 124.0),
        ("six million lines of proof", "Bb major", 130.0),
        ("street tracks stand strong, shh!", "E minor", 174.0),
    ] {
        let v = hook(lyrics, key, bpm, 2);
        let sr = 48_000.0;
        let a = v.render(sr);
        assert_eq!(a.len(), (8.0 * 60.0 / bpm * sr).round() as usize);
        let peak = a.iter().fold(0f32, |m, x| m.max(x.abs()));
        assert!(peak > 0.8 && peak < 0.9, "{peak}");
        assert!(a.iter().all(|x| x.is_finite()));
        // sound in every bar
        let bar = a.len() / 2;
        for b in 0..2 {
            let e: f64 = a[b * bar..(b + 1) * bar].iter().map(|x| (*x as f64).powi(2)).sum();
            assert!(e > 1.0, "bar {b} of {lyrics} is silent");
        }
        // deterministic
        assert_eq!(a, v.render(sr));
    }
}

#[test]
fn the_notes_sit_on_the_beat_grid_in_the_render() {
    // four quarter-note syllables: onsets at beats 0, 1, 2, 3 of the bar
    let v = hook("la la la la", "C major", 120.0, 1);
    let a = v.render(48_000.0);
    let beat = 24_000;
    // the energy just after each beat is larger than just before it
    for k in 1..4 {
        let at = k * beat;
        let before: f64 = a[at - 1200..at].iter().map(|x| (*x as f64).powi(2)).sum();
        let after: f64 = a[at + 1200..at + 2400].iter().map(|x| (*x as f64).powi(2)).sum();
        assert!(after > before, "beat {k}: {before} {after}");
    }
}
