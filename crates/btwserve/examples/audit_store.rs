//! List the stored programs the storage gate would refuse today (read-only: nothing is changed or deleted).
//!
//! `cargo run -p btwserve --example audit_store -- <data>/programs`

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "data/programs".into());
    let bad = btwserve::gate::audit_dir(std::path::Path::new(&dir));
    let total = std::fs::read_dir(&dir).map(|rd| rd.flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".json")).count()).unwrap_or(0);
    for (name, why) in &bad {
        println!("{name}\t{why}");
    }
    eprintln!("{} of {total} stored programs would not be admitted today", bad.len());
}
