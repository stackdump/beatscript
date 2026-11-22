//! Measure the clock thread on its own: process CPU time and tick jitter, through a virtual
//! MIDI port this program owns (no device hears anything).
//!
//! ```text
//! cargo run --release -p djhost --features midi --example clock_bench -- [BPM] [SECONDS]
//! ```
//!
//! Prints the process CPU time per second of wall clock (the clock thread plus the backend's
//! receive thread; run with BPM 0 for the idle baseline, no clock at all), and each tick's
//! arrival against the ideal grid: a straight line fitted through the arrivals (so the
//! backend's constant latency drops out), as mean / p99 / max absolute deviation, plus the
//! thread's own worst send lateness.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use djengine::clock::{OutputConfig, OutputRole};
use djhost::clock::Clock;
use midir::os::unix::VirtualInput;
use midir::MidiInput;

/// What the virtual port received: arrival time and bytes.
type Received = Arc<Mutex<Vec<(Instant, Vec<u8>)>>>;

fn cpu_seconds() -> f64 {
    // SAFETY: getrusage fills a plain struct.
    #[allow(unsafe_code)]
    let ru = unsafe {
        let mut ru: libc::rusage = std::mem::zeroed();
        libc::getrusage(libc::RUSAGE_SELF, &mut ru);
        ru
    };
    let tv = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 * 1e-6;
    tv(ru.ru_utime) + tv(ru.ru_stime)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let bpm: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(120.0);
    let secs: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(10.0);
    let name = format!("djrust-clock-bench-{}", std::process::id());
    let got: Received = Arc::default();
    let g = got.clone();
    let input = MidiInput::new("djrust-clock-bench").expect("MIDI backend");
    let _conn = input
        .create_virtual(
            &name,
            move |_, bytes, _| g.lock().unwrap().push((Instant::now(), bytes.to_vec())),
            (),
        )
        .expect("virtual port");
    std::thread::sleep(Duration::from_millis(300));
    if bpm <= 0.0 {
        let c0 = cpu_seconds();
        std::thread::sleep(Duration::from_secs_f64(secs));
        println!(
            "idle baseline: cpu {:.2} ms/s",
            (cpu_seconds() - c0) / secs * 1000.0
        );
        return;
    }
    let clock = Clock::spawn("djrust-clock-bench", bpm);
    clock.set_outputs(vec![OutputConfig {
        port: name.clone(),
        role: OutputRole::Clock,
        offset: 0.0,
        enabled: true,
    }]);
    std::thread::sleep(Duration::from_millis(500));
    clock.start_at(clock.now() + 0.05);
    std::thread::sleep(Duration::from_millis(100));
    clock.reset_stats();
    got.lock().unwrap().clear();
    let c0 = cpu_seconds();
    let w0 = Instant::now();
    std::thread::sleep(Duration::from_secs_f64(secs));
    let cpu = cpu_seconds() - c0;
    let wall = w0.elapsed().as_secs_f64();
    let status = clock.status();
    clock.stop();
    drop(clock);
    let got = got.lock().unwrap().clone();
    let ticks: Vec<f64> = got
        .iter()
        .filter(|(_, b)| b == &[0xF8])
        .map(|(t, _)| t.duration_since(w0).as_secs_f64())
        .collect();
    let n = ticks.len() as f64;
    // least-squares line t = a + b k
    let ks: Vec<f64> = (0..ticks.len()).map(|k| k as f64).collect();
    let mk = ks.iter().sum::<f64>() / n;
    let mt = ticks.iter().sum::<f64>() / n;
    let b = ks
        .iter()
        .zip(&ticks)
        .map(|(k, t)| (k - mk) * (t - mt))
        .sum::<f64>()
        / ks.iter().map(|k| (k - mk).powi(2)).sum::<f64>();
    let a = mt - b * mk;
    let mut dev: Vec<f64> = ks
        .iter()
        .zip(&ticks)
        .map(|(k, t)| (t - (a + b * k)).abs() * 1000.0)
        .collect();
    dev.sort_by(|x, y| x.partial_cmp(y).unwrap());
    let mean = dev.iter().sum::<f64>() / n;
    let p99 = dev[((n * 0.99) as usize).min(dev.len() - 1)];
    let max = *dev.last().unwrap();
    let ideal = 60.0 / (bpm * 24.0);
    println!(
        "{bpm} BPM, {:.1} s: {} ticks, period {:.4} ms (ideal {:.4}), cpu {:.2} ms/s ({:.2}%), \
         |dev| mean {mean:.4} ms p99 {p99:.4} ms max {max:.4} ms, max late {:.4} ms, skipped {}, realtime {}, spin margin now {:.0} us",
        wall,
        ticks.len(),
        b * 1000.0,
        ideal * 1000.0,
        cpu / wall * 1000.0,
        cpu / wall * 100.0,
        status.max_late * 1000.0,
        status.skipped,
        status.realtime,
        status.spin_margin * 1e6,
    );
}
