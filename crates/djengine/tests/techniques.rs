//! The opt-in composition techniques (gen/techniques.rs, compose_options.rs), measured on
//! what the composed net actually plays, as Swift's `ComposeTechniqueTests` do: the graph
//! is stepped through the whole song.
//!
//! No oracle: beats does not have these. The assertions are the Swift suite's literal
//! expectations. The stepper below is a minimal Petri-net executor for the nets `compose`
//! writes (one token each: the first enabled transition fires every tick), with the
//! mute / unmute / activate-slot control actions; the real wave engine lives in `wave`.

use djengine::cid;
use djengine::compose_options;
use djengine::gen::techniques::{
    auto_layer_budget, build_hook, hook_mode, ChordSnap, ComposeTechniques,
};
use djengine::gen::theme::{
    build_track_theme, chord_at, clamp_degree, cycle_steps, degree_to_midi, motif_mode,
};
use djengine::gen::BeatsTables;
use djengine::json::{JsonObject, JsonValue};
use djengine::pflow::{NetBundle, Project};
use djengine::TrackSpec;
use std::collections::{BTreeMap, BTreeSet};

fn spec(options: &[(&str, JsonValue)], genre: &str, seed: f64) -> TrackSpec {
    let o = JsonObject::from_pairs(options.iter().map(|(k, v)| (*k, v.clone())));
    compose_options::apply(&o, &TrackSpec::new(genre, seed, Some("standard"))).unwrap()
}

fn edm(options: &[(&str, JsonValue)]) -> TrackSpec {
    spec(options, "edm", 4242.0)
}

#[derive(Clone, Debug)]
struct Heard {
    step: i64,
    role: String,
    note: i64,
}

struct Runner<'a> {
    nb: &'a NetBundle,
    marking: Vec<f64>,
    places: BTreeMap<&'a str, usize>,
    /// per transition (in order): inputs (place, weight, inhibit), outputs (place, weight)
    ins: Vec<Vec<(usize, f64, bool)>>,
    outs: Vec<Vec<(usize, f64)>>,
    tids: Vec<&'a str>,
}

fn weight(w: &[f64]) -> f64 {
    if w.is_empty() {
        1.0
    } else {
        w.iter().sum()
    }
}

impl<'a> Runner<'a> {
    fn new(nb: &'a NetBundle) -> Self {
        let mut places = BTreeMap::new();
        let mut marking = Vec::new();
        for (id, p) in nb.places.iter() {
            places.insert(id, marking.len());
            marking.push(p.initial.iter().sum());
        }
        let tids: Vec<&str> = nb.transitions.keys().iter().map(String::as_str).collect();
        let mut ins = vec![vec![]; tids.len()];
        let mut outs = vec![vec![]; tids.len()];
        for a in &nb.arcs {
            if let (Some(&p), Some(t)) = (
                places.get(a.source.as_str()),
                tids.iter().position(|t| *t == a.target),
            ) {
                ins[t].push((p, weight(&a.weight), a.inhibit));
            }
            if let (Some(t), Some(&p)) = (
                tids.iter().position(|t| *t == a.source),
                places.get(a.target.as_str()),
            ) {
                outs[t].push((p, weight(&a.weight)));
            }
        }
        Runner {
            nb,
            marking,
            places,
            ins,
            outs,
            tids,
        }
    }

    /// Fire the first enabled transition; its id, if any.
    fn step(&mut self) -> Option<&'a str> {
        let _ = &self.places;
        for t in 0..self.tids.len() {
            if self.ins[t].is_empty() {
                continue;
            }
            let ok = self.ins[t].iter().all(|(p, w, inh)| {
                if *inh {
                    self.marking[*p] < *w
                } else {
                    self.marking[*p] >= *w
                }
            });
            if !ok {
                continue;
            }
            for (p, w, inh) in self.ins[t].clone() {
                if !inh {
                    self.marking[p] = (self.marking[p] - w).max(0.0);
                }
            }
            for (p, w) in self.outs[t].clone() {
                self.marking[p] += w;
            }
            return Some(self.tids[t]);
        }
        None
    }
}

/// Every audible note of the song: (step, role, note), role = riff group or net id.
fn play(p: &Project) -> Vec<Heard> {
    let total: i64 = p.structure.iter().map(|s| s.steps).sum();
    let ids: Vec<String> = p.nets.keys().to_vec();
    let mut runners: Vec<Runner> = ids
        .iter()
        .map(|id| Runner::new(p.nets.get(id).unwrap()))
        .collect();
    let mut muted: BTreeSet<String> = p.initial_mutes.iter().cloned().collect();
    let mut out = Vec::new();
    for tick in 0..total {
        for (n, id) in ids.iter().enumerate() {
            let nb = runners[n].nb;
            let Some(t) = runners[n].step() else { continue };
            if nb.role == "control" {
                if let Some(c) = nb.control_bindings.get(t) {
                    match c.action.as_str() {
                        "mute-track" => {
                            muted.insert(c.target_net.clone());
                        }
                        "unmute-track" => {
                            muted.remove(&c.target_net);
                        }
                        "activate-slot" => {
                            let group = p.nets.get(&c.target_net).map(|n| n.riff_group.clone());
                            if let Some(g) = group.filter(|g| !g.is_empty()) {
                                for (oid, onb) in p.nets.iter() {
                                    if onb.riff_group == g {
                                        muted.insert(oid.to_string());
                                    }
                                }
                            }
                            muted.remove(&c.target_net);
                        }
                        _ => {}
                    }
                }
            } else if let Some(b) = nb.bindings.get(t) {
                if !muted.contains(id) {
                    let role = if nb.riff_group.is_empty() {
                        id.clone()
                    } else {
                        nb.riff_group.clone()
                    };
                    out.push(Heard {
                        step: tick,
                        role,
                        note: b.note,
                    });
                }
            }
        }
    }
    out
}

/// Structure changes land late: beats' accumulator nets spend a tick per event, so by the
/// end of a standard song a slot switch is up to a bar behind its boundary. Notes are only
/// attributed to a section past this margin.
const LAG: i64 = 24;

fn sections(p: &Project) -> Vec<(String, i64, i64)> {
    let mut pos = 0;
    p.structure
        .iter()
        .map(|s| {
            let r = (s.name.clone(), pos, s.steps);
            pos += s.steps;
            r
        })
        .collect()
}

const MELODIC: [&str; 4] = ["bass", "melody", "harmony", "arp"];

fn pcs(genre: &str, seed: f64, step: i64) -> BTreeSet<i64> {
    let g = &BeatsTables::shared().genres[genre];
    let plan = build_track_theme(genre, seed).plan;
    let key = g.scale(g.root_note);
    chord_at(&plan, step)
        .tones
        .iter()
        .map(|t| degree_to_midi(clamp_degree(*t), &key) % 12)
        .collect()
}

#[test]
fn options_off_are_the_beats_track() {
    let plain = TrackSpec::new("edm", 4242.0, Some("standard"));
    let off = edm(&[
        ("hook", JsonValue::Bool(false)),
        ("chordSnap", "off".into()),
        ("layerBudget", "off".into()),
        ("bassFollow", JsonValue::Bool(false)),
        ("keyLock", JsonValue::Bool(false)),
    ]);
    assert_eq!(off, plain);
    assert_eq!(
        cid::share_cid(&off.payload()),
        cid::share_cid(&plain.payload())
    );
    // compose ignores the off values too, even unnormalised
    let mut ov = plain.overrides().as_object().unwrap().clone();
    ov.set("hook", JsonValue::Bool(false));
    ov.set("chordSnap", "off".into());
    assert_eq!(
        djengine::compose("edm", &ov.into()).model_cid(),
        plain.project().model_cid()
    );
}

#[test]
fn validation_rejects_bad_values() {
    let base = TrackSpec::new("edm", 1.0, None);
    let cases: Vec<(&str, JsonValue)> = vec![
        ("hook", "yes".into()),
        ("hookBars", JsonValue::int(3)),
        ("chordSnap", "some".into()),
        ("layerBudget", JsonValue::int(9)),
        ("layerBudget", 1.5.into()),
        ("melodyDensity", JsonValue::int(0)),
        ("ghostNotes", 2.0.into()),
        ("humanize", (-1.0).into()),
        ("nope", JsonValue::Bool(true)),
        ("vox", "opera".into()),
        ("vox", JsonValue::int(1)),
    ];
    for (k, v) in cases {
        let o = JsonObject::from_pairs([(k, v)]);
        assert!(compose_options::apply(&o, &base).is_err(), "{k}");
    }
    // ghostNotes is stored under beats' own key, and reads back under its option name
    let s = compose_options::apply(&JsonObject::from_pairs([("ghostNotes", 0.1.into())]), &base)
        .unwrap();
    assert_eq!(s.extra.get("ghost-notes"), Some(&JsonValue::Number(0.1)));
    assert_eq!(
        compose_options::describe(&s).get("ghostNotes"),
        Some(&JsonValue::Number(0.1))
    );
    // and survives the share payload round trip
    assert_eq!(TrackSpec::from_payload(&s.payload()), Some(s));
}

#[test]
fn technique_flags_parse_like_swift() {
    let ov = JsonValue::parse(
        r#"{"hook":true,"hookBars":1,"chordSnap":"all","chordSnapArp":true,"layerBudget":"auto","bassFollow":true,"keyLock":true,"melodyDensity":0.4}"#,
    )
    .unwrap();
    let t = ComposeTechniques::from_json(&ov);
    assert!(t.hook && t.chord_snap_arp && t.bass_follow && t.key_lock && t.needs_context());
    assert_eq!(
        (t.hook_bars, t.chord_snap, t.layer_budget, t.melody_density),
        (1, ChordSnap::All, Some(0), Some(0.4))
    );
    let t = ComposeTechniques::from_json(
        &JsonValue::parse(r#"{"hookBars":3,"chordSnap":"x","layerBudget":0.5}"#).unwrap(),
    );
    assert_eq!(
        (t.hook_bars, t.chord_snap, t.layer_budget),
        (2, ChordSnap::Off, None)
    );
    assert!(!t.needs_context());
    assert_eq!(
        ComposeTechniques::from_json(&JsonValue::parse(r#"{"layerBudget":3}"#).unwrap())
            .layer_budget,
        Some(3)
    );
}

#[test]
fn hook_plays_in_every_melody_section() {
    for bars in [1, 2] {
        let s = edm(&[
            ("hook", JsonValue::Bool(true)),
            ("hookBars", JsonValue::int(bars)),
        ]);
        let p = s.project();
        let hook = build_hook(
            "edm",
            4242.0,
            bars,
            BeatsTables::shared().genres["edm"].melody_density,
        );
        let heard: Vec<Heard> = play(&p)
            .into_iter()
            .filter(|h| h.role == "melody")
            .collect();
        let mut melody_sections = 0;
        let mut active = 0;
        for (name, start, steps) in sections(&p) {
            let mode = hook_mode(&name);
            let notes: Vec<&Heard> = heard
                .iter()
                .filter(|h| h.step >= start + LAG && h.step < start + steps)
                .collect();
            if notes.is_empty() {
                continue;
            }
            active += 1;
            melody_sections += 1;
            let stretch = if mode == motif_mode::AUGMENTED { 2 } else { 1 };
            for n in &notes {
                // on the hook's rhythm (fragment: its first half only)
                assert_eq!(n.step % stretch, 0, "{name}");
                let h = ((n.step / stretch) as usize) % hook.length();
                assert!(hook.mask[h], "{name} step {} is off the hook", n.step);
                if mode == motif_mode::FRAGMENT {
                    assert!(h < hook.length() / 2);
                }
                // and on the sounding chord
                assert!(
                    pcs("edm", 4242.0, n.step).contains(&(n.note % 12)),
                    "{name} step {}",
                    n.step
                );
            }
            // same contour: within a bar, each move goes the hook's way (or holds)
            if mode != motif_mode::PLAY && mode != motif_mode::TRANSPOSED {
                continue;
            }
            let bars_seen: BTreeSet<i64> = notes.iter().map(|n| n.step / 16).collect();
            for bar in bars_seen {
                let mut in_bar: Vec<&&Heard> =
                    notes.iter().filter(|n| n.step / 16 == bar).collect();
                in_bar.sort_by_key(|n| n.step);
                for w in in_bar.windows(2) {
                    let want = hook.target[(w[1].step as usize) % hook.length()]
                        - hook.target[(w[0].step as usize) % hook.length()];
                    let got = w[1].note - w[0].note;
                    assert!(got == 0 || (got > 0) == (want > 0), "{name} bar {bar}");
                }
            }
        }
        // edm standard: every section with melody in it carries the hook
        assert!(melody_sections >= 3, "bars {bars}: {melody_sections}");
        assert_eq!(melody_sections, active);
    }
}

#[test]
fn chord_snap_leaves_no_non_chord_tones_on_its_beats() {
    for mode in ["strong", "all"] {
        let every = if mode == "strong" { 8 } else { 4 };
        for (genre, cohesion) in [("edm", "v2"), ("techno", "v1"), ("jazz", "v1")] {
            let mut s = spec(
                &[
                    ("chordSnap", mode.into()),
                    ("chordSnapArp", JsonValue::Bool(true)),
                ],
                genre,
                4242.0,
            );
            s.cohesion = cohesion.into();
            let heard: Vec<Heard> = play(&s.project())
                .into_iter()
                .filter(|h| (h.role == "melody" || h.role == "arp") && h.step % every == 0)
                .collect();
            assert!(!heard.is_empty(), "{genre}");
            for n in heard {
                assert!(
                    pcs(genre, 4242.0, n.step).contains(&(n.note % 12)),
                    "{genre} {} step {} note {}",
                    n.role,
                    n.step,
                    n.note
                );
            }
        }
    }
}

#[test]
fn layer_budget_caps_melodic_roles_per_section() {
    let roles_in = |p: &Project, heard: &[Heard], start: i64, steps: i64| {
        let _ = p;
        heard
            .iter()
            .filter(|h| {
                MELODIC.contains(&h.role.as_str())
                    && h.step >= start + LAG
                    && h.step < start + steps
            })
            .map(|h| h.role.clone())
            .collect::<BTreeSet<_>>()
    };
    let max_roles = |s: &TrackSpec, cap: &dyn Fn(&str) -> i64| {
        let p = s.project();
        let heard = play(&p);
        for (name, start, steps) in sections(&p) {
            let roles = roles_in(&p, &heard, start, steps);
            assert!(
                roles.len() as i64 <= cap(&name),
                "{name}: {roles:?} over {}",
                cap(&name)
            );
        }
    };
    // without a budget the drop stacks four
    let plain = TrackSpec::new("edm", 4242.0, Some("standard"));
    let pp = plain.project();
    let p0 = play(&pp);
    let drop = sections(&pp).into_iter().find(|s| s.0 == "drop").unwrap();
    assert_eq!(roles_in(&pp, &p0, drop.1, drop.2).len(), 4);

    max_roles(&edm(&[("layerBudget", "auto".into())]), &|n| {
        auto_layer_budget(n)
    });
    max_roles(&edm(&[("layerBudget", JsonValue::int(1))]), &|_| 1);
    // the budget keeps bass first, then melody
    let one = play(&edm(&[("layerBudget", JsonValue::int(1))]).project());
    assert!(!one.iter().any(|h| h.role == "arp" || h.role == "harmony"));
    // and leaves the nets in place
    assert_eq!(
        edm(&[("layerBudget", JsonValue::int(1))])
            .project()
            .nets
            .len(),
        pp.nets.len()
    );
}

#[test]
fn bass_follow_repeats_one_pattern_per_cycle() {
    for (genre, cohesion) in [("edm", "v2"), ("techno", "v1")] {
        let mut s = spec(&[("bassFollow", JsonValue::Bool(true))], genre, 4242.0);
        s.cohesion = cohesion.into();
        let p = s.project();
        let cycle = cycle_steps(&build_track_theme(genre, 4242.0).plan);
        let bass: Vec<Heard> = play(&p).into_iter().filter(|h| h.role == "bass").collect();
        assert!(!bass.is_empty());
        // wherever it plays, the bass at step s is the bass at s + one cycle
        let by_step: BTreeMap<i64, i64> = bass.iter().map(|b| (b.step, b.note)).collect();
        let mut compared = 0;
        for (step, note) in &by_step {
            if let Some(later) = by_step.get(&(step + cycle)) {
                assert_eq!(later, note, "{genre} step {step}");
                compared += 1;
            }
        }
        assert!(compared > 16, "{genre}: {compared}");
    }
}

#[test]
fn key_lock_keeps_tension_phrases_in_key() {
    let g = &BeatsTables::shared().genres["edm"];
    let key: BTreeSet<i64> = g.scale(g.root_note).iter().map(|n| n % 12).collect();
    let out_of_key = |s: &TrackSpec| {
        play(&s.project())
            .iter()
            .filter(|h| (h.role == "melody" || h.role == "arp") && !key.contains(&(h.note % 12)))
            .count()
    };
    // beats' tension curve moves B / C phrases out of the key...
    assert!(out_of_key(&TrackSpec::new("edm", 4242.0, Some("standard"))) > 0);
    // ...keyLock keeps them in it
    assert_eq!(out_of_key(&edm(&[("keyLock", JsonValue::Bool(true))])), 0);
}

#[test]
fn determinism() {
    let all: Vec<(&str, JsonValue)> = vec![
        ("hook", JsonValue::Bool(true)),
        ("chordSnap", "strong".into()),
        ("layerBudget", "auto".into()),
        ("melodyDensity", 0.35.into()),
        ("ghostNotes", 0.1.into()),
        ("bassFollow", JsonValue::Bool(true)),
    ];
    let a = edm(&all);
    let b = edm(&all);
    assert_eq!(cid::share_cid(&a.payload()), cid::share_cid(&b.payload()));
    assert_eq!(a.project().model_cid(), b.project().model_cid());
    let sets: Vec<Vec<(&str, JsonValue)>> = vec![
        vec![],
        vec![("hook", JsonValue::Bool(true))],
        vec![("chordSnap", "strong".into())],
        vec![
            ("hook", JsonValue::Bool(true)),
            ("chordSnap", "strong".into()),
            ("layerBudget", "auto".into()),
        ],
        all,
    ];
    let (mut seen, mut models) = (BTreeSet::new(), BTreeSet::new());
    for o in &sets {
        let s = edm(o);
        seen.insert(cid::share_cid(&s.payload()));
        models.insert(s.project().model_cid());
    }
    assert_eq!(seen.len(), 5);
    assert_eq!(models.len(), 5);
    // (hook + chordSnap is the hook's own model: every hook note is already a chord tone)
    assert_eq!(
        edm(&[
            ("hook", JsonValue::Bool(true)),
            ("chordSnap", "strong".into())
        ])
        .project()
        .model_cid(),
        edm(&[("hook", JsonValue::Bool(true))])
            .project()
            .model_cid()
    );
}

#[test]
fn density_and_humanize_overrides_reach_the_project() {
    let mut plain = TrackSpec::new("techno", 7.0, Some("standard"));
    // v1: the markov melody and bass read the densities (a v2 motif cell ignores them)
    plain.cohesion = "v1".into();
    let mut humane = plain.clone();
    humane.extra.set("humanize", 30.0.into());
    assert_eq!(humane.project().humanize, 30.0);
    assert_eq!(
        plain.project().humanize,
        BeatsTables::shared().genres["techno"].humanize
    );
    // the numeric overrides change the track, deterministically
    for key in ["melodyDensity", "bassDensity"] {
        let mut s = plain.clone();
        s.extra.set(key, 0.2.into());
        let (a, b) = (s.project().model_cid(), s.project().model_cid());
        assert_eq!(a, b);
        assert_ne!(a, plain.project().model_cid(), "{key}");
    }
}

/// `vox`: the sung part (djrust gen::vox) rides in the spec, reaches compose, and off is the
/// plain spec.
#[test]
fn vox_is_a_compose_option() {
    let base = TrackSpec::new("trance", 4.0, None);
    let s = compose_options::apply(&JsonObject::from_pairs([("vox", "choir".into())]), &base)
        .unwrap();
    assert_eq!(s.extra.get("vox"), Some(&JsonValue::str("choir")));
    assert!(s.project().nets.get("vox").is_some());
    let t = compose_options::apply(&JsonObject::from_pairs([("vox", true.into())]), &base).unwrap();
    assert_eq!(t.extra.get("vox"), Some(&JsonValue::str("auto")));
    let off = compose_options::apply(&JsonObject::from_pairs([("vox", "off".into())]), &base)
        .unwrap();
    assert_eq!(off, base);
    assert!(base.project().nets.get("vox").is_none());
}
