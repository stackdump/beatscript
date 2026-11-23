//! Radio locality by listen numbers (docs/btw/radio.md "Knobs"): a small knob turn is a small musical change. The tracks are
//! rendered and measured as the MCP `listen` tool measures them (the tempo and key heard, the loudness of the master and of
//! each deck, the loudness bar by bar), and a distance is taken from those numbers alone:
//!
//! - a neighbour (one notch of tempo, key, forms or seed) plays, at the same position, a track measurably closer to this
//!   station's than a far station does (another genre, tempo, key, seed and rotation);
//! - the seed walk stays near: two tracks one step apart on the walk are closer than a track of the far station;
//! - the segue is near both tracks it joins.
//!
//! Each track is listened to over its first 16 bars, with no part renders (`budget_seconds: 0`), so the test is a few seconds
//! of rendering on a laptop. The numbers are printed (`--nocapture`) so a change in the generator shows where it moved.

use std::collections::BTreeMap;

use djengine::json::JsonValue;
use djengine::key::Key;

use crate::listen::{listen, ListenOptions};

const BASE: &str = "tempo 128\nkey am\nstation {\n  from gen@2 techno seed=42\n  forms seeded passacaglia fugue\n}\n";
const FAR: &str = "tempo 174\nkey f#\nstation {\n  from gen@2 dnb seed=9001\n  forms cross rondo canon\n}\n";

/// What a listen heard, as the numbers the distance reads.
#[derive(Clone, Debug)]
struct Heard {
    bpm: f64,
    key: Option<Key>,
    lufs: f64,
    decks: BTreeMap<String, f64>,
    bars: Vec<f64>,
}

fn num(v: Option<&JsonValue>) -> Option<f64> {
    v.and_then(JsonValue::as_f64)
}

fn heard(text: &str, cache: &mut BTreeMap<String, Heard>) -> Heard {
    if let Some(h) = cache.get(text) {
        return h.clone();
    }
    btwapi::generate::install();
    let lo = ListenOptions { section: None, bars: Some(16), max_bars: 16, budget_seconds: 0.0, sr: crate::listen::SR };
    let v = listen(text, &lo).unwrap_or_else(|e| panic!("listen: {e:?}\n{text}"));
    let det = v.get("detected").expect("detected");
    let bpm = num(det.get("bpm")).or_else(|| num(det.get("declared_bpm"))).expect("a tempo");
    let tonal = det.get("tonal").and_then(JsonValue::as_bool).unwrap_or(false);
    let key_word = if tonal { det.get("key").and_then(JsonValue::as_str) } else { None }.or_else(|| det.get("declared_key").and_then(JsonValue::as_str));
    let key = key_word.and_then(Key::parse);
    let lufs = num(v.get("master").and_then(|m| m.get("lufs"))).unwrap_or(-70.0);
    let mut decks = BTreeMap::new();
    if let Some(JsonValue::Object(d)) = v.get("decks") {
        for (name, x) in d.iter() {
            decks.insert(name.to_string(), num(x.get("lufs")).unwrap_or(-70.0).max(-70.0));
        }
    }
    let bars = v.get("bar_lufs").and_then(JsonValue::as_array).map(|a| a.iter().map(|x| x.as_f64().unwrap_or(-70.0).max(-70.0)).collect()).unwrap_or_default();
    let h = Heard { bpm, key, lufs: lufs.max(-70.0), decks, bars };
    cache.insert(text.to_string(), h.clone());
    h
}

/// The distance of two tracks by what was heard, 0 (the same numbers) to 1: the tempo (a half or double time is near), the
/// key (by the Camelot wheel's compatibility), the loudness of the master and of each deck, and the loudness bar by bar.
fn distance(a: &Heard, b: &Heard) -> f64 {
    let ratio = [0.5, 1.0, 2.0].iter().map(|m| (m * a.bpm / b.bpm).ln().abs()).fold(f64::INFINITY, f64::min);
    let c_t = (ratio / 1.08f64.ln()).min(1.0);
    let c_k = match (a.key, b.key) {
        (Some(x), Some(y)) => 1.0 - djengine::key::compat(x, y).score(),
        _ => 0.5,
    };
    let c_l = ((a.lufs - b.lufs).abs() / 6.0).min(1.0);
    let names: Vec<&String> = a.decks.keys().chain(b.decks.keys()).collect();
    let c_d = if names.is_empty() { 0.0 } else { names.iter().map(|n| ((a.decks.get(*n).copied().unwrap_or(-70.0) - b.decks.get(*n).copied().unwrap_or(-70.0)).abs() / 6.0).min(1.0)).sum::<f64>() / names.len() as f64 };
    let len = a.bars.len().max(b.bars.len()).max(1);
    let at = |v: &Vec<f64>, i: usize| v.get(i).copied().unwrap_or(-70.0);
    let c_e = ((0..len).map(|i| (at(&a.bars, i) - at(&b.bars, i)).abs()).sum::<f64>() / len as f64 / 12.0).min(1.0);
    (2.0 * c_t + 2.0 * c_k + c_l + c_d + c_e) / 7.0
}

fn station(src: &str) -> djlive::station::Station {
    djlive::station::read(src).expect("the station reads").with_defaults().expect("its tempo and key")
}

#[test]
fn a_small_knob_turn_is_a_small_change_by_listen_numbers() {
    let mut cache = BTreeMap::new();
    let base = station(BASE);
    let far = station(FAR);
    let mut near_all = Vec::new();
    let mut far_all = Vec::new();
    let mut walk_all = Vec::new();
    for n in [1u64, 2, 3] {
        let here = heard(&base.track(n).unwrap().text, &mut cache);
        let there = heard(&far.track(n).unwrap().text, &mut cache);
        let d_far = distance(&here, &there);
        far_all.push(d_far);
        for (knob, v) in [("tempo", "+1"), ("key", "+1"), ("forms", "+1"), ("seed", "+1")] {
            let nb = base.retune(knob, v).unwrap();
            let t = nb.track(n).unwrap();
            assert_eq!(t.n, n, "{knob}: retuning keeps the position");
            let d = distance(&here, &heard(&t.text, &mut cache));
            println!("n={n} {knob}{v}: d={d:.3} (far {d_far:.3})");
            assert!(d < d_far, "n={n}: a notch of {knob} ({d:.3}) is nearer than the far station ({d_far:.3})");
            near_all.push(d);
        }
        // the walk: the next track of this station
        let next = heard(&base.track(n + 1).unwrap().text, &mut cache);
        let d_walk = distance(&here, &next);
        println!("n={n} walk to {}: d={d_walk:.3}", n + 1);
        assert!(d_walk < d_far, "n={n}: a step of the walk ({d_walk:.3}) is nearer than the far station ({d_far:.3})");
        walk_all.push(d_walk);
        // the segue from n to n+1 is near both
        let sg = heard(&base.segue(n).unwrap().text, &mut cache);
        let (ds_a, ds_b) = (distance(&here, &sg), distance(&next, &sg));
        println!("n={n} segue: d={ds_a:.3} / {ds_b:.3}");
        assert!(ds_a < d_far && ds_b < d_far, "n={n}: the segue is near both tracks it joins");
    }
    let mean = |v: &Vec<f64>| v.iter().sum::<f64>() / v.len() as f64;
    let (near, far_m, walk) = (mean(&near_all), mean(&far_all), mean(&walk_all));
    println!("mean: neighbours {near:.3}, walk {walk:.3}, far {far_m:.3}");
    assert!(near < 0.6 * far_m, "the neighbours ({near:.3}) are well nearer than the far station ({far_m:.3})");
    assert!(walk < 0.75 * far_m, "the walk ({walk:.3}) stays well nearer than the far station ({far_m:.3})");
    // the tempo and key notches move only their own numbers: a tempo notch is two BPM, a key notch one step of the wheel
    let here = heard(&base.track(2).unwrap().text, &mut cache);
    let t = heard(&base.retune("tempo", "+1").unwrap().track(2).unwrap().text, &mut cache);
    assert!((t.bpm - here.bpm - 2.0).abs() < 1.0 || (t.bpm / here.bpm - 130.0 / 128.0).abs() < 0.02, "a tempo notch is heard as two BPM: {} -> {}", here.bpm, t.bpm);
}
