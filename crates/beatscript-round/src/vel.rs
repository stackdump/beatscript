//! The velocity of a `vel=lo..hi` hit, as the compiler picks it (`djlive::compile`, `Compiled::song_nets`):
//! `lo + fnv("{deck}:{role}:{bar}:{step}:{note}") % (hi - lo + 1)`. Replicated here (it is `pub(crate)` there)
//! so that every (lo, hi) can be tried without a compile each. `tests::agrees_with_the_compiler` pins it to
//! the compiler's own nets, so a change on that side fails here instead of silently misleading the search.

pub fn fnv(s: &str) -> u32 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    ((h >> 32) ^ h) as u32
}

pub fn pick(deck: &str, role: &str, bar: u32, step: usize, note: u8, lo: u8, hi: u8) -> u8 {
    let span = (hi as i32 - lo as i32 + 1).max(1) as u32;
    (lo as i32 + (fnv(&format!("{deck}:{role}:{bar}:{step}:{note}")) % span) as i32).clamp(1, 127) as u8
}

/// The MIDI note a drum role plays in the nets.
pub fn drum_note(role: &str) -> u8 {
    match role {
        "kick" => 36,
        "snare" => 38,
        "hihat" => 42,
        _ => 39,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beatscript::part_velocities;

    #[test]
    fn agrees_with_the_compiler() {
        let steps = [2usize, 6, 10, 14];
        for (lo, hi) in [(60u8, 90u8), (50, 100), (70, 70), (60, 91), (1, 127), (20, 33)] {
            let text = format!("#! btw 1\ntempo 124\ndeck A: hihat ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ vel={lo}..{hi}\nend at bar 4\n");
            let nets = part_velocities(&text, "A", "hihat").unwrap();
            let mine: Vec<u8> = if lo == hi { vec![lo; 4] } else { steps.iter().map(|s| pick("A", "hihat", 0, *s, 42, lo, hi)).collect() };
            assert_eq!(nets, mine, "vel={lo}..{hi}");
        }
        // a bass note's hash takes its own note number
        let text = "#! btw 1\ntempo 124\ndeck B: bass a2 ~ ~ a2 ~ ~ c3 ~ ~ ~ ~ ~ ~ ~ ~ ~ inst=sub-bass vel=40..110\nend at bar 4\n";
        let nets = part_velocities(text, "B", "bass").unwrap();
        let mine = vec![pick("B", "bass", 0, 0, 45, 40, 110), pick("B", "bass", 0, 3, 45, 40, 110), pick("B", "bass", 0, 6, 48, 40, 110)];
        assert_eq!(nets, mine);
    }

    #[test]
    fn known_first_beat_velocities() {
        // vel=60..90 on first-beat's hihat is [67, 77, 78, 89] in the nets
        let v: Vec<u8> = [2usize, 6, 10, 14].iter().map(|s| pick("A", "hihat", 0, *s, 42, 60, 90)).collect();
        assert_eq!(v, vec![67, 77, 78, 89]);
    }
}
