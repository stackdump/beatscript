//! Where a render's CPU goes: the server's pipeline (beatscript's `render_to` → f32 PCM → the in-process Ogg Opus
//! encoder on its own thread, or s16 WAV), sampled with pprof (SIGPROF, no perf_event privileges needed).
//!
//!   CARGO_PROFILE_RELEASE_DEBUG=line-tables-only cargo run --release -p beatscript-serve --example profile_render -- \
//!       <program.beatscript> [opus|wav] [out.svg]
//!
//! Prints the top functions by self time and by inclusive time, and writes a flamegraph.

#[path = "../../beatscript-cli/src/render.rs"]
#[allow(dead_code)]
mod render;
#[path = "../src/opus.rs"]
#[allow(dead_code)]
mod opus;

use std::collections::HashMap;
use std::io::{self, Write};
use std::time::Instant;

struct Chan(std::sync::mpsc::SyncSender<Vec<f32>>, Vec<u8>);
impl Write for Chan {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.1.extend_from_slice(b);
        let whole = self.1.len() / 8 * 8;
        let v: Vec<f32> = self.1[..whole].chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        self.1.drain(..whole);
        self.0.send(v).map_err(|_| io::Error::other("encoder gone"))?;
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let text = std::fs::read_to_string(&a[1]).expect("program");
    let fmt = a.get(2).map_or("opus", String::as_str).to_string();
    let svg = a.get(3).cloned().unwrap_or_else(|| "profile.svg".into());
    let guard = pprof::ProfilerGuardBuilder::default().frequency(997).blocklist(&["libc", "libgcc", "pthread", "vdso"]).build().unwrap();
    let t0 = Instant::now();
    let mut ev_out = io::sink();
    let mut ev = render::Events::new(&mut ev_out);
    let summary = if fmt == "wav" {
        let o = render::Options { file: None, sr: 44_100, format: render::Format::S16le, chunk_ms: 250, max_bars: 2400, from_bar: 0 };
        let mut out = Vec::new();
        render::render_to(&text, &o, &mut out, &mut ev, t0).expect("render")
    } else {
        let o = render::Options { file: None, sr: 48_000, format: render::Format::F32le, chunk_ms: 250, max_bars: 2400, from_bar: 0 };
        let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(8);
        let enc = std::thread::spawn(move || {
            let mut out = Vec::new();
            {
                let mut e = opus::OggOpus::new(&mut out, 44_100, 96_000, 1).unwrap();
                for c in rx {
                    e.push(&c).unwrap();
                }
                e.finish().unwrap();
            }
            out.len()
        });
        let s = {
            let mut sink = Chan(tx, Vec::new());
            render::render_to(&text, &o, &mut sink, &mut ev, t0).expect("render")
        };
        let n = enc.join().unwrap();
        eprintln!("opus bytes {n}");
        s
    };
    let wall = t0.elapsed().as_secs_f64();
    let report = guard.report().build().unwrap();
    let (mut selft, mut incl): (HashMap<String, isize>, HashMap<String, isize>) = (HashMap::new(), HashMap::new());
    let mut total = 0isize;
    for (frames, n) in &report.data {
        total += n;
        let names: Vec<String> = frames.frames.iter().flat_map(|f| f.iter().map(|s| s.name())).collect();
        if let Some(leaf) = names.first() {
            *selft.entry(short(leaf)).or_default() += n;
        }
        let mut seen = std::collections::HashSet::new();
        for nm in &names {
            let k = short(nm);
            if seen.insert(k.clone()) {
                *incl.entry(k).or_default() += n;
            }
        }
    }
    println!("{fmt}: {:.1} s of audio in {wall:.2} s wall ({:.1}x real time), {total} samples", summary.seconds, summary.seconds / wall);
    let top = |m: &HashMap<String, isize>, title: &str, k: usize| {
        let mut v: Vec<_> = m.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1));
        println!("\n{title}");
        for (name, n) in v.into_iter().take(k) {
            println!("{:6.2}%  {name}", *n as f64 * 100.0 / total as f64);
        }
    };
    top(&selft, "self", 30);
    top(&incl, "inclusive", 45);
    let f = std::fs::File::create(&svg).unwrap();
    report.flamegraph(f).unwrap();
    eprintln!("flamegraph: {svg}");
}

fn short(s: &str) -> String {
    // drop the hash suffix and generic noise
    let s = s.rsplit_once("::h").filter(|(_, h)| h.len() == 16).map_or(s, |(a, _)| a);
    s.chars().take(140).collect()
}
