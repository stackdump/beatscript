//! `cargo run -p djlive --example control_net -- FILE.btw [BARS]`: what a program's control lines come to as a net
//! (docs/design/NETS-ALL-THE-WAY-DOWN.md): its size, its CID, the events it fires, and what stays beside it.

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("a .btw file");
    let src = std::fs::read_to_string(&path).expect("readable");
    let c = djlive::check(&src);
    let Some(comp) = c.compiled else {
        eprintln!("{}", djlive::diag::render_all(&c.diagnostics, &src));
        std::process::exit(1);
    };
    let bars: u32 = a.next().and_then(|b| b.parse().ok()).or(comp.song_bars()).or(comp.end).unwrap_or(16);
    let t = std::time::Instant::now();
    let n = djlive::nets::control_net(&comp, bars);
    let built = t.elapsed();
    let (p, tr, ar) = n.size();
    let t = std::time::Instant::now();
    let ev = n.events(None);
    let ran = t.elapsed();
    println!("{path}: {bars} bars, {} control lines", comp.controls.len());
    println!("  in the net: {} lines; beside it: {}", n.lines.len(), n.residual.len());
    for (k, why) in &n.residual {
        println!("    beside: {k}  ({why})");
    }
    println!("  net: {p} places, {tr} transitions, {ar} arcs; json {} bytes", n.project.canonical_js().len());
    println!("  cid {}  model cid {}", n.cid(), n.model_cid());
    println!("  performance cid {}", djlive::nets::performance_cid(&comp, bars));
    println!("  {} events over {} ticks; built in {built:?}, run in {ran:?}", ev.len(), n.ticks);
}
