//! `djctl render --mute / --solo / --options / --cohesion` (`RenderRequest`): the parsing, the
//! plain request staying the render it was, and a soloed bass being a bass stem. Port of
//! `RenderStemTests.swift`; each test names the Swift one. No golden: the Swift suite's
//! literals are all there is to hold it to (the type does no arithmetic).

use djengine::compose_options::Invalid;
use djengine::json::JsonValue;
use djengine::musicality::audio_metrics::OfflineRender;
use djengine::render_request::RenderRequest;
use djengine::wave::{RoleFilter, WaveOptions};
use djengine::TrackSpec;

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

// RenderStemTests.emptyFlagsAreNoFilter
#[test]
fn empty_flags_are_no_filter() {
    assert_eq!(RenderRequest::filter(None, None), None);
    assert_eq!(RenderRequest::filter(Some(""), None), None);
    assert_eq!(
        RenderRequest::new(
            "techno",
            42.0,
            Some("standard"),
            None,
            None,
            Some(RoleFilter::default())
        )
        .filter,
        None
    );
    // blanks and stray commas name nothing either
    assert_eq!(RenderRequest::filter(Some(" , ,"), None), None);
}

// RenderStemTests.muteAndSoloListsParse
#[test]
fn mute_and_solo_lists_parse() {
    let f = RenderRequest::filter(Some("hats, kick"), Some("bass"));
    assert_eq!(
        f,
        Some(RoleFilter {
            mute: strs(&["hats", "kick"]),
            solo: Some(strs(&["bass"])),
        })
    );
    assert_eq!(
        RenderRequest::filter(None, Some("drums,bass"))
            .unwrap()
            .solo,
        Some(strs(&["drums", "bass"]))
    );
    // Swift's lists are sets: a name given twice is one, and tabs trim like spaces
    let g = RenderRequest::filter(Some("kick,\tkick ,hats"), None).unwrap();
    assert_eq!(g.mute, strs(&["kick", "hats"]));
    // `--solo ""` names no role: the filter keeps none (Swift: solo is a non-nil empty set)
    let none = RenderRequest::filter(None, Some("")).unwrap();
    assert_eq!(none.solo, Some(vec![]));
    assert!(!none.is_empty());
}

// RenderStemTests.optionsMustBeAnObject
#[test]
fn options_must_be_an_object() {
    assert_eq!(RenderRequest::options(None), Ok(None));
    assert!(RenderRequest::options(Some("[1, 2]")).is_err());
    assert!(RenderRequest::options(Some("{not json")).is_err());
    let o = RenderRequest::options(Some(r#"{"hook": true}"#))
        .unwrap()
        .unwrap();
    assert_eq!(o.get("hook").and_then(JsonValue::as_bool), Some(true));
}

// RenderStemTests.unknownOptionIsRefused
#[test]
fn unknown_option_is_refused() {
    let r = RenderRequest::new(
        "techno",
        42.0,
        Some("standard"),
        None,
        RenderRequest::options(Some(r#"{"nope": 1}"#)).unwrap(),
        None,
    );
    let e: Invalid = r.spec().unwrap_err();
    assert!(e.description.contains("unknown option nope"), "{e}");
}

// RenderStemTests.plainRequestIsThePlainSpec
#[test]
fn plain_request_is_the_plain_spec() {
    let r = RenderRequest::plain("techno", 42.0);
    assert_eq!(
        r.spec().unwrap(),
        TrackSpec::new("techno", 42.0, Some("standard"))
    );
    assert_eq!(r.applied(&WaveOptions::extended()).role_filter, None);
    assert!(r.report_keys(&r.spec().unwrap()).is_empty());
    // the defaults Swift's init has: a standard song, the spec's own cohesion
    assert_eq!(r.structure.as_deref(), Some("standard"));
    assert_eq!(r.cohesion, "v2");
}

// RenderStemTests.cohesionAndOptionsReachTheSpec
#[test]
fn cohesion_and_options_reach_the_spec() {
    let r = RenderRequest::new(
        "techno",
        42.0,
        Some("standard"),
        Some("v1"),
        RenderRequest::options(Some(r#"{"ghostNotes": 0.5}"#)).unwrap(),
        None,
    );
    let spec = r.spec().unwrap();
    assert_eq!(spec.cohesion, "v1");
    assert_eq!(
        spec.extra.get("ghost-notes").and_then(JsonValue::as_f64),
        Some(0.5)
    );
    let keys = r.report_keys(&spec);
    assert_eq!(
        keys.get("options")
            .and_then(|o| o.get("ghostNotes"))
            .and_then(JsonValue::as_f64),
        Some(0.5)
    );
    assert_eq!(keys.get("cohesion").and_then(JsonValue::as_str), Some("v1"));
}

// RenderStemTests.filterReachesEveryRenderAndTheReport
#[test]
fn filter_reaches_every_render_and_the_report() {
    let f = RenderRequest::filter(Some("hats"), Some("bass"));
    let r = RenderRequest::new("techno", 42.0, Some("standard"), None, None, f.clone());
    assert_eq!(r.applied(&WaveOptions::extended()).role_filter, f);
    let keys = r.report_keys(&r.spec().unwrap());
    let filter = keys.get("filter").unwrap();
    assert_eq!(
        filter.get("solo").and_then(JsonValue::strings),
        Some(strs(&["bass"]))
    );
    assert_eq!(
        filter.get("mute").and_then(JsonValue::strings),
        Some(strs(&["hats"]))
    );
}

/// `request` rendered offline for `seconds`, as `RenderStemTests.render` does (through the
/// wave runner this port has: no samplers, no master chain).
fn render(request: &RenderRequest, seconds: f64) -> OfflineRender {
    let spec = request.spec().unwrap();
    let project =
        djengine::musicality::apply(&spec.project(), &spec, djengine::musicality::Fixes::ALL);
    let opts = request.applied(&WaveOptions::extended());
    OfflineRender::new(&project, &opts, 48_000.0, seconds)
}

// RenderStemTests.noFilterRendersTheSameBytes
#[test]
fn no_filter_renders_the_same_bytes() {
    let plain = render(&RenderRequest::plain("techno", 42.0), 4.0);
    let empty = render(
        &RenderRequest::new(
            "techno",
            42.0,
            Some("standard"),
            None,
            None,
            RenderRequest::filter(Some(""), None),
        ),
        4.0,
    );
    assert_eq!(plain.left, empty.left);
    assert_eq!(plain.right, empty.right);
    assert!(plain.left.iter().any(|x| *x != 0.0));
}

// RenderStemTests.soloBassIsABassStem
#[test]
fn solo_bass_is_a_bass_stem() {
    let r = RenderRequest::new(
        "techno",
        42.0,
        Some("standard"),
        None,
        None,
        RenderRequest::filter(None, Some("bass")),
    );
    let all = render(&RenderRequest::plain("techno", 42.0), 10.0).metrics();
    let m = render(&r, 10.0).metrics();
    let band = |m: &djengine::musicality::audio_metrics::AudioMetrics, k: &str| {
        m.bands.get(k).copied().unwrap_or(0.0)
    };
    let low = band(&m, "sub") + band(&m, "low");
    assert!(m.rms_db > -60.0, "the bass stem plays: {}", m.rms_db);
    // Swift asserts >= 0.75: its soloed techno 42 bass measures 0.787 through the deck's chain
    // (`djhost/tests/deck_acceptance.rs` holds that bound, rendered through `DeckCore` and the master
    // chain). This render is the bare runner: every fix the runner plays (`WaveOptions::extended`
    // holds them all) and the musicality pass, without the master chain or the limiter, and it
    // measures 0.718 (0.863 with the fixes off, 0.573 without the pass).
    assert!(low >= 0.6, "sub + low = {low}");
    // and it is a stem: quieter than the whole track
    assert!(m.rms_db < all.rms_db, "{} vs {}", m.rms_db, all.rms_db);
}
