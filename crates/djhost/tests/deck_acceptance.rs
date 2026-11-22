//! The Swift acceptance tests that need the track played as a deck plays it (every fix, the
//! master chain, the limiter): `LoudnessSpreadTests` (24 songs there; the 12 synth ones here),
//! `SpectralSlopeTests` / `HFTiltTests.everyGenreFallsWithNoBump`, `RenderStemTests.soloBassIsABassStem`
//! (the deck-chain half: the bare-runner number in `render_request.rs` is not Swift's) and
//! `PaletteTests.theSynthPaletteIsTheSynthRender`. Form (a): each test names the Swift one.
//!
//! A track is composed through the engine's own `Preparer` (so it is what `PreparedTrack` makes of
//! the deck's options: the pass, the Hits, the genre's settings, the measured gain and trims) and
//! rendered through a [`DeckCore`] on the app's options (`OfflineRender(track:source:seconds:)`
//! over a `WaveDeckSource`). The sampled half of `LoudnessSpreadTests` and `HFTiltTests` (a deck's
//! own bank) is not run here: it needs a SoundFont no test may assume (see `deck_sampled.rs`).

use std::sync::Arc;

use djengine::deck::level_match::LevelMatch;
use djengine::musicality::audio_metrics::AudioMetrics;
use djengine::musicality::loudness::IntegratedLoudness;
use djengine::musicality::mix_targets::MixTargets;
use djengine::musicality::spectrum::Spectrum;
use djengine::render_request::RenderRequest;
use djengine::wave::{Master, RoleFilter, WaveOptions};
use djengine::TrackSpec;
use djhost::deck::{engine, Command, DeckCore, DeckMirror, EngineConfig, PreparedTrack, Preparer};

/// A rendered song: both channels, and what `prepare` measured for it.
struct Song {
    left: Vec<f32>,
    right: Vec<f32>,
    rate: f64,
    gain: f64,
}

impl Song {
    fn metrics(&self) -> AudioMetrics {
        AudioMetrics::new(&self.left, &self.right, self.rate)
    }
}

/// `OfflineRender(track:source:seconds:)`: `track` played from its start on a deck whose options
/// are `deck_opts` (`WaveDeckSource(sampleRate:opts:)`), up to `seconds` or the end of the song.
fn deck_render(track: PreparedTrack, deck_opts: WaveOptions, rate: f64, seconds: f64) -> Song {
    let gain = track.prepared.wave().gain;
    let total = (seconds * rate) as usize;
    let mirror = Arc::new(DeckMirror::new());
    let mut core = DeckCore::new(
        "A",
        rate,
        deck_opts.clone(),
        deck_opts.turntable_brake,
        mirror.clone(),
    );
    core.apply(Command::Load(track.prepared.project));
    core.apply(Command::Play);
    let (mut left, mut right) = (vec![0f32; total], vec![0f32; total]);
    let block = 512;
    let mut done = 0;
    while done < total {
        let n = block.min(total - done);
        let (l, r) = (&mut left[done..done + n], &mut right[done..done + n]);
        core.render(l, r, done as i64);
        done += n;
        if mirror.stopped.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
    }
    left.truncate(done);
    right.truncate(done);
    Song {
        left,
        right,
        rate,
        gain,
    }
}

fn preparer(rate: f64) -> Preparer {
    let (e, _r) = engine(EngineConfig::new(rate));
    e.preparer("A").unwrap()
}

/// A song as a deck plays it: `tweak` adjusts the deck's options before the track is prepared
/// with them; the deck itself plays on `.extended` (`WaveDeckSource(sampleRate:opts: .extended)`).
fn song(
    spec: &TrackSpec,
    rate: f64,
    seconds: f64,
    start_tick: i64,
    tweak: impl FnOnce(&mut Preparer),
) -> Song {
    let mut p = preparer(rate);
    tweak(&mut p);
    let deck_opts = {
        let mut o = WaveOptions::extended();
        o.master = p.opts.master;
        o
    };
    let track = PreparedTrack::compose(spec, &p, start_tick);
    deck_render(track, deck_opts, rate, seconds)
}

fn lufs(s: &Song) -> Option<f64> {
    IntegratedLoudness::integrated(&s.left, Some(&s.right), s.rate)
}

fn f1(x: f64) -> String {
    format!("{x:.1}")
}

// MARK: LoudnessSpreadTests

/// The genres the matrix plays: the level surveys' twelve (`LevelMatchTests`).
const SPREAD_GENRES: [&str; 12] = [
    "techno",
    "house",
    "edm",
    "trance",
    "dnb",
    "dubstep",
    "synthwave",
    "lofi",
    "jazz",
    "funk",
    "bossa",
    "country",
];

/// LoudnessSpreadTests.everyFamilyTakesTheSingleReferenceUntilItsBandIsMeasured
#[test]
fn every_family_takes_the_single_reference_until_its_band_is_measured() {
    use djengine::musicality::loudness::Loudness;
    use djengine::musicality::mix_balance::MixBalance;
    let targets = MixTargets::shared();
    let mut genres: Vec<Option<&str>> = SPREAD_GENRES.iter().map(|g| Some(*g)).collect();
    genres.push(None);
    for genre in genres {
        let family = MixBalance::family(genre);
        if genre.is_some() {
            assert!(family.is_some(), "{genre:?}");
        }
        if let Some(family) = family {
            assert!(targets.band("lufs", family).is_none(), "{family}");
        }
        assert_eq!(LevelMatch::target_lufs(genre), LevelMatch::REFERENCE_LUFS);
        assert_eq!(Loudness::target_db_for(genre), Loudness::target_db());
    }
}

/// The twelve songs, whole (seed 42, standard structure), integrated loudness each.
fn spread_matrix() -> Vec<(&'static str, Song)> {
    let rate = 24_000.0;
    SPREAD_GENRES
        .iter()
        .map(|g| {
            let spec = TrackSpec::new(g, 42.0, Some("standard"));
            (*g, song(&spec, rate, 400.0, 0, |_| {}))
        })
        .collect()
}

// LoudnessSpreadTests.synthSongsSpreadWithinTwoLU, and .aCentredCrossfadeKeepsTheLevel over the
// techno and lofi renders: whole songs of twelve genres as a deck plays them land within
// `LevelMatch.maxSpreadLU` of each other; a centred A -> B crossfade lands within
// `LevelMatch.crossfadeToleranceLU` of either deck alone.
#[test]
fn synth_songs_spread_within_two_lu_and_a_centred_crossfade_keeps_the_level() {
    let matrix = spread_matrix();
    let mut levels = Vec::new();
    for (genre, s) in &matrix {
        let l = lufs(s).unwrap_or_else(|| panic!("synth {genre}: silent"));
        println!(
            "LoudnessSpread synth {genre:<10} {} LUFS (target {}), gain {} dB",
            f1(l),
            f1(LevelMatch::target_lufs(Some(genre))),
            f1(20.0 * s.gain.log10())
        );
        levels.push((*genre, l));
    }
    let hi = levels.iter().map(|x| x.1).fold(f64::MIN, f64::max);
    let lo = levels.iter().map(|x| x.1).fold(f64::MAX, f64::min);
    println!(
        "LoudnessSpread synth: spread {} LU over {}",
        f1(hi - lo),
        levels.len()
    );
    assert_eq!(levels.len(), SPREAD_GENRES.len());
    assert!(
        hi - lo <= LevelMatch::MAX_SPREAD_LU,
        "synth LUFS spread {} LU: {levels:?}",
        hi - lo
    );

    // a dance track on A, a chill one on B, unity knobs, mixed through the crossfader's own
    // gains (`DJEngine.crossfadeGain`)
    use djhost::deck::crossfade_gain;
    let find = |g: &str| &matrix.iter().find(|(n, _)| *n == g).unwrap().1;
    let (a, b) = (find("techno"), find("lofi"));
    let n = a.left.len().min(b.left.len());
    assert!(n > (a.rate * 30.0) as usize);
    let mix = |at: f64| -> Option<f64> {
        let (ga, gb) = (
            crossfade_gain("A", at) as f32,
            crossfade_gain("B", at) as f32,
        );
        let l: Vec<f32> = (0..n).map(|i| ga * a.left[i] + gb * b.left[i]).collect();
        let r: Vec<f32> = (0..n).map(|i| ga * a.right[i] + gb * b.right[i]).collect();
        IntegratedLoudness::integrated(&l, Some(&r), a.rate)
    };
    let (la, lb, lc) = (mix(0.0).unwrap(), mix(1.0).unwrap(), mix(0.5).unwrap());
    println!(
        "LoudnessSpread crossfade: A {}, B {}, centre {} LUFS",
        f1(la),
        f1(lb),
        f1(lc)
    );
    assert!(
        (lc - la).abs() <= LevelMatch::CROSSFADE_TOLERANCE_LU,
        "centre {lc} vs A {la}"
    );
    assert!(
        (lc - lb).abs() <= LevelMatch::CROSSFADE_TOLERANCE_LU,
        "centre {lc} vs B {lb}"
    );
}

// MARK: SpectralSlopeTests / HFTiltTests

/// The matrix's genres (scripts/musicality-matrix.py `GENRES`).
const SURVEY_GENRES: [&str; 12] = [
    "techno",
    "house",
    "edm",
    "trance",
    "dnb",
    "dubstep",
    "synthwave",
    "lofi",
    "jazz",
    "funk",
    "bossa",
    "ambient",
];

/// SpectralSlopeTests' `render`: `genre` at `seed` as a deck plays it, from the top, for
/// `seconds`: `.extended`, the loudness gain fixed at 1 and a linear master so the mix and its
/// solos are rendered at one gain (the parts' trims and energy curve are taken from the whole
/// project, `WaveRunner.prepare`), so each solo sits where it does in the mix. `without` is the
/// fixes turned off.
fn slope_song(
    genre: &str,
    seed: f64,
    solo: Option<&[&str]>,
    seconds: f64,
    without: djengine::musicality::Fixes,
) -> Song {
    let spec = TrackSpec::new(genre, seed, Some("standard"));
    let filter = solo.map(|names| {
        let names: Vec<String> = if names.is_empty() {
            // the fill chains (`fill-N`), or a name nothing has
            let fills: Vec<String> = spec
                .project()
                .nets
                .keys()
                .iter()
                .filter(|k| k.starts_with("fill-"))
                .map(|k| k.to_string())
                .collect();
            if fills.is_empty() {
                vec!["(no fills)".to_string()]
            } else {
                fills
            }
        } else {
            names.iter().map(|s| s.to_string()).collect()
        };
        RoleFilter {
            mute: vec![],
            solo: Some(names),
        }
    });
    song(&spec, 48_000.0, seconds, 0, |p| {
        p.opts.master = Master::Linear;
        p.opts.track_gain = Some(1.0);
        p.opts.musicality = djengine::musicality::Fixes(p.opts.musicality.0 & !without.0);
        p.role_filter = filter;
    })
}

struct Slope {
    slope: f64,
    bump: f64,
    hf_power: f64,
    plays: bool,
}

/// `SlopeRow`.
fn slope_row(s: &Song) -> Slope {
    let sp = Spectrum::new(&s.left, &s.right, s.rate);
    let bump_bands = djengine::musicality::spectrum::BUMP_BANDS;
    let mut hf = 0.0;
    for (i, hz) in djengine::musicality::spectrum::BANDS_HZ.iter().enumerate() {
        if bump_bands.contains(hz) {
            hf += 10f64.powf(sp.ltas_db[i] / 10.0);
        }
    }
    Slope {
        slope: sp.slope_db_per_oct,
        bump: sp.hf_bump_db,
        hf_power: hf,
        plays: sp.ltas_db.iter().any(|x| *x > -150.0),
    }
}

// SpectralSlopeTests.aMixAndASoloMeasureFinite
#[test]
fn a_mix_and_a_solo_measure_finite() {
    use djengine::musicality::Fixes;
    let mix = slope_row(&slope_song("techno", 42.0, None, 12.0, Fixes::EMPTY));
    let hats = slope_row(&slope_song(
        "techno",
        42.0,
        Some(&["hihat"]),
        12.0,
        Fixes::EMPTY,
    ));
    assert!(mix.plays && hats.plays);
    for v in [mix.slope, mix.bump, hats.slope, hats.bump] {
        assert!(v.is_finite(), "{v}");
    }
    assert!(mix.hf_power > 0.0 && mix.hf_power.is_finite());
    assert!(
        hats.hf_power <= mix.hf_power * 1.5,
        "a solo is not louder up top than its mix"
    );
}

// HFTiltTests.lowersTheHatsTop: the soloed hats' 12.5 / 16 kHz power falls with the fix on, the
// rest of their spectrum stays: the fix takes the top off and leaves the hats in.
#[test]
fn hf_tilt_lowers_the_hats_top() {
    use djengine::musicality::Fixes;
    let on = slope_row(&slope_song(
        "techno",
        42.0,
        Some(&["hihat"]),
        12.0,
        Fixes::EMPTY,
    ));
    let off = slope_row(&slope_song(
        "techno",
        42.0,
        Some(&["hihat"]),
        12.0,
        Fixes::HF_TILT,
    ));
    assert!(on.plays && off.plays);
    assert!(on.hf_power > 0.0 && off.hf_power > 0.0);
    let drop_db = 10.0 * (off.hf_power / on.hf_power).log10();
    println!(
        "hfTilt techno hats: 12.5/16 kHz {} dB down; bump {} -> {}",
        f1(drop_db),
        f1(off.bump),
        f1(on.bump)
    );
    assert!(drop_db >= 4.0, "the top falls: {drop_db} dB");
    assert!(on.bump < off.bump);
}

// HFTiltTests.everyGenreFallsWithNoBump: the acceptance: with every fix on (`.extended`), each of
// the 12 genres' mixes falls (slope < 0, inside the family's band) and has no 16 kHz bump
// (hfBump <= `MixTargets.maxHFBumpDB`). Seed 42, 30 s from the top; the synth voices (the
// sampled ones are a deck's bank: `deck_sampled.rs`).
#[test]
fn every_genre_falls_with_no_bump() {
    use djengine::musicality::Fixes;
    let mut lines = Vec::new();
    for genre in SURVEY_GENRES {
        let mix = slope_row(&slope_song(genre, 42.0, None, 30.0, Fixes::EMPTY));
        lines.push(format!(
            "{genre} synth: slope {} hfBump {}",
            f1(mix.slope),
            f1(mix.bump)
        ));
        assert!(mix.plays, "{genre}");
        assert!(mix.slope < 0.0, "{genre} synth: slope {}", mix.slope);
        // synthwave on synth voices reads 3.3 dB in Swift (first run, 2026-10-19): a dark mix
        // whose 12.5 / 16 kHz power is the hats and, about as much, the synth snare, which
        // `.hfTilt` leaves alone by design; held where it is, not at the target
        let limit = if genre == "synthwave" {
            MixTargets::MAX_HF_BUMP_DB + 1.5
        } else {
            MixTargets::MAX_HF_BUMP_DB
        };
        assert!(
            mix.bump <= limit,
            "{genre} synth: hfBump {} (limit {limit})",
            mix.bump
        );
    }
    for l in lines {
        println!("hfTilt {l}");
    }
}

// MARK: RenderStemTests

// RenderStemTests.soloBassIsABassStem (the deck-chain half): `djctl render --synth` as the deck
// plays it, through the master chain and the limiter.
#[test]
fn solo_bass_is_a_bass_stem_through_the_deck_chain() {
    let request = RenderRequest::new(
        "techno",
        42.0,
        Some("standard"),
        None,
        None,
        RenderRequest::filter(None, Some("bass")),
    );
    let spec = request.spec().unwrap();
    let rate = 48_000.0;
    let stem = song(&spec, rate, 10.0, 0, |p| {
        p.role_filter = request.filter.clone()
    });
    let whole = song(&spec, rate, 10.0, 0, |_| {});
    let m = stem.metrics();
    let low =
        m.bands.get("sub").copied().unwrap_or(0.0) + m.bands.get("low").copied().unwrap_or(0.0);
    println!(
        "the soloed techno 42 bass: sub {:.3} + low {:.3} = {low:.3} (Swift 0.787), {:.1} dB",
        m.bands.get("sub").copied().unwrap_or(0.0),
        m.bands.get("low").copied().unwrap_or(0.0),
        m.rms_db
    );
    assert!(m.rms_db > -60.0, "the bass stem plays: {}", m.rms_db);
    // >= 0.75: techno 42's soloed synth bass measures 0.787 in Swift (sub 0.12, low 0.66; its
    // saw's harmonics are lowMid), the same with or without `.unison`, `.kickTune` or
    // `.energy`; `.bassLine`'s line is what lifts it out of the sub
    assert!(low >= 0.75, "sub + low = {low}");
    // and it is a stem: quieter than the whole track
    assert!(m.rms_db < whole.metrics().rms_db);
}

// MARK: PaletteTests

// PaletteTests.theSynthPaletteIsTheSynthRender: the Synth palette is beats' own synth voices:
// bit for bit what the deck plays with no sampler at all.
#[test]
fn the_synth_palette_is_the_synth_render() {
    use djengine::sound::Installed;
    let spec = TrackSpec::new("house", 11.0, Some("standard"));
    let synth = spec.with_palette(Some("synth"), &Installed::nothing(""));
    let a = song(&synth, 24_000.0, 6.0, 0, |_| {});
    let b = song(&spec, 24_000.0, 6.0, 0, |_| {});
    assert!(a.left == b.left && a.right == b.right, "bit for bit");
    assert!(a.left.iter().any(|x| *x != 0.0));
}
