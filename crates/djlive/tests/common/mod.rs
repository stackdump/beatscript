//! Helpers the property tests share: a small deterministic RNG and `reformat`, which writes a program out again
//! token by token with other whitespace, comments, blank lines, indentation, continuation lines and number and
//! note spellings. Every feature's property test (docs/BTW-EXTENSIONS.md) uses it, so a construct whose tokens
//! must stay glued together (`6db`, `x?0.3`, `A.kick`, `==`) is taught here, once, in `glued_pair`.
#![allow(dead_code)]

use djlive::lex::{lex, Tok};
use djlive::music;

pub struct Rng(pub u64);
impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// `src` written out again token by token with other whitespace, comments, blank lines, indentation,
/// continuation lines, number spellings and note spellings.
pub fn reformat(src: &str, rng: &mut Rng) -> String {
    let lexed = lex(src);
    let mut out = String::new();
    let mut prev_text = String::new();
    let mut line_start = true;
    let toks: Vec<_> = lexed.tokens.iter().collect();
    for (i, t) in toks.iter().enumerate() {
        match &t.tok {
            Tok::Eof => break,
            Tok::Newline => {
                if rng.below(3) == 0 {
                    out.push_str(&format!("  # {}", ["noise", "a note to self", "wobble here", "x ~ x"][rng.below(4) as usize]));
                }
                out.push('\n');
                for _ in 0..rng.below(3) {
                    out.push_str(if rng.below(2) == 0 { "\n" } else { "   # a comment line\n" });
                }
                line_start = true;
                prev_text.clear();
                continue;
            }
            _ => {}
        }
        let mut text = t.span.text(src).to_string();
        // spellings
        match &t.tok {
            Tok::Num { .. } if rng.below(2) == 0 => {
                if let Some(base) = text.strip_suffix('k') {
                    if let Ok(v) = base.parse::<f64>() {
                        text = format!("{}", (v * 1000.0) as i64);
                    }
                } else if text.contains('.') && !text.ends_with('%') {
                    text.push('0');
                }
            }
            Tok::Word(w) => {
                let after_deck = matches!(i.checked_sub(1).and_then(|j| toks.get(j)).map(|t| &t.tok), Some(Tok::Word(d)) if d == "deck");
                if !after_deck && w != "A" && w != "B" {
                    if let Some((pc, oct)) = music::parse_note(w) {
                        const FLAT: [&str; 12] = ["c", "db", "d", "eb", "e", "f", "gb", "g", "ab", "a", "bb", "b"];
                        const SHARP: [&str; 12] = ["c", "c#", "d", "d#", "e", "f", "f#", "g", "g#", "a", "a#", "b"];
                        let name = if rng.below(2) == 0 { FLAT[pc as usize] } else { SHARP[pc as usize] };
                        let name = if rng.below(3) == 0 { name.to_uppercase() } else { name.to_string() };
                        // only a bare note word (a pitch in a pattern), not a keyword that happens to parse
                        if !matches!(w.as_str(), "a" | "b" | "e" | "d" | "c" | "f" | "g") || oct.is_some() || toks.get(i + 1).map(|n| &n.tok) != Some(&Tok::LParen) {
                            let candidate = format!("{name}{}", oct.map(|o| o.to_string()).unwrap_or_default());
                            // `e(5,8)` is a generator, `a` after `deck` a deck: leave words next to `(` alone
                            if toks.get(i + 1).map(|n| &n.tok) != Some(&Tok::LParen) {
                                text = candidate;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        // whitespace before the token
        let src_adjacent = i > 0 && toks[i - 1].span.end == t.span.start;
        let glue = matches!(t.tok, Tok::Bang | Tok::Caret) || (src_adjacent && glued_pair(&toks[i - 1].tok, &t.tok)) || (matches!(t.tok, Tok::LParen) && prev_text.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '#' || c == '-') && !prev_text.is_empty());
        if line_start {
            for _ in 0..rng.below(4) {
                out.push(if rng.below(2) == 0 { ' ' } else { '\t' });
            }
            // a `|` may begin a continuation line
        } else if !glue {
            // adjacency matters for a few tokens only; spaces anywhere else are free
            if t.tok == Tok::Pipe && rng.below(3) == 0 {
                out.push_str("\n    ");
            } else {
                for _ in 0..(1 + rng.below(3)) {
                    out.push(if rng.below(5) == 0 { '\t' } else { ' ' });
                }
            }
        }
        out.push_str(&text);
        prev_text = text;
        line_start = false;
    }
    out.push('\n');
    out
}


/// Tokens that mean something only when they touch the one before them in the source: a unit and its number
/// (`6db`), `x?0.3`, `5'`, `A.kick`, the two marks of `==` `!=` `<=` `>=`, `hihat/12`, `a.out`, a glob of an
/// `arrange { }` mutation (`acid_*`, `*_a`). Whoever adds a
/// construct that depends on adjacency adds its pair here.
pub fn glued_pair(prev: &Tok, t: &Tok) -> bool {
    matches!(t, Tok::Unit(_) | Tok::Dot | Tok::Question | Tok::Apostrophe | Tok::Slash | Tok::PlusMinus | Tok::OctDown | Tok::Ghost | Tok::Amp)
        // a ratchet: `x:3` touches its step and its number (a colon touching what is before it is how the text writes it anyway)
        || matches!(t, Tok::Colon)
        || (matches!(t, Tok::Num { .. }) && matches!(prev, Tok::Colon))
        || matches!(prev, Tok::Dot | Tok::Question | Tok::Slash)
        // `c2~3` (pattern sugar, elongate): the count glued to its `~`, the `~` glued to its step (and its marks)
        || (matches!(prev, Tok::Tilde) && matches!(t, Tok::Num { .. }))
        || (matches!(t, Tok::Tilde) && matches!(prev, Tok::Word(_) | Tok::Num { .. } | Tok::RBrack | Tok::Gt | Tok::RParen | Tok::Tilde | Tok::Bang | Tok::Caret | Tok::Ghost | Tok::Amp))
        || (matches!(t, Tok::Eq) && matches!(prev, Tok::Eq | Tok::Lt | Tok::Gt | Tok::Bang))
        || (matches!(t, Tok::Star) && matches!(prev, Tok::Word(_)))
        || (matches!(t, Tok::Word(_)) && matches!(prev, Tok::Star))
        // a seed reference: `seed(` touches (seed.rs)
        || (matches!(t, Tok::LParen) && matches!(prev, Tok::Word(w) if w == "seed"))
        // and a template reference: `template(` touches (template.rs)
        || (matches!(t, Tok::LParen) && matches!(prev, Tok::Word(w) if w == "template"))
}
