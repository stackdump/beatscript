//! `cargo run -p djhost --features decode --example decode_probe -- FILE... [--dump OUT]`:
//! decode audio files as a sample deck does and print their length, rate and channels;
//! --dump writes the last one's left channel as raw f32 (to compare against another decoder).
fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let dump = args.iter().position(|a| a == "--dump").map(|i| {
        let p = args[i + 1].clone();
        args.drain(i..=i + 1);
        p
    });
    let mut last = None;
    for p in &args {
        match djhost::decode::decode_native(std::path::Path::new(p), true) {
            Ok(n) => {
                println!("{p}: {} frames @ {:?} stereo={}", n.left.len(), n.rate, n.right.is_some());
                last = Some(n.left);
            }
            Err(e) => println!("{p}: ERR {e}"),
        }
    }
    if let (Some(out), Some(l)) = (dump, last) {
        let bytes: Vec<u8> = l.iter().flat_map(|v| v.to_le_bytes()).collect();
        std::fs::write(out, bytes).unwrap();
    }
}
