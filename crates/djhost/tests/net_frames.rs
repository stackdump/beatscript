//! djnet dates the stream's samples by the master tap's frames and the beat points by the
//! mixer's rendered frames (`Engine::loop_frames`): they must be one count.
#![cfg(feature = "network")]

use djhost::deck::{engine, EngineConfig};

#[test]
fn the_master_tap_and_the_beat_points_count_the_same_frames() {
    let mut c = EngineConfig::new(48_000.0);
    c.extended = false;
    let (e, mut r) = engine(c);
    let taps = e.bus_taps();
    let (mut l, mut rr) = (vec![0f32; 256], vec![0f32; 256]);
    for k in 1..=37 {
        let n = if k % 3 == 0 { 128 } else { 256 };
        r.render_block(&mut l[..n], &mut rr[..n]);
        let tap_end = taps.master.span().map_or(0, |(_, end)| end);
        assert_eq!(tap_end, e.loop_frames(), "block {k}");
    }
}
