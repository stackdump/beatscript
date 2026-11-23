//! Writes `crates/beatscript-api/data/snapshot.json` from djhost's registry and catalog:
//! `cargo run -q -p beatscript-api --example gen_snapshot`.
#[path = "../tests/support/snapshot_gen.rs"]
mod snapshot_gen;

fn main() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data/snapshot.json");
    let text = snapshot_gen::generate();
    std::fs::write(&path, &text).expect("write snapshot.json");
    eprintln!("wrote {} ({} bytes)", path.display(), text.len());
}
