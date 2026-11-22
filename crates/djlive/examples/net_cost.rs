//! What a long net costs: an N-bar program (deck A five parts, deck B three, every part its own N-bar
//! alternation so the net really is N bars) compiled and timed stage by stage, with the size of what it makes.
//!
//!   cargo run --release -p djlive --example net_cost -- 64 128 256 512
//!
//! (docs/BTW.md, "How long a program can be": the numbers there are this example's.)

use std::time::Instant;

fn bar(role: &str, k: usize) -> String {
    let v = k % 7;
    match role {
        "kick" => ["x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~", "x ~ ~ x x ~ ~ ~ x ~ x ~ x ~ ~ ~"][v % 2].to_string(),
        "snare" => ["~ ~ ~ ~ x ~ ~ ~ ~ ~ ~ ~ x ~ ~ ~", "~ ~ ~ ~ x ~ ~ x ~ ~ ~ ~ x ~ x ~"][v % 2].to_string(),
        "hihat" => ["x ~ x ~ x ~ x ~ x ~ x ~ x ~ x ~", "x x x x x x x x x x x x x x x x"][v % 2].to_string(),
        "bass" => ["a2 ~ a2 a2 ~ a2 ~ a2 e2 ~ e2 e2 ~ e2 ~ e2", "f2 ~ f2 f2 ~ f2 ~ f2 c2 ~ c2 c2 ~ c2 ~ c2", "g2 ~ g2 g2 ~ g2 ~ g2 d2 ~ d2 d2 ~ d2 ~ d2"][v % 3].to_string(),
        "melody" => ["e5 ~ ~ ~ d5 ~ c5 ~ ~ ~ ~ ~ a4 ~ ~ ~", "g5 ~ ~ ~ e5 ~ c5 ~ ~ ~ ~ ~ ~ ~ ~ ~", "~ ~ ~ ~ b4 ~ ~ ~ d5 ~ ~ ~ a5 ~ ~ ~"][v % 3].to_string(),
        "arp" => "a4 c5 e5 a5 e5 c5 a4 c5 a4 c5 e5 a5 e5 c5 a4 c5".to_string(),
        _ => "chord(am)".to_string(),
    }
}

fn part(role: &str, n: usize, extra: &str) -> String {
    let terms: Vec<String> = (0..n).map(|k| if role == "harmony" { bar(role, k) } else { format!("[{}]", bar(role, k)) }).collect();
    // a prime-ish stride so the alternation does not fold into a shorter period
    format!("{role} <{}> {extra}", terms.join(" "))
}

fn program(n: usize) -> String {
    format!(
        "tempo 124\nkey am\ndeck A: {} | {} | {} | {} | {}\ndeck B: {} | {} | {}\n",
        part("kick", n, "vel=112"),
        part("snare", n, "vel=80..110"),
        part("hihat", n, "vel=70..95"),
        part("bass", n, "inst=sub-bass len=1"),
        part("melody", n, "vel=105 len=8"),
        part("arp", n, "vel=70..95 len=1"),
        part("harmony", n, "inst=warm-pad len=16 vel=80"),
        part("melody", n, "inst=detuned-saw vel=90"),
    )
}

fn main() {
    // `dump N FILE` writes the N-bar program (for `djctl btw render`)
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("dump") {
        let n: usize = args[1].parse().expect("bars");
        std::fs::write(&args[2], program(n)).expect("write");
        return;
    }
    let sizes: Vec<usize> = std::env::args().skip(1).filter_map(|a| a.parse().ok()).collect();
    let sizes = if sizes.is_empty() { vec![64, 128, 256, 512] } else { sizes };
    println!("{:>5} {:>10} {:>10} {:>11} {:>11} {:>9} {:>8}", "bars", "text KB", "check ms", "net KB (A+B)", "envelope ms", "cid ms", "nets");
    for n in sizes {
        let src = program(n);
        let t = Instant::now();
        let c = djlive::check(&src);
        let check_ms = t.elapsed().as_secs_f64() * 1e3;
        let Some(comp) = c.compiled else {
            println!("{n:>5}  refused: {}", c.diagnostics.iter().find(|d| d.is_error()).map_or(String::new(), |d| d.message.clone()));
            continue;
        };
        let mut bytes = 0usize;
        let mut env_ms = 0.0;
        let mut cid_ms = 0.0;
        let mut nets = 0;
        let segs: Vec<_> = if comp.arranged { comp.segments.iter().map(|s| &s.decks).collect() } else { vec![&comp.decks] };
        for decks in segs {
            for ir in decks.values() {
                let t = Instant::now();
                let e = ir.envelope(0);
                env_ms += t.elapsed().as_secs_f64() * 1e3;
                bytes += e.canonical_js().len();
                let t = Instant::now();
                let _ = ir.net_cid();
                cid_ms += t.elapsed().as_secs_f64() * 1e3;
                nets += 1;
            }
        }
        println!("{:>5} {:>10.1} {:>10.1} {:>11.1} {:>11.1} {:>9.1} {:>8}", n, src.len() as f64 / 1024.0, check_ms, bytes as f64 / 1024.0, env_ms, cid_ms, nets);
    }
}
