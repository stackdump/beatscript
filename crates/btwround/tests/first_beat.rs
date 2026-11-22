//! Rung 1 of docs/ROUNDTRIP.md: render first-beat, recover a program from the audio alone, and it must be the
//! same song (`same_as`), sample identical when rendered, with the original's own CID.
//!
//! Needs a built `btwd` (`cargo build --release -p btwd`, or `$BTWD`) and takes a few minutes (hundreds of
//! renders), so it is `#[ignore]`d:
//!
//!     cargo test --release -p btwround -- --ignored --nocapture

use btwround::btwd::{self, Btwd};
use btwround::recover::{recover, Options};

#[test]
#[ignore = "needs a built btwd and takes minutes: cargo test --release -p btwround -- --ignored"]
fn first_beat_round_trips() {
    let bin = Btwd::find_bin();
    if !bin.exists() {
        eprintln!("no btwd at {} (build it or set BTWD): skipping", bin.display());
        return;
    }
    let dir = std::env::temp_dir().join(format!("btwround-test-{}", std::process::id()));
    let btwd = Btwd::new(bin, &dir).unwrap();
    let original = include_str!("../fixtures/first-beat.btw");

    // The audio is all the recovery sees; the program text is only the answer key.
    let audio = btwd.render(original).unwrap();
    let mut opts = Options::default();
    opts.jobs = 3;
    opts.verbose = true;
    let rep = recover(&audio, &btwd, &opts, Some(original)).unwrap();
    eprintln!("{}", rep.text);

    assert!(rep.l0.identical(), "L0: {:?}", rep.l0);
    let (same, diff) = rep.same_as.clone().unwrap();
    assert!(same, "L1: {diff:?}");
    let (ok_a, cid_a) = btwd::check(&rep.text).unwrap();
    let (ok_b, cid_b) = btwd::check(original).unwrap();
    assert!(ok_a && ok_b);
    assert_eq!(cid_a, cid_b, "the recovered program is the original program");
    let _ = std::fs::remove_dir_all(&dir);
}
