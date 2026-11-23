//! Sentences of the grammar: programs written from `grammar.ebnf` alone, by a seeded random walk of its productions.
//! The other direction of the drift test (tests/grammar.rs): every sentence must be a program the parser reads, so the
//! grammar cannot describe more than the language is.
//!
//! The walk is bounded: past a depth every production takes its shortest way out (worked out once, as a fixed point),
//! and a repetition runs at most twice. A token class writes a sample of itself ([`sample`]); tokens are joined with a
//! space, but for the marks the lexer reads only when they touch the token before them ([`glued`]).

use std::collections::HashMap;

use super::ebnf::{Grammar, Node};

/// splitmix64: deterministic, so a failing sentence is the same sentence on every run.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed ^ 0x9e37_79b9_7f4a_7c15)
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

/// A sample of a token class, as the lexer would read it back as one of that class.
pub fn sample(class: &str, rng: &mut Rng) -> String {
    let pick = |rng: &mut Rng, xs: &[&str]| xs[rng.below(xs.len())].to_string();
    match class {
        "NEWLINE" => "\n".into(),
        "NAME" => pick(rng, &["riff", "groove", "lead2", "wash", "drop_b"]),
        "WORD" => pick(rng, &["sub-bass", "warm-pad", "on"]),
        // numbers that are in range wherever the grammar puts them (a phase is 0 to 1, `step N` is 2 up): ranges are the
        // checker's and the parser's, not the grammar's
        "INT" => pick(rng, &["2", "4", "8"]),
        "NUMBER" => pick(rng, &["0.5", "0.25", "1", "50%"]),
        "DECIMAL" => pick(rng, &["6", "0.5", "30"]),
        "STRING" => "\"hello world\"".into(),
        "PARAM" => "$n".into(),
        "NOTE" => pick(rng, &["c3", "f#2", "bb4", "a"]),
        "PITCH" => pick(rng, &["c4", "f#2"]),
        "KEY" => pick(rng, &["am", "c", "f#m", "d:dorian"]),
        "CHORD" => pick(rng, &["am7", "cmaj7", "g7"]),
        "ROMAN" => pick(rng, &["i", "VI", "V7", "iv"]),
        "SDEG" => pick(rng, &["b3", "#4"]),
        "UNIT" => pick(rng, &["ms", "s"]),
        "TOKEN" => pick(rng, &["x", "~", "c2"]),
        "GLOB" => "acid_*".into(),
        "SUGAR" => pick(rng, &["vocoder", "freeze"]),
        "PHRASE" => pick(rng, &["@hands_up", "@feel_the_bass", "@drop_it", "@tempest_such_stuff"]),
        "GEN" => pick(rng, &["gen@1", "gen@2", "gen"]),
        "MACRO" => pick(rng, &["macro", "macro@1"]),
        "SEED" => pick(rng, &["seed", "seed@2", "seed@1"]),
        "GENRE" => pick(rng, &["genre@1", "genre"]),
        // a ghost's `.` touches its step, and nothing touches it (a `.` before a word is `A.kick`'s): marked for [`join`]
        "GHOST" => format!("{GHOST}."),
        "RATCHET" => pick(rng, &[":2", ":3", ":4"]),
        "GEN" => pick(rng, &["gen@2", "gen@1", "gen"]),
        // the classes of a lyric's text: no program sentence reaches them (the lyric is one STRING), but a sample is cheap
        "TEXT" => pick(rng, &["la", ",", "-"]),
        "SPACE" => " ".into(),
        "PHONES" => pick(rng, &["h ii", "w ee - aa: r", "_"]),
        other => format!("<{other}>"),
    }
}

/// Marks the lexer reads only touching the token before them: `x!` `c2^` `5'` `x?0.3` `A.kick` `6db` `5ms` `hihat/12`
/// `x*4`, and the token after a `.` or a `name=` (`seed=7`).
pub fn glued(prev: &str, next: &str) -> bool {
    let word_end = prev.chars().last().is_some_and(|c| c.is_alphanumeric() || c == '\u{b0}');
    // a unit glues to a number, not to a percent: `50%db` is one token the lexer cannot read, `50% db` is the value then
    // its unit (as `(1 + 2)db` is)
    if prev.ends_with('%') && matches!(next, "db" | "ms" | "s" | "m") {
        return false;
    }
    let count = next.chars().next().is_some_and(|c| c.is_ascii_digit());
    // a chance after its `?` (a bare `?` is a half: `x? ~`); a ratchet's `:N` is one token, glued to its step (`c2~3`,
    // the elongate's count, is left apart: a `~` that stands alone is a rest, and `x ~ 3` is three steps the parser
    // reads as well)
    let chance = prev == "?" && (count || next == "(");
    matches!(next, "!" | "^" | "&" | "'" | "?" | "." | "db" | "ms" | "s" | "m" | "*")
        || (next == "/" && word_end)
        || (next.starts_with(':') && next.len() > 1)
        || prev.ends_with('=')
        || prev == "."
        || prev == "*"
        || chance
        || prev == "/"
}

/// A `(` that goes on a sequence after a name (a call: `e(`, `deg(`, `knob(`) touches it; one that opens a sequence (a
/// group: `x (1 + 2)`) does not. Marked with a leading NUL until [`join`].
const GLUE: char = '\0';
/// A ghost's `.`: it touches the token before it and nothing touches it. Marked with a leading U+0003 until [`join`].
const GHOST: char = '\u{3}';

/// The fewest tokens each production can be (None: it never ends), worked out to a fixed point.
fn shortest(g: &Grammar) -> HashMap<String, usize> {
    let mut best: HashMap<String, usize> = HashMap::new();
    fn len(n: &Node, best: &HashMap<String, usize>, g: &Grammar) -> Option<usize> {
        match n {
            Node::Term(_) => Some(1),
            Node::Ref(r) => match g.rule(r) {
                Some(_) => best.get(r).copied(),
                None => Some(1),
            },
            Node::Seq(v) => v.iter().try_fold(0usize, |a, x| len(x, best, g).map(|l| a + l)),
            Node::Choice(v) => v.iter().filter_map(|x| len(x, best, g)).min(),
            Node::Opt(_) | Node::Rep(_) => Some(0),
        }
    }
    loop {
        let mut changed = false;
        for r in &g.rules {
            if let Some(l) = len(&r.node, &best, g) {
                if best.get(&r.name).is_none_or(|b| l < *b) {
                    best.insert(r.name.clone(), l);
                    changed = true;
                }
            }
        }
        if !changed {
            return best;
        }
    }
}

/// `x { "," x }`.
fn is_list(v: &[Node]) -> bool {
    match v {
        [x @ Node::Ref(_), Node::Rep(r)] => matches!(&**r, Node::Seq(s) if s.len() == 2 && s[0] == Node::Term(",".into()) && s[1] == *x),
        _ => false,
    }
}

/// Does the node name `class` anywhere (not through other productions)?
fn mentions(n: &Node, class: &str) -> bool {
    let mut r = Vec::new();
    n.refs(&mut r);
    r.iter().any(|x| x == class)
}

/// A writer of sentences.
pub struct Writer<'g> {
    g: &'g Grammar,
    short: HashMap<String, usize>,
    /// How many def or repeat bodies the walk is inside.
    body: std::cell::Cell<usize>,
    /// Every production a walk has written (the drift test asks that each is).
    pub visited: std::cell::RefCell<std::collections::BTreeSet<String>>,
    /// Past this depth, the shortest way out.
    pub max_depth: usize,
}

impl<'g> Writer<'g> {
    pub fn new(g: &'g Grammar) -> Writer<'g> {
        Writer { g, short: shortest(g), body: std::cell::Cell::new(0), visited: Default::default(), max_depth: 7 }
    }

    /// Does the node name, directly or within two steps, a production no walk has written yet?
    fn leads_somewhere_new(&self, n: &Node) -> bool {
        let seen = self.visited.borrow();
        let mut r = Vec::new();
        n.refs(&mut r);
        // (a production that needs a host is never written, so it is never new: it does not pull the walk toward it)
        let host = |x: &str| self.g.rule(x).is_some_and(|ru| ru.source.contains("[host]"));
        r.iter().any(|x| {
            !host(x)
                && self.g.rule(x).is_some_and(|ru| {
                    if !seen.contains(x) {
                        return true;
                    }
                    let mut r2 = Vec::new();
                    ru.node.refs(&mut r2);
                    r2.iter().any(|y| self.g.rule(y).is_some() && !host(y) && !seen.contains(y))
                })
        })
    }

    /// The productions a walk can reach from `start` without one that needs a host.
    pub fn reachable(&self, start: &str) -> std::collections::BTreeSet<String> {
        let mut out = std::collections::BTreeSet::new();
        let mut todo = vec![start.to_string()];
        while let Some(n) = todo.pop() {
            let Some(r) = self.g.rule(&n) else { continue };
            if r.source.contains("[host]") || !out.insert(n) {
                continue;
            }
            let mut refs = Vec::new();
            r.node.refs(&mut refs);
            todo.extend(refs);
        }
        out
    }

    /// Does the node name a production that needs a host's store (its note says `[host]`)?
    fn needs_host(&self, n: &Node) -> bool {
        let mut r = Vec::new();
        n.refs(&mut r);
        r.iter().any(|x| self.g.rule(x).is_some_and(|ru| ru.source.contains("[host]")))
    }

        fn body_enter(&self) {
        self.body.set(self.body.get() + 1);
    }

    fn body_leave(&self) {
        self.body.set(self.body.get() - 1);
    }

    fn walk_rule(&self, r: &str, depth: usize, rng: &mut Rng, out: &mut Vec<String>, goes_on: bool) {
        match self.g.rule(r) {
            Some(rule) => {
                self.visited.borrow_mut().insert(r.to_string());
                self.walk_in(&rule.node, depth + 1, rng, out, false)
            }
            None => out.push(sample(r, rng)),
        }
    }

        fn node_len(&self, n: &Node) -> usize {
        match n {
            Node::Term(_) => 1,
            Node::Ref(r) => self.short.get(r).copied().unwrap_or(if self.g.rule(r).is_some() { usize::MAX / 4 } else { 1 }),
            Node::Seq(v) => v.iter().map(|x| self.node_len(x)).fold(0usize, |a, b| a.saturating_add(b)),
            Node::Choice(v) => v.iter().map(|x| self.node_len(x)).min().unwrap_or(0),
            Node::Opt(_) | Node::Rep(_) => 0,
        }
    }

    fn walk(&self, n: &Node, depth: usize, rng: &mut Rng, out: &mut Vec<String>) {
        self.walk_in(n, depth, rng, out, false)
    }

    /// `goes_on`: the node is not the first of the sequence it is in.
    fn walk_in(&self, n: &Node, depth: usize, rng: &mut Rng, out: &mut Vec<String>, goes_on: bool) {
        let deep = depth >= self.max_depth;
        // a def's or a repeat's body: a `$name` may stand anywhere there (it is filled in when the body is expanded); the
        // parser leaves the body as tokens until then, and refuses a `$name` anywhere else
        if let Node::Ref(r) = n {
            if r == "def_line" || r == "body_line" {
                self.body_enter();
                self.walk_rule(r, depth, rng, out, goes_on);
                self.body_leave();
                return;
            }
        }
        match n {
            Node::Term(t) => {
                if t == "(" && goes_on && out.last().is_some_and(|p| p.chars().last().is_some_and(|c| c.is_alphanumeric())) {
                    out.push(format!("{GLUE}("));
                } else if !t.is_empty() {
                    out.push(t.clone());
                }
            }
            Node::Ref(r) => self.walk_rule(r, depth, rng, out, goes_on),
            // `x { "," x }`: a list whose items are distinct (ports, names: the parser refuses one said twice)
            Node::Seq(v) if is_list(v) => {
                let mut seen: Vec<Vec<String>> = Vec::new();
                let mut first = Vec::new();
                self.walk_in(&v[0], depth, rng, &mut first, goes_on);
                seen.push(first.clone());
                out.extend(first);
                let n = if deep { 0 } else { rng.below(3) };
                for _ in 0..n {
                    for _try in 0..5 {
                        let mut item = Vec::new();
                        self.walk_in(&v[0], depth, rng, &mut item, true);
                        if !seen.contains(&item) {
                            out.push(",".into());
                            out.extend(item.clone());
                            seen.push(item);
                            break;
                        }
                    }
                }
            }
            Node::Seq(v) => {
                let mut i = 0;
                while i < v.len() {
                    if i + 1 < v.len() && is_list(&v[i..i + 2]) {
                        self.walk_in(&Node::Seq(v[i..i + 2].to_vec()), depth, rng, out, i > 0 || goes_on);
                        i += 2;
                    } else {
                        self.walk_in(&v[i], depth, rng, out, i > 0 || goes_on);
                        i += 1;
                    }
                }
            }
            Node::Choice(v) => {
                // outside a body, the alternatives with no `$name` in them; never one that needs a host ([host]); never a
                // minus or a modulo (the grammar cannot know a value: a length, a count or a bar is not negative or 0)
                let ok: Vec<usize> = (0..v.len())
                    .filter(|&i| self.body.get() > 0 || !mentions(&v[i], "PARAM"))
                    .filter(|&i| !self.needs_host(&v[i]) && v[i] != Node::Term("-".into()) && v[i] != Node::Term("%".into()))
                    .collect();
                let ok = if ok.is_empty() { (0..v.len()).collect() } else { ok };
                // half the time, an alternative that leads to a production no walk has written yet: every corner of the
                // grammar gets its sentences
                let fresh: Vec<usize> = ok.iter().copied().filter(|&i| self.leads_somewhere_new(&v[i])).collect();
                let i = if deep {
                    ok.iter().copied().min_by_key(|&i| self.node_len(&v[i])).unwrap_or(0)
                } else if !fresh.is_empty() && rng.below(2) == 0 {
                    fresh[rng.below(fresh.len())]
                } else {
                    ok[rng.below(ok.len())]
                };
                self.walk_in(&v[i], depth, rng, out, goes_on);
            }
            Node::Opt(x) if self.needs_host(x) || **x == Node::Term("-".into()) => {}
            Node::Opt(x) => {
                if !deep && (rng.below(2) == 0 || self.leads_somewhere_new(x)) {
                    self.walk_in(x, depth, rng, out, goes_on);
                }
            }
            // a repetition of options (`key=c`, `release=5ms`, `vel=70`): each is given at most once, in any order (the
            // parser refuses one given twice; EBNF cannot say "each once, any order" short of every permutation)
            Node::Rep(x) => {
                let n = if deep { 0 } else { rng.below(3) };
                let mut keys: Vec<String> = Vec::new();
                for _ in 0..n {
                    let mut item = Vec::new();
                    self.walk_in(x, depth, rng, &mut item, goes_on);
                    let key = match item.as_slice() {
                        [k, ..] if k.ends_with('=') => Some(k.clone()),
                        [k, e, ..] if e == "=" => Some(k.clone()),
                        _ => None,
                    };
                    if let Some(k) = key {
                        if keys.contains(&k) {
                            continue;
                        }
                        keys.push(k);
                    }
                    out.extend(item);
                }
            }
        }
    }

    /// The tokens of one sentence of `start`.
    pub fn tokens(&self, start: &str, rng: &mut Rng) -> Vec<String> {
        let mut out = Vec::new();
        self.visited.borrow_mut().insert(start.to_string());
        if let Some(r) = self.g.rule(start) {
            self.walk(&r.node, 0, rng, &mut out);
        }
        out
    }

    /// One sentence of `start` as text.
    pub fn sentence(&self, start: &str, rng: &mut Rng) -> String {
        join(&self.tokens(start, rng))
    }
}

/// Tokens as text: a space between two, none where [`glued`], a line's end as it is.
pub fn join(toks: &[String]) -> String {
    let mut s = String::new();
    let mut prev = String::from("\n");
    for t in toks {
        if let Some(r) = t.strip_prefix(GHOST) {
            s.push_str(r);
            prev = GHOST.to_string();
            continue;
        }
        let (t, glue) = match t.strip_prefix(GLUE) {
            Some(r) => (r.to_string(), true),
            None => (t.clone(), false),
        };
        if !(glue || prev == "\n" || t == "\n" || glued(&prev, &t)) {
            s.push(' ');
        }
        s.push_str(&t);
        prev = t;
    }
    s
}
