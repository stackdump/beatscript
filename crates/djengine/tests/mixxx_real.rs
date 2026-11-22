//! Real Mixxx mappings, read from the user's own copy at test time. Nothing is committed: set
//! `MIXXX_CONTROLLERS_DIR` to a folder of `*.midi.xml` (Mixxx's `res/controllers`, an install's
//! `controllers` folder, or a download of it), or have Mixxx installed in its usual place.
//! With neither, the test says so and passes.

use djengine::mixxx::{self, discover, profile::Why};
use std::path::PathBuf;

fn dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(d) = std::env::var("MIXXX_CONTROLLERS_DIR") {
        v.push(PathBuf::from(d));
    }
    v.push("/Applications/Mixxx.app/Contents/Resources/controllers".into());
    v.push("/usr/share/mixxx/controllers".into());
    v
}

fn files() -> Vec<PathBuf> {
    for d in dirs() {
        if let Ok(rd) = std::fs::read_dir(&d) {
            let mut f: Vec<PathBuf> = rd
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.to_string_lossy().ends_with(".midi.xml"))
                .collect();
            f.sort();
            if !f.is_empty() {
                return f;
            }
        }
    }
    Vec::new()
}

#[test]
fn every_real_mapping_parses_and_builds_a_profile_that_hangs_together() {
    let files = files();
    if files.is_empty() {
        eprintln!("skipped: no Mixxx mappings (set MIXXX_CONTROLLERS_DIR)");
        return;
    }
    let (mut controls, mut mapped, mut script) = (0, 0, 0);
    for f in &files {
        let bytes = std::fs::read(f).unwrap();
        let preset = mixxx::parse(&bytes).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
        let built = mixxx::build(&preset, "Some Port");
        let p = &built.profile;
        // every binding names a control that exists
        for b in &p.layout {
            let g = p.group(&b.group).unwrap_or_else(|| panic!("{}: no group {}", f.display(), b.group));
            assert!(b.index.unwrap() < g.control_numbers().len(), "{}: {:?}", f.display(), b);
        }
        // every control is accounted for: mapped, or listed with a reason
        let accounted = built.report.mapped.len() + built.report.unmapped.len();
        let lsb = preset.controls.iter().filter(|c| c.options.fourteen_bit_lsb).count();
        assert!(accounted + lsb >= p.groups.iter().map(|g| g.control_numbers().len()).sum::<usize>(), "{}", f.display());
        assert!(built.report.unmapped.iter().all(|u| !u.reason.is_empty()));
        // the profile round-trips through the router's own decoder: the first mapped control
        // decodes to the control the binding is on
        if let Some(b) = p.layout.first() {
            let g = p.group(&b.group).unwrap();
            let n = g.control_numbers()[b.index.unwrap()];
            let ch = g.channel.unwrap();
            let msg = match g.kind {
                djengine::control::surface::ControlType::Note => vec![0x90 | ch, n, 127],
                djengine::control::surface::ControlType::Cc => vec![0xB0 | ch, n, 127],
                djengine::control::surface::ControlType::Cc14 => vec![0xB0 | ch, n, 127],
                _ => continue,
            };
            let ev = p.parse(&msg);
            assert_eq!(ev.as_ref().map(|e| (e.group.as_str(), e.index)), Some((b.group.as_str(), b.index.unwrap())), "{}", f.display());
        }
        controls += preset.controls.len();
        mapped += built.report.mapped.len();
        script += built.report.count(Why::Script);
    }
    eprintln!("{} mappings: {controls} controls, {mapped} mapped, {script} call a script", files.len());
    assert!(controls > 0);
}

#[test]
fn discovery_finds_the_mapping_a_real_port_name_asks_for() {
    let files = files();
    if files.is_empty() {
        eprintln!("skipped: no Mixxx mappings");
        return;
    }
    let cands: Vec<discover::Candidate> = files
        .iter()
        .filter_map(|f| {
            let p = mixxx::parse(&std::fs::read(f).ok()?).ok()?;
            Some(discover::Candidate {
                id: f.to_string_lossy().into(),
                file: f.file_name()?.to_string_lossy().into(),
                name: p.name,
                controller_id: p.controller_id,
            })
        })
        .collect();
    let name_of = |port: &str| match discover::pick(port, &cands, &[]) {
        discover::Pick::Name(c) => Some(c.file.clone()),
        _ => None,
    };
    // only when the install has them (a download of Mixxx's controllers does)
    if cands.iter().any(|c| c.file == "Hercules_DJControl_Inpulse_200.midi.xml") {
        assert_eq!(name_of("Hercules DJControl Inpulse 200").as_deref(), Some("Hercules_DJControl_Inpulse_200.midi.xml"));
        assert_eq!(name_of("DJControl Inpulse 200:DJControl Inpulse 200 MIDI 1 20:0").as_deref(), Some("Hercules_DJControl_Inpulse_200.midi.xml"));
        // a different unit is not taken for it
        assert_ne!(name_of("Hercules DJControl Inpulse 300").as_deref(), Some("Hercules_DJControl_Inpulse_200.midi.xml"));
    }
    assert_eq!(name_of("Some Synth Nobody Mapped 9000"), None);
}

/// `cargo test -p djengine --test mixxx_real survey -- --ignored --nocapture`: one line a
/// mapping (controls, mapped, script, unsupported) and what the unsupported ones asked for.
#[test]
#[ignore]
fn survey() {
    let mut reasons: std::collections::BTreeMap<String, usize> = Default::default();
    let mut rows = Vec::new();
    for f in files() {
        let Ok(p) = mixxx::parse(&std::fs::read(&f).unwrap()) else { continue };
        let b = mixxx::build(&p, "x");
        for u in b.report.unmapped.iter().filter(|u| u.why == Why::Unsupported) {
            *reasons.entry(format!("{} {}", u.group, u.key)).or_default() += 1;
        }
        rows.push((b.report.mapped.len(), p.controls.len(), b.report.count(Why::Script), b.report.count(Why::Unsupported), p.name));
    }
    rows.sort_by(|a, b| b.0.cmp(&a.0));
    for (m, c, s, u, n) in &rows {
        println!("{m:4} mapped / {c:4} controls  script {s:4}  unsupported {u:4}  {n}");
    }
    let mut r: Vec<_> = reasons.into_iter().collect();
    r.sort_by(|a, b| b.1.cmp(&a.1));
    println!("--- most asked-for and unsupported");
    for (k, n) in r.iter().take(40) {
        println!("{n:4} {k}");
    }
}
