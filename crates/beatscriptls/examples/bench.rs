//! Check latency: how long one keystroke's analysis takes (`Analysis::new`: lex, parse, expand, compile, plus the
//! service's own passes), on the largest programs of the corpus and on synthetic hour-long songs.
//!
//! `cargo run --release -p beatscriptls --example bench [-- FILE...]`

use std::time::Instant;

use beatscriptls::Analysis;

fn median(mut ms: Vec<f64>) -> (f64, f64, f64) {
    ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (ms[ms.len() / 2], ms[0], ms[ms.len() - 1])
}

/// The syntax tier (`Analysis::new`: lex, parse, symbols: every keystroke) and the check (`full_check`: the compile, run
/// off the keystroke path), each the median of several runs.
fn time(name: &str, text: &str) {
    let runs = if text.len() > 5000 { 5 } else { 9 };
    let (mut syn, mut chk) = (Vec::new(), Vec::new());
    let mut errs = 0;
    for _ in 0..runs {
        let t = Instant::now();
        let a = Analysis::new(text);
        syn.push(t.elapsed().as_secs_f64() * 1000.0);
        let t = Instant::now();
        let c = beatscriptls::analysis::full_check(text);
        chk.push(t.elapsed().as_secs_f64() * 1000.0);
        errs = c.diagnostics.iter().filter(|d| d.is_error()).count();
        std::hint::black_box((&a, &c));
    }
    let (s, _, smax) = median(syn);
    let (c, cmin, cmax) = median(chk);
    println!("{name:<42} {:>6} B {:>4} lines | syntax {s:>6.2} ms (max {smax:>6.2}) | check {c:>8.2} ms (min {cmin:>8.2}, max {cmax:>8.2}) | errors {errs}", text.len(), text.lines().count());
}

/// An hour at 120 BPM (1800 bars): eight 16-bar sections arranged over and over.
fn hour_arranged() -> String {
    let mut t = String::from("tempo 120\nkey am\n");
    let names = ["intro", "verse", "chorus", "bridge", "drop", "breakdown", "build", "outro"];
    for (i, n) in names.iter().enumerate() {
        t.push_str(&format!(
            "section {n} 16 bars {{\n  deck A: kick e({},16) vel=90..110 | hihat x*8?0.7 | snare <~ x ~ x>\n  deck B: bass <a1 c2 e2 g2> inst=sub-bass oct=1 | melody deg(1 3 5 {}) inst=pluck\n  master: filter lp {}k\n}}\n",
            (i % 4) + 3,
            (i % 7) + 1,
            i + 2
        ));
    }
    let mut arr = Vec::new();
    while arr.len() * 16 < 1800 {
        arr.push(names[arr.len() % names.len()]);
    }
    t.push_str(&format!("arrange {}\n", arr.join(" ")));
    t
}

/// An hour of distinct sections: 113 sections of 16 bars, each its own text (the most text an hour can be in this shape).
fn hour_distinct() -> String {
    let mut t = String::from("tempo 120\nkey am\nlet groove = e(4,16)\n");
    let mut names = Vec::new();
    for i in 0..113 {
        let n = format!("part{i}");
        t.push_str(&format!(
            "section {n} 16 bars {{\n  deck A: kick groove vel=90..110 | hihat x*8?0.{} | snare <~ x ~ x>\n  deck B: bass <a1 c2 e2 g2> inst=sub-bass oct=1 | melody deg(1 3 5 {}) inst=pluck\n  every 4 bars: master: filter lp {}k\n}}\n",
            (i % 8) + 1,
            (i % 7) + 1,
            (i % 9) + 2
        ));
        names.push(n);
    }
    t.push_str(&format!("arrange {}\n", names.join(" ")));
    t
}

fn main() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let mut files: Vec<String> = std::env::args().skip(1).collect();
    if files.is_empty() {
        for dir in ["examples/btw", "crates/btwapi/tests/corpus"] {
            if let Ok(rd) = std::fs::read_dir(format!("{root}/{dir}")) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.extension().is_some_and(|x| x == "btw") {
                        files.push(p.display().to_string());
                    }
                }
            }
        }
        files.sort_by_key(|f| std::cmp::Reverse(std::fs::metadata(f).map(|m| m.len()).unwrap_or(0)));
        files.truncate(5);
    }
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap();
        time(f.rsplit('/').next().unwrap(), &text);
    }
    time("synthetic: 60 min, 8 sections arranged", &hour_arranged());
    time("synthetic: 60 min, 113 distinct sections", &hour_distinct());
}
