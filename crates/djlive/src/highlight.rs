//! Syntax classes for the editor: what each stretch of the text is, so a window can colour it.

use crate::diag::Span;
use crate::lex::{lex, Tok};
use crate::parse::{ACTION_WORDS, PARAMS, ROLES};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Comment,
    /// `deck`, `master`, `every`, `on`, `at`, `tempo`, `key`, `swing`, `over`, `bars` ...
    Keyword,
    /// kick, snare, bass ...
    Role,
    /// swap, filter, level ... what a control line does.
    Action,
    /// A deck name, A or B.
    Deck,
    Number,
    /// `x` and a note or a degree.
    Step,
    Rest,
    /// A generator: e(...), markov(...).
    Generator,
    /// `vel=`, `oct=` ...
    Param,
    Operator,
    Word,
}

const KEYWORDS: [&str; 38] = [
    "deck", "master", "every", "on", "at", "bar", "bars", "phrase", "over", "tempo", "key", "swing", "with", "pairs", "pair", "group", "bpm", "wire", "emit", "in", "out", "chords", "drums", "rhythm", "pattern", "when", "and", "or", "not", "beat", "beats", "from", "sing", "set", "until", "station", "forms", "next"];

/// Every token's class, in text order (comments included, whitespace left out).
pub fn highlight(src: &str) -> Vec<(Span, Class)> {
    let lexed = lex(src);
    let mut out: Vec<(Span, Class)> = lexed.comments.iter().map(|s| (*s, Class::Comment)).collect();
    let t = &lexed.tokens;
    // where a statement starts, and whether it is a def's header (its ports are keywords there) or an `assert`
    let (mut at_start, mut in_header, mut in_assert) = (true, false, false);
    // a line of an `arrange { }` block that starts with `~` or `@` is a mutation: its mark and its operators are syntax
    let mut mutation_line = false;
    for (i, tok) in t.iter().enumerate() {
        let next = t.get(i + 1).map(|n| &n.tok);
        let prev = if i > 0 { Some(&t[i - 1].tok) } else { None };
        if at_start {
            mutation_line = matches!(tok.tok, Tok::Tilde | Tok::At);
        }
        if mutation_line {
            let class = match &tok.tok {
                Tok::Tilde | Tok::At => Some(Class::Keyword),
                Tok::Word(w) if crate::mutate::operator_args(w).is_some() => Some(Class::Action),
                _ => None,
            };
            if let Some(c) = class {
                out.push((tok.span, c));
                at_start = false;
                continue;
            }
        }
        if at_start {
            if let Tok::Word(w) = &tok.tok {
                in_header = w == "def";
                in_assert = w == "assert";
            }
        }
        match &tok.tok {
            Tok::Newline => {
                at_start = true;
                in_header = false;
                in_assert = false;
            }
            Tok::LBrace => {
                at_start = false;
                in_header = false;
            }
            _ => at_start = false,
        }
        // a word of the extensions that names a let, a def or a section too is a keyword where the language reads it as syntax
        let name_here = |w: &str| -> bool {
            let freed = crate::parse::EXT_WORDS.contains(&w) && !crate::parse::EXT_RESERVED.contains(&w) && !matches!(w, "voice" | "tempo");
            freed && !(in_header && matches!(w, "in" | "out" | "chords" | "drums" | "rhythm" | "pattern")) && !crate::parse::is_syntax_here(w, t, i, &t[..i], in_assert)
        };
        let class = match &tok.tok {
            Tok::Word(w) => {
                if crate::algebra::WORDS.contains(&w.as_str()) {
                    Class::Keyword
                } else if w == "control" && crate::parse::control_head_at(next, prev) {
                    // the control deck's head, as `deck` and `master` are (a `control` anywhere else is a name)
                    Class::Keyword
                } else if matches!(w.as_str(), "period" | "length" | "ends") && matches!(prev, Some(Tok::Word(a)) if a == "assert") {
                    // (`assert period(drop)` is the assertion's word, not a generator)
                    Class::Keyword
                } else if next == Some(&Tok::LParen) {
                    Class::Generator
                } else if (next == Some(&Tok::Eq) && (PARAMS.contains(&w.as_str()) || crate::mixmoves::PARAMS.contains(&w.as_str()) || crate::shape::LFO_PARAMS.contains(&w.as_str()) || crate::sing::PARAMS.contains(&w.as_str()))) || w == "voice-lead" {
                    // `voice-lead` (F5) is a part's switch, bare or `=on`; `voice=` (F9) names a sung voice
                    Class::Param
                } else if w == "A" || w == "B" {
                    Class::Deck
                } else if ROLES.contains(&w.as_str()) {
                    Class::Role
                } else if (KEYWORDS.contains(&w.as_str()) || crate::chance::HIGHLIGHT_KEYWORDS.contains(&w.as_str()) || crate::mixmoves::KEYWORDS.contains(&w.as_str()) || crate::shape::HIGHLIGHT_KEYWORDS.contains(&w.as_str()) || crate::assertion::HIGHLIGHT_KEYWORDS.contains(&w.as_str()) || (w == "pan" && matches!(prev, Some(Tok::Word(d)) if d == "A" || d == "B" || d == "master"))) && !name_here(w) {
                    Class::Keyword
                } else if ACTION_WORDS.contains(&w.as_str()) || (w == "drive" && !name_here(w)) {
                    Class::Action
                } else if w == "x" || crate::music::parse_note(w).is_some() || crate::scale::is_roman(w) || crate::scale::degree_word(w).is_some() {
                    // a note, a Roman numeral (`ii°`, `V7`) or a flat / sharp degree in `deg( )` (F5)
                    Class::Step
                } else {
                    Class::Word
                }
            }
            Tok::Num { .. } | Tok::Unit(_) => Class::Number,
            Tok::Tilde => Class::Rest,
            Tok::Newline | Tok::Eof => continue,
            _ => Class::Operator,
        };
        out.push((tok.span, class));
    }
    out.sort_by_key(|(s, _)| s.start);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classes(src: &str) -> Vec<(&str, Class)> {
        highlight(src).into_iter().map(|(s, c)| (s.text(src), c)).collect()
    }

    #[test]
    fn a_line_is_coloured_by_what_each_word_is() {
        let c = classes("deck A: kick e(5,8) vel=90 # hi");
        assert_eq!(c[0], ("deck", Class::Keyword));
        assert_eq!(c[1], ("A", Class::Deck));
        assert_eq!(c[3], ("kick", Class::Role));
        assert_eq!(c[4], ("e", Class::Generator));
        assert!(c.contains(&("vel", Class::Param)));
        assert_eq!(c.last().unwrap().1, Class::Comment);
        // the words of arithmetic, guards and sub-bar time
        let c = classes("at bar 2 beat 3 from: master: level min(1, 2) when $i == 1 and not 2 < 3");
        let class_of = |w: &str| c.iter().find(|(t, _)| *t == w).map(|(_, k)| *k);
        assert_eq!((class_of("beat"), class_of("when"), class_of("and"), class_of("not"), class_of("from")), (Some(Class::Keyword), Some(Class::Keyword), Some(Class::Keyword), Some(Class::Keyword), Some(Class::Keyword)));
        assert_eq!(class_of("min"), Some(Class::Generator));
        // a set's words: `set` and `until` where they are syntax, `mix` on a set's mix line; and where they are names, names
        let c = classes("set {\n  play a\n  mix 4 bars with crossfade\n  play b until 20m\n}");
        let class_of = |w: &str| c.iter().find(|(t, _)| *t == w).map(|(_, k)| *k);
        assert_eq!((class_of("set"), class_of("mix"), class_of("until"), class_of("play"), class_of("with")), (Some(Class::Keyword), Some(Class::Keyword), Some(Class::Keyword), Some(Class::Action), Some(Class::Keyword)));
        let c = classes("let set = 2\nlet until = 3\nsection mix 4 bars {\n  deck A: kick x\n}");
        assert!(c.iter().filter(|(t, _)| matches!(*t, "set" | "until" | "mix")).all(|(_, k)| *k == Class::Word), "{c:?}");
        // an arrange block's mutation lines: the mark and the operators are syntax, the targets names, a section a name
        let c = classes("arrange {\n  intro\n  ~ acid_a rot 2 | thin 0.1, hoov up 2\n  @ hoov rev\n}");
        let class_of = |w: &str| c.iter().find(|(t, _)| *t == w).map(|(_, k)| *k);
        assert_eq!((class_of("~"), class_of("@"), class_of("rot"), class_of("thin"), class_of("rev"), class_of("acid_a")), (Some(Class::Keyword), Some(Class::Keyword), Some(Class::Action), Some(Class::Action), Some(Class::Action), Some(Class::Word)));
        // a count after the mark (`~3`, `@4`: every Nth pass) is a number, and the rest of the line reads as before
        let c = classes("arrange {\n  intro\n  ~3 acid_a rot 2\n  @4 hoov rev\n}");
        let class_of = |w: &str| c.iter().find(|(t, _)| *t == w).map(|(_, k)| *k);
        assert_eq!((class_of("3"), class_of("4"), class_of("rot"), class_of("rev"), class_of("acid_a")), (Some(Class::Number), Some(Class::Number), Some(Class::Action), Some(Class::Action), Some(Class::Word)));
        // a template reference: `arrange from` is syntax, `template(` a call, its name and phrase words; as a name `template` is a name
        let c = classes("arrange from template(deep-house, @tempest_such_stuff)");
        let class_of = |w: &str| c.iter().find(|(t, _)| *t == w).map(|(_, k)| *k);
        assert_eq!((class_of("from"), class_of("template"), class_of("deep-house"), class_of("@tempest_such_stuff")), (Some(Class::Keyword), Some(Class::Generator), Some(Class::Word), Some(Class::Word)));
        let c = classes("let template = x ~ x ~");
        assert_eq!(c[1], ("template", Class::Word));
        let c = classes("x ~ c3 q");
        assert_eq!((c[0].1, c[1].1, c[2].1, c[3].1), (Class::Step, Class::Rest, Class::Step, Class::Word));
    }
}
