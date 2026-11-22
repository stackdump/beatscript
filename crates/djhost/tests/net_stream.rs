//! The phone's mix into a desktop strip over real TCP (docs/NET-PROTOCOL.md §8), against ground
//! truth: the sender writes a click on every beat (120 BPM) into a capture tap in real time, its
//! clock `DJNET_PPM` off this box's (a phone's crystal); the receiver feeds a ring that a stand-in
//! output pulls from in real time. Each click's render time on the sender and the moment it comes
//! out here are both on this process's clock, so the true end-to-end latency is known, and so
//! is where the receiver *thinks* each click is on the beat grid.
//!
//! `cargo test --release -p djhost --features network --test net_stream -- --ignored --nocapture`
//! runs 20 s; `DJNET_SECS=180 DJNET_PPM=100` measures drift over three minutes. It is real time
//! and ignored by default: a loaded machine's timers overshoot and it fails for that reason
//! (docs/NET-PROTOCOL.md §13). The drop-out test runs always.
#![cfg(feature = "network")]

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use djhost::deck::capture::CaptureTap;
use djhost::deck::input::InputMirror;
use djhost::heos::stream::{ClockCell, StreamServer};
use djhost::net::clock::{mono_ns, wrap_half, BeatPoint};
use djhost::net::receiver::Receiver;

const RATE: f64 = 48_000.0;
const BPM: f64 = 120.0;
const FPB: i64 = 24_000; // frames a beat at 120 BPM
const BLOCK: usize = 256;

fn env(k: &str, d: f64) -> f64 {
    std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d)
}

struct Sender {
    stop: Arc<AtomicBool>,
    /// (click frame, render time ns)
    clicks: Arc<Mutex<Vec<(i64, i64)>>>,
    thread: Option<std::thread::JoinHandle<()>>,
    srv: StreamServer,
}

fn sender(ppm: f64) -> Sender {
    let tap = Arc::new(CaptureTap::new());
    let cell: ClockCell = Arc::new(Mutex::new(None));
    let srv = StreamServer::start_with_clock("127.0.0.1:0".parse().unwrap(), tap.clone(), RATE as u32, cell.clone()).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let clicks = Arc::new(Mutex::new(Vec::new()));
    let (flag, cl) = (stop.clone(), clicks.clone());
    let thread = std::thread::spawn(move || {
        let real = RATE * (1.0 + ppm * 1e-6);
        let t0 = Instant::now();
        let mut frame = 0i64;
        let mut buf = vec![0f32; BLOCK];
        while !flag.load(Ordering::Relaxed) {
            for (k, x) in buf.iter_mut().enumerate() {
                let f = frame + k as i64;
                *x = if f % FPB < 48 { 0.9 } else { 0.0 };
            }
            tap.write(frame, &buf, &buf);
            let now = mono_ns();
            // the block's frames were "rendered" now
            for k in 0..BLOCK as i64 {
                if (frame + k) % FPB == 0 {
                    cl.lock().unwrap().push((frame + k, now));
                }
            }
            frame += BLOCK as i64;
            *cell.lock().unwrap() = Some(BeatPoint {
                bpm: BPM,
                beat: frame as f64 / FPB as f64,
                t: now,
                frame,
                rate: RATE,
                playing: true,
                source: "A".into(),
                out_ns: 0,
            });
            let due = Duration::from_secs_f64(frame as f64 / real);
            if let Some(w) = due.checked_sub(t0.elapsed()) {
                std::thread::sleep(w);
            }
        }
    });
    Sender { stop, clicks, thread: Some(thread), srv }
}

#[test]
#[ignore = "real time, 20 s+: run on a quiet machine (cargo test --release … -- --ignored --nocapture)"]
fn the_stream_plays_on_its_beat_grid_with_a_measured_latency() {
    let secs = env("DJNET_SECS", 20.0);
    let ppm = env("DJNET_PPM", 0.0);
    let latency_ms = env("DJNET_LATENCY_MS", 80.0);
    let mut s = sender(ppm);
    let url = format!("http://127.0.0.1:{}/master.djl", s.srv.port());

    // this side's strip: a ring and its mirror, pulled in real time by a stand-in output
    let (prod, mut cons) = rtrb::RingBuffer::<f32>::new(RATE as usize);
    let mirror = Arc::new(InputMirror::default());
    mirror.block.store(BLOCK as u32, Ordering::Relaxed);
    let rx = Arc::new(Receiver::start(&url, prod, mirror.clone(), RATE, latency_ms));
    rx.set_peer_offset(Some(0)); // one process: one clock
    let stop = Arc::new(AtomicBool::new(false));
    let underruns = Arc::new(AtomicU32::new(0));
    // (when a click came out here, the receiver's beat for that moment)
    let heard: Arc<Mutex<Vec<(i64, f64)>>> = Arc::new(Mutex::new(Vec::new()));
    let (flag, rx2, h2, u2) = (stop.clone(), rx.clone(), heard.clone(), underruns.clone());
    let out = std::thread::spawn(move || {
        let t0 = Instant::now();
        let mut done = 0u64;
        let mut prev = 0f32;
        let mut primed = false;
        while !flag.load(Ordering::Relaxed) {
            let avail = cons.slots();
            if !primed && avail >= 1024 + BLOCK {
                primed = true;
            }
            if primed {
                let take = avail.min(BLOCK);
                if take < BLOCK {
                    u2.fetch_add(1, Ordering::Relaxed);
                }
                if let Ok(c) = cons.read_chunk(take) {
                    let now = mono_ns();
                    for (k, x) in c.into_iter().enumerate() {
                        if x > 0.4 && prev <= 0.4 {
                            // this block plays now; sample k of it a little later
                            let at = now + (k as f64 * 1e9 / RATE) as i64;
                            if let Some(b) = rx2.beat() {
                                h2.lock().unwrap().push((at, b.beat_at(at)));
                            }
                        }
                        prev = x;
                    }
                }
            }
            done += BLOCK as u64;
            let due = Duration::from_secs_f64(done as f64 / RATE);
            if let Some(w) = due.checked_sub(t0.elapsed()) {
                std::thread::sleep(w);
            }
        }
    });

    let t0 = Instant::now();
    let mut samples = Vec::new();
    while t0.elapsed() < Duration::from_secs_f64(secs) {
        std::thread::sleep(Duration::from_secs(1));
        let st = rx.stats();
        let e = t0.elapsed().as_secs_f64();
        println!(
            "{e:6.1}s {:>10} buffer {:6.1}/{:5.1} ms  correction {:+7.1} ppm  e2e(reported) {:>7}  underruns {} jumps {} resets {} chunks {} kB {}",
            st.state,
            st.buffer_ms,
            st.target_ms,
            st.correction_ppm,
            st.e2e_ms.map_or("—".into(), |x| format!("{x:.1} ms")),
            st.underruns,
            st.jumps, st.resets, st.chunks, st.bytes / 1000
        );
        samples.push((e, st));
    }
    stop.store(true, Ordering::Relaxed);
    let _ = out.join();
    s.stop.store(true, Ordering::Relaxed);
    let _ = s.thread.take().unwrap().join();

    // ground truth: each click heard, against the click's render time on the sender
    let clicks = s.clicks.lock().unwrap().clone();
    let heard = heard.lock().unwrap().clone();
    assert!(heard.len() as f64 > secs * 1.5, "clicks heard: {}", heard.len());
    // the k-th click heard after the first is the click whose beat the receiver names
    let mut e2e = Vec::new();
    let mut phase = Vec::new();
    for (at, beat) in &heard {
        let k = beat.round() as i64;
        if let Some((_, rendered)) = clicks.iter().find(|(f, _)| *f == k * FPB) {
            e2e.push((at - rendered) as f64 / 1e6);
        }
        phase.push((at, wrap_half(*beat)));
    }
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
    let skip = phase.len() / 5; // after settling
    let settled: Vec<f64> = phase[skip..].iter().map(|(_, p)| *p).collect();
    let worst = settled.iter().fold(0f64, |w, p| w.max(p.abs()));
    let first: Vec<f64> = settled.iter().take(10).copied().collect();
    let last: Vec<f64> = settled.iter().rev().take(10).copied().collect();
    let drift = mean(&last) - mean(&first);
    // against a desktop master: a perfect 120 BPM grid on this box's clock, lined up on the
    // first settled click (a Sync), then left alone: how far the stream drifts from it
    let (a0, _) = phase[skip];
    let vs_master: Vec<f64> = heard[skip..]
        .iter()
        .map(|(at, _)| wrap_half((at - a0) as f64 * BPM / 60e9))
        .collect();
    let master_drift = mean(&vs_master.iter().rev().take(10).copied().collect::<Vec<_>>())
        - mean(&vs_master.iter().take(10).copied().collect::<Vec<_>>());
    let reported: Vec<f64> = samples[samples.len() / 5..].iter().filter_map(|(_, s)| s.e2e_ms).collect();
    let true_e2e = mean(&e2e[e2e.len() / 5..]);
    let last = &samples.last().unwrap().1;
    println!("--- {secs} s at {ppm:+} ppm, latency setting {latency_ms} ms");
    println!("clicks heard {} of {}", heard.len(), clicks.len());
    println!("end-to-end (sender render → out here): true {true_e2e:.1} ms, receiver's own figure {:.1} ms", mean(&reported));
    println!("beat phase of the clicks as the receiver dates them: worst {:.4} beat ({:.2} ms)", worst, worst * 500.0);
    println!("phase drift first→last 10 clicks: {:+.4} beat ({:+.2} ms) over {:.0} s", drift, drift * 500.0, secs * 0.8);
    println!(
        "against a desktop master synced once at the start: {:+.4} beat ({:+.2} ms) after {:.0} s; expected from the clocks {:+.4} beat",
        master_drift,
        master_drift * 500.0,
        secs * 0.8,
        -ppm * 1e-6 * secs * 0.8 * BPM / 60.0
    );
    println!("correction settled at {:+.1} ppm (sender {ppm:+} ppm); underruns here {}, receiver {}, jumps {}", last.correction_ppm, underruns.load(Ordering::Relaxed), last.underruns, last.jumps);

    // a quiet box: worst ~5 ms; a loaded one's timers jitter far more
    assert!(worst < 0.02, "the receiver dates the clicks within 1/50 beat (10 ms): worst {worst}");
    assert!((true_e2e - mean(&reported)).abs() < 10.0, "its latency figure is within 10 ms of the truth");
    assert!(true_e2e < latency_ms + 80.0, "end to end {true_e2e} ms for a {latency_ms} ms buffer");
    assert!(drift.abs() < 0.01, "no drift on the beat grid: {drift}");
    assert_eq!(last.reconnects, 0);
    match Arc::try_unwrap(rx) {
        Ok(r) => assert!(r.stop().is_some(), "the ring comes back"),
        Err(_) => panic!("still shared"),
    }
}


/// The sender goes away for a second and a half (the phone's Wi-Fi drops, its server restarts on
/// the same port): the receiver says so, reconnects, plays again, and the gap is faded both ways.
#[test]
fn a_dropped_sender_is_reconnected_and_the_gap_is_faded() {
    let tap = Arc::new(CaptureTap::new());
    let cell: ClockCell = Arc::new(Mutex::new(None));
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let bind = format!("127.0.0.1:{port}").parse().unwrap();
    let mut srv = Some(StreamServer::start_with_clock(bind, tap.clone(), RATE as u32, cell.clone()).unwrap());
    let stop = Arc::new(AtomicBool::new(false));
    let (flag, t2) = (stop.clone(), tap.clone());
    // a 220 Hz sine at 0.5, in real time
    let writer = std::thread::spawn(move || {
        let t0 = Instant::now();
        let mut frame = 0i64;
        let mut buf = vec![0f32; BLOCK];
        while !flag.load(Ordering::Relaxed) {
            for (k, x) in buf.iter_mut().enumerate() {
                *x = 0.5 * (2.0 * std::f32::consts::PI * 220.0 * (frame + k as i64) as f32 / RATE as f32).sin();
            }
            t2.write(frame, &buf, &buf);
            frame += BLOCK as i64;
            let due = Duration::from_secs_f64(frame as f64 / RATE);
            if let Some(w) = due.checked_sub(t0.elapsed()) {
                std::thread::sleep(w);
            }
        }
    });
    let (prod, mut cons) = rtrb::RingBuffer::<f32>::new(RATE as usize);
    let mirror = Arc::new(InputMirror::default());
    mirror.block.store(BLOCK as u32, Ordering::Relaxed);
    let rx = Receiver::start(&format!("http://127.0.0.1:{port}/master.djl"), prod, mirror, RATE, 80.0);
    let out: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let (flag, o2) = (stop.clone(), out.clone());
    let reader = std::thread::spawn(move || {
        let t0 = Instant::now();
        let mut done = 0u64;
        while !flag.load(Ordering::Relaxed) {
            if cons.slots() >= BLOCK {
                if let Ok(c) = cons.read_chunk(BLOCK) {
                    o2.lock().unwrap().extend(c.into_iter());
                }
            }
            done += BLOCK as u64;
            let due = Duration::from_secs_f64(done as f64 / RATE);
            if let Some(w) = due.checked_sub(t0.elapsed()) {
                std::thread::sleep(w);
            }
        }
    });
    let wait_for = |what: &str, f: &dyn Fn(&djhost::net::receiver::RxStats) -> bool| {
        let t0 = Instant::now();
        while !f(&rx.stats()) {
            assert!(t0.elapsed() < Duration::from_secs(10), "never {what}: {:?}", rx.stats());
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    wait_for("playing", &|s| s.state == "playing");
    std::thread::sleep(Duration::from_secs(2));
    drop(srv.take()); // the phone's server goes
    wait_for("noticing", &|s| s.state == "reconnecting");
    std::thread::sleep(Duration::from_millis(1500));
    srv = Some(StreamServer::start_with_clock(bind, tap.clone(), RATE as u32, cell).unwrap());
    wait_for("playing again", &|s| s.state == "playing" && s.reconnects >= 1);
    std::thread::sleep(Duration::from_secs(2));
    let st = rx.stats();
    stop.store(true, Ordering::Relaxed);
    let _ = writer.join();
    let _ = reader.join();
    drop(srv);
    let o = out.lock().unwrap();
    // the sine's steepest step is 0.5 × 2π × 220 / 48000 ≈ 0.0144: anything far above is a click
    let worst = o.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0f32, f32::max);
    let silent = o.iter().filter(|x| x.abs() < 1e-6).count();
    if let Some(k) = o.windows(2).position(|w| (w[1] - w[0]).abs() == worst) {
        let lo = k.saturating_sub(4);
        println!("steepest at sample {k} of {} ({:.3} s): {:?}", o.len(), k as f64 / RATE, &o[lo..(k + 5).min(o.len())]);
    }
    println!("reconnects {} underruns {} jumps {} resets {}; steepest step {worst:.4}, {silent} silent samples", st.reconnects, st.underruns, st.jumps, st.resets);
    assert!(st.reconnects >= 1 && st.underruns >= 1, "{st:?}");
    assert!(silent as f64 > RATE, "the gap is silence: {silent}");
    assert!(worst < 0.05, "no click at the gap: steepest step {worst}");
}
