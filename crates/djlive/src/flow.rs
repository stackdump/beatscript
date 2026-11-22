//! Named arrangements and how a song moves between them (the flow of a program).
//!
//! `arrange open: intro build then set` / `arrange set: drop wubs loop` / `arrange close: outro end`. The flow is a small
//! deterministic walk: start at `start NAME` (else the first arrangement written); after an arrangement comes its
//! ending: `then X` goes to X, `loop` plays it again for ever, `loop N` plays it N passes in all and then falls through,
//! `end` stops, and nothing said falls through to the next arrangement written (the song ends after the last one). The
//! walk is unrolled into a list of visits. A visit state (arrangement, pass) seen twice is a cycle: the song is infinite and
//! starts again from the first visit of that state, which is where the lap begins (`loop_from`).

use crate::ast::ArrEnd;

/// What the walk needs of each arrangement.
#[derive(Clone, Debug)]
pub struct FlowNode {
    pub name: String,
    pub ends: ArrEnd,
    /// Index of the arrangement named by `then`, if it names one that exists.
    pub then: Option<usize>,
}

/// The most visits a flow unrolls to.
pub const MAX_VISITS: usize = 2048;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flow {
    /// Arrangement index of each visit, in play order.
    pub visits: Vec<usize>,
    /// The visit the song goes back to when it gets to the end of `visits`: None, the song ends there.
    pub loop_from: Option<usize>,
    /// Arrangements no walk from the start reaches.
    pub unreachable: Vec<usize>,
    /// Hit [`MAX_VISITS`].
    pub too_long: bool,
}

pub fn walk(nodes: &[FlowNode], start: usize) -> Flow {
    let mut visits: Vec<usize> = Vec::new();
    let mut seen: std::collections::BTreeMap<(usize, u32), usize> = std::collections::BTreeMap::new();
    let mut reached = vec![false; nodes.len()];
    let mut cur = Some((start, 0u32));
    let mut loop_from = None;
    let mut too_long = false;
    while let Some((a, pass)) = cur {
        if let Some(&v) = seen.get(&(a, pass)) {
            loop_from = Some(v);
            break;
        }
        if visits.len() >= MAX_VISITS {
            too_long = true;
            break;
        }
        seen.insert((a, pass), visits.len());
        visits.push(a);
        reached[a] = true;
        let fall = if a + 1 < nodes.len() { Some((a + 1, 0)) } else { None };
        cur = match &nodes[a].ends {
            ArrEnd::Fall => fall,
            ArrEnd::Then(_) => nodes[a].then.map(|t| (t, 0)),
            ArrEnd::Loop => Some((a, 0)),
            ArrEnd::Times(n, _) => {
                if pass + 1 < *n {
                    Some((a, pass + 1))
                } else {
                    fall
                }
            }
            ArrEnd::End => None,
        };
    }
    let unreachable = (0..nodes.len()).filter(|i| !reached[*i]).collect();
    Flow { visits, loop_from, unreachable, too_long }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Word;
    use crate::diag::Span;

    fn w(t: &str) -> Word {
        Word { text: t.into(), span: Span::default() }
    }
    fn node(name: &str, ends: ArrEnd, then: Option<usize>) -> FlowNode {
        FlowNode { name: name.into(), ends, then }
    }

    #[test]
    fn then_loop_end() {
        let n = vec![node("open", ArrEnd::Then(w("set")), Some(1)), node("set", ArrEnd::Loop, None), node("close", ArrEnd::End, None)];
        let f = walk(&n, 0);
        assert_eq!(f.visits, vec![0, 1]);
        assert_eq!(f.loop_from, Some(1));
        assert_eq!(f.unreachable, vec![2]);
    }

    #[test]
    fn loop_n_falls_through_and_ends() {
        let n = vec![node("a", ArrEnd::Times(3, Span::default()), None), node("b", ArrEnd::Fall, None)];
        let f = walk(&n, 0);
        assert_eq!(f.visits, vec![0, 0, 0, 1]);
        assert_eq!(f.loop_from, None);
        assert!(f.unreachable.is_empty());
    }

    #[test]
    fn then_cycle_loops_from_its_first_visit() {
        let n = vec![node("a", ArrEnd::Then(w("b")), Some(1)), node("b", ArrEnd::Then(w("a")), Some(0))];
        let f = walk(&n, 0);
        assert_eq!(f.visits, vec![0, 1]);
        assert_eq!(f.loop_from, Some(0));
    }
}
