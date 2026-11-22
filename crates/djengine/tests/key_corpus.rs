//! Key detection on rendered generated tracks, whose keys are known (docs/KEY.md, "How well it
//! works"). Opt in, as it needs renders on disk:
//!
//! ```sh
//! mkdir -p /tmp/keycorpus
//! for g in acid ambient blues bossa country deephouse dnb dubstep edm electro funk garage house \
//!          italo jazz lofi metal reggae synthwave techno trance trap; do
//!   for s in 1 2; do djctl bounce $g $s /tmp/keycorpus/$g-$s.wav --seconds 30; done
//! done
//! DJ_KEY_CORPUS=/tmp/keycorpus cargo test -p djengine --test key_corpus -- --ignored --nocapture
//! ```
//!
//! It prints, per chroma setting and profile, how many detections are the composed key exactly
//! and how many mix with it (same / relative / adjacent), and the symbolic ceiling: the profiles
//! run on the notes the nets actually play. It asserts the default holds its measured level.

use djengine::key::{self, ChromaOpts, Key, Profile};

fn read_wav(path: &std::path::Path) -> (Vec<f32>, f64) {
    let b = std::fs::read(path).unwrap();
    let ch = u16::from_le_bytes([b[22], b[23]]) as usize;
    let sr = u32::from_le_bytes([b[24], b[25], b[26], b[27]]) as f64;
    let bits = u16::from_le_bytes([b[34], b[35]]);
    let mut i = 12;
    while &b[i..i + 4] != b"data" {
        i += 8 + u32::from_le_bytes([b[i + 4], b[i + 5], b[i + 6], b[i + 7]]) as usize;
    }
    let d = &b[i + 8..];
    let w = (bits / 8) as usize;
    let mono = d
        .chunks_exact(w * ch)
        .map(|f| {
            (0..ch)
                .map(|c| match w {
                    4 => f32::from_le_bytes([f[4 * c], f[4 * c + 1], f[4 * c + 2], f[4 * c + 3]]),
                    _ => i16::from_le_bytes([f[2 * c], f[2 * c + 1]]) as f32 / 32768.0,
                })
                .sum::<f32>()
                / ch as f32
        })
        .collect();
    (mono, sr)
}

#[test]
#[ignore]
fn detection_on_rendered_generated_tracks() {
    let dir = std::env::var("DJ_KEY_CORPUS").expect("DJ_KEY_CORPUS: a folder of <genre>-<seed>.wav renders");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "wav"))
        .collect();
    files.sort();
    let data: Vec<(String, Key, Vec<f32>, f64, [f64; 12])> = files
        .iter()
        .filter_map(|p| {
            let stem = p.file_stem()?.to_string_lossy().to_string();
            let (g, s) = stem.rsplit_once('-')?;
            let proj = djengine::loopsets::played(g, s.parse().ok()?);
            let k = Key::from_project(&proj)?;
            let mut notes = [0.0f64; 12];
            for (id, n) in proj.nets.iter() {
                if djengine::stems::is_drum_net(id, n) || n.role == "control" {
                    continue;
                }
                for (_, b) in n.bindings.iter() {
                    notes[b.note.rem_euclid(12) as usize] += 1.0;
                }
            }
            let (x, sr) = read_wav(p);
            Some((stem, k, x, sr, notes))
        })
        .collect();
    let n = data.len();
    assert!(n > 0, "no renders in {dir}");
    let profiles = [Profile::Ensemble, Profile::KrumhanslKessler, Profile::Temperley, Profile::AlbrechtShanahan];
    for p in profiles {
        let exact = data.iter().filter(|d| key::from_chroma(&d.4, p).key == d.1).count();
        let harm = data.iter().filter(|d| key::compat(d.1, key::from_chroma(&d.4, p).key).harmonic()).count();
        println!("notes (ceiling)  {:10} exact {exact:2}/{n}  mixes {harm:2}/{n}", p.name());
    }
    let mut default_mixes = 0;
    for (name, o) in [("plain", ChromaOpts::PLAIN), ("default", ChromaOpts::DEFAULT)] {
        for p in profiles {
            let dets: Vec<Key> = data.iter().map(|d| key::detect_with(&d.2, d.3, p, o).unwrap().key).collect();
            let exact = data.iter().zip(&dets).filter(|(d, k)| **k == d.1).count();
            let harm = data.iter().zip(&dets).filter(|(d, k)| key::compat(d.1, **k).harmonic()).count();
            println!("audio {name:9}  {:10} exact {exact:2}/{n}  mixes {harm:2}/{n}", p.name());
            if name == "default" && p == Profile::Ensemble {
                default_mixes = harm;
            }
        }
    }
    // measured 2026-11-17 on 44 renders (22 genres x seeds 1-2, 30 s): 33/44 mix
    assert!(default_mixes as f64 >= 0.7 * n as f64, "the default mixes with the composed key on {default_mixes}/{n}");
}
