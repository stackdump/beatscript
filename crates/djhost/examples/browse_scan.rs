//! `cargo run -p djhost --example browse_scan -- [QUERY] [--kind K]`: scan the browser's
//! sources (Apple Loops, ~/Music) without the cache and print how long it took, the packs and
//! the first matches.
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let kind = args.iter().position(|a| a == "--kind").and_then(|i| args.get(i + 1)).and_then(|k| djhost::browse::Kind::parse(k));
    let text = args.first().filter(|a| !a.starts_with("--")).cloned().unwrap_or_default();
    let t = std::time::Instant::now();
    let entries = djhost::browse::scan(&djhost::browse::roots(), &Default::default());
    println!("{} files in {:.2}s", entries.len(), t.elapsed().as_secs_f64());
    for (p, n) in djhost::browse::packs(&entries, None).iter().take(12) {
        println!("  {p}: {n}");
    }
    let q = djhost::browse::Query { text, kind, ..Default::default() };
    for e in entries.iter().filter(|e| q.admits(e)).take(10) {
        println!("{} [{}] {}", e.name, e.pack, e.detail());
    }
}
