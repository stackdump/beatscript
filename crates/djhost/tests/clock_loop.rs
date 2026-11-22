//! The clock thread against a real MIDI backend, through a virtual port this test owns, so no
//! device hears anything: ticks arrive at 24 PPQN with low jitter, Start / Stop arrive, and a
//! live note goes through. Skips (passes with a note) where the backend refuses virtual ports
//! (a build box with no ALSA sequencer access).

#![cfg(feature = "midi")]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use djengine::clock::{OutputConfig, OutputRole};
use djhost::clock::Clock;
use midir::os::unix::VirtualInput;
use midir::MidiInput;

#[test]
fn ticks_arrive_on_time_through_a_virtual_port() {
    let name = format!("djrust-clock-loop-{}", std::process::id());
    let got: Arc<Mutex<Vec<(Instant, Vec<u8>)>>> = Arc::default();
    let g = got.clone();
    let Ok(input) = MidiInput::new("djrust-clock-test") else {
        eprintln!("no MIDI backend: skipped");
        return;
    };
    let conn = match input.create_virtual(
        &name,
        move |_, bytes, _| g.lock().unwrap().push((Instant::now(), bytes.to_vec())),
        (),
    ) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("no virtual MIDI ports here ({e}): skipped");
            return;
        }
    };
    std::thread::sleep(Duration::from_millis(200)); // let the port appear
    let clock = Clock::spawn("djrust-clock-test", 120.0);
    clock.set_outputs(vec![OutputConfig {
        port: name.clone(),
        role: OutputRole::Clock,
        offset: 0.0,
        enabled: true,
    }]);
    std::thread::sleep(Duration::from_millis(300));
    clock.start_at(clock.now() + 0.05);
    std::thread::sleep(Duration::from_millis(2000));
    clock.send_now(&name, &[0x90, 60, 100]);
    clock.stop();
    std::thread::sleep(Duration::from_millis(100));
    let status = clock.status();
    drop(clock);
    drop(conn);
    let got = got.lock().unwrap().clone();
    assert!(status.last_error.is_none(), "{:?}", status.last_error);
    let start = got
        .iter()
        .position(|(_, b)| b == &[0xFA])
        .expect("Start arrived");
    assert_eq!(got[start + 1].1, vec![0xF8], "a clock follows Start");
    assert!(got.iter().any(|(_, b)| b == &[0xFC]), "Stop arrived");
    assert!(
        got.iter().any(|(_, b)| b == &[0x90, 60, 100]),
        "the live note arrived"
    );
    // ticks after Start: 2 s at 120 BPM = 96 ticks, every 20.833 ms
    let ticks: Vec<Instant> = got[start + 1..]
        .iter()
        .filter(|(_, b)| b == &[0xF8])
        .map(|(t, _)| *t)
        .collect();
    assert!(ticks.len() >= 90, "{} ticks", ticks.len());
    let iv: Vec<f64> = ticks
        .windows(2)
        .map(|w| w[1].duration_since(w[0]).as_secs_f64() * 1000.0)
        .collect();
    let mean = iv.iter().sum::<f64>() / iv.len() as f64;
    let sd = (iv.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / iv.len() as f64).sqrt();
    let worst = iv
        .iter()
        .map(|x| (x - 1000.0 / 48.0).abs())
        .fold(0.0, f64::max);
    eprintln!(
        "clock loop: {} ticks, mean {mean:.4} ms, sd {sd:.4} ms, worst {worst:.3} ms, max late {:.3} ms",
        ticks.len(),
        status.max_late * 1000.0
    );
    assert!((mean - 1000.0 / 48.0).abs() < 0.05, "mean {mean}");
    assert!(sd < 0.5, "sd {sd}");
    assert!(worst < 3.0, "worst {worst}");
}
