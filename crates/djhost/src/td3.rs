//! The TD-3's pattern slots through the `td3-midi` command (td3-rs) as a separate process: the
//! licence route of docs/HUB.md M7. djrust never links td3-rs (GPL-3.0); it writes the pattern
//! as td3's text format ([`djengine::gear::td3::Td3Pattern::to_text`]) and runs
//! `td3-midi write FILE --slot S --port P [--write --confirm S]`, which backs up all 64 slots,
//! writes one, reads it back and compares. Portable: plain `std::process`.
//!
//! The TD-3's MIDI port is busy while it writes: the caller releases its own connection first
//! (`Clock::release`) and refuses while the transport runs.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{channel, Receiver};

/// Where `td3-midi` is: `$TD3_BIN`, then `td3` or `td3-midi` on `PATH`, then a td3-rs release
/// build next to this checkout (`~/Workspace/td3-rs/target/release/td3-midi`).
pub fn find_td3() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("TD3_BIN").map(PathBuf::from) {
        return p.is_file().then_some(p);
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            for name in ["td3", "td3-midi"] {
                let p = dir.join(name);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let p = home.join("Workspace/td3-rs/target/release/td3-midi");
    p.is_file().then_some(p)
}

/// Is `slot` one `td3-midi` takes: `G-PP` (group 1-4, pattern 1-16) or `G-<1-8><A|B>`?
pub fn valid_slot(slot: &str) -> bool {
    let Some((g, p)) = slot.split_once('-') else {
        return false;
    };
    let group_ok = matches!(g, "1" | "2" | "3" | "4");
    let pat_ok = match p.parse::<u8>() {
        Ok(n) => (1..=16).contains(&n),
        Err(_) => {
            let (n, ab) = p.split_at(p.len().saturating_sub(1));
            matches!(ab, "A" | "B" | "a" | "b")
                && n.parse::<u8>().is_ok_and(|n| (1..=8).contains(&n))
        }
    };
    group_ok && pat_ok
}

/// How a write went.
#[derive(Clone, Debug, PartialEq)]
pub struct Td3Result {
    pub ok: bool,
    /// Whether it really transmitted (false: a dry run).
    pub wrote: bool,
    pub slot: String,
    /// stdout then stderr, as the command printed them.
    pub output: String,
}

/// The arguments for one write (separate so tests can check them).
pub fn write_args(file: &Path, slot: &str, port: &str, really: bool) -> Vec<String> {
    let mut a = vec![
        "write".to_string(),
        file.display().to_string(),
        "--slot".into(),
        slot.into(),
        "--port".into(),
        port.into(),
    ];
    if really {
        a.extend(["--write".into(), "--confirm".into(), slot.into()]);
    }
    a
}

/// Write `text` (td3's pattern text) into `slot` through `bin`, on a worker thread. `really`
/// false is a dry run: td3-midi parses and shows the pattern, sends nothing. The answer arrives
/// on the receiver.
pub fn write_pattern(
    bin: &Path,
    text: &str,
    slot: &str,
    port: &str,
    really: bool,
) -> Receiver<Td3Result> {
    let (tx, rx) = channel();
    let (bin, text, slot, port) = (
        bin.to_path_buf(),
        text.to_string(),
        slot.to_string(),
        port.to_string(),
    );
    std::thread::spawn(move || {
        let fail = |output: String| Td3Result {
            ok: false,
            wrote: false,
            slot: slot.clone(),
            output,
        };
        if !valid_slot(&slot) {
            let _ = tx.send(fail(format!(
                "not a TD-3 slot: {slot:?} (G-PP or G-<1-8><A|B>)"
            )));
            return;
        }
        let file = std::env::temp_dir().join(format!(
            "djrust-td3-{}-{}.txt",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis())
        ));
        if let Err(e) = std::fs::write(&file, &text) {
            let _ = tx.send(fail(format!("could not write {}: {e}", file.display())));
            return;
        }
        let out = Command::new(&bin)
            .args(write_args(&file, &slot, &port, really))
            .output();
        let _ = std::fs::remove_file(&file);
        let r = match out {
            Ok(o) => Td3Result {
                ok: o.status.success(),
                wrote: really && o.status.success(),
                slot: slot.clone(),
                output: format!(
                    "{}{}",
                    String::from_utf8_lossy(&o.stdout),
                    String::from_utf8_lossy(&o.stderr)
                ),
            },
            Err(e) => fail(format!("could not run {}: {e}", bin.display())),
        };
        let _ = tx.send(r);
    });
    rx
}

/// The SysEx that sets the TD-3's clock source to MIDI USB, so it follows the hub's clock and
/// Start / Stop (td3-rs PROTOCOL.md, verified on fw 2.0.1 2026-11-02: ACKed, then it played only
/// between Start and Stop). It changes a setting on the device, so the app sends it only when asked.
pub const CLOCK_SOURCE_USB: [u8; 10] = [0xF0, 0x00, 0x20, 0x32, 0x00, 0x01, 0x0A, 0x1B, 0x02, 0xF7];

#[cfg(test)]
mod tests {
    use super::*;
    use djengine::gear::td3::Td3Pattern;

    #[test]
    fn slots() {
        for s in ["1-1", "4-16", "4-8B", "2-1a"] {
            assert!(valid_slot(s), "{s}");
        }
        for s in ["0-1", "5-1", "4-17", "4-9A", "4-8C", "416", ""] {
            assert!(!valid_slot(s), "{s}");
        }
    }

    #[test]
    fn arguments() {
        let f = Path::new("/tmp/p.txt");
        assert_eq!(
            write_args(f, "4-16", "TD-3", false),
            ["write", "/tmp/p.txt", "--slot", "4-16", "--port", "TD-3"]
        );
        let a = write_args(f, "4-16", "TD-3", true);
        assert_eq!(a[6..], ["--write", "--confirm", "4-16"]);
    }

    /// A dry run through the real td3-midi, when one is installed: it must accept the text.
    /// Sends nothing to any device.
    #[test]
    fn td3_midi_accepts_the_text_in_a_dry_run() {
        let Some(bin) = find_td3() else {
            eprintln!("td3-midi not found: skipped");
            return;
        };
        let text = Td3Pattern::acid(7.0, 9).to_text();
        let r = write_pattern(&bin, &text, "4-16", "TD-3", false)
            .recv()
            .unwrap();
        assert!(r.ok && !r.wrote, "{}", r.output);
    }
}
