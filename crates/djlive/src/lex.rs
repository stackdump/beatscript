//! The lexer: words, numbers, a few marks, newlines. A `#` that starts a token is a comment (a `#`
//! inside a word is a sharp: `f#`, `a#4`). A newline before a line that starts with `|` is not a
//! statement end, so a deck's parts can run over several lines.

use crate::diag::{Diagnostic, Span};

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    /// A lower- or upper-case word: `kick`, `deck`, `A`, `a#4`, `echo_out`, `sub-bass`, `am7`.
    Word(String),
    /// A number with its marks folded in (`2k` = 2000, `50%` = 0.5). `int` is set when it was
    /// written as a whole number with no mark.
    Num { value: f64, int: bool },
    Colon,
    Comma,
    Pipe,
    LBrack,
    RBrack,
    Lt,
    Gt,
    LParen,
    RParen,
    Star,
    Eq,
    Tilde,
    /// `!` right after a step: an accent.
    Bang,
    /// `/` between two numbers: a ratio, 1/4.
    Slash,
    /// A `%` on its own (`gen % 4`); after digits it is the percent mark of a number.
    Percent,
    /// `^` right after a step: a slide.
    Caret,
    /// `.` right after a step, with nothing word-like after it: a ghost note (`x.`, `c3.`).
    Ghost,
    /// `&` right after a step: a flam.
    Amp,
    LBrace,
    RBrace,
    /// A quoted string (`"hello world"`), quotes off, escapes read.
    Str(String),
    /// `$name`: a parameter of a `def` (or `$i` of a `repeat`), filled in when it is expanded.
    Dollar(String),
    DotDot,
    Arrow,
    Minus,
    /// `+` (arithmetic; also a stack mark inside `deg(...)`).
    Plus,
    /// `?` (a hit's chance: `x?0.3`).
    Question,
    /// `'` (an octave mark: `5'`).
    Apostrophe,
    /// `.` between two words (`A.kick`, `a.out`); a lone `.` elsewhere is still an error.
    Dot,
    /// `±` (a tolerance: `3m30s ± 5s`).
    PlusMinus,
    /// A unit mark glued to the number before it: `6db`, `120ms`, `3m30s` is `3` `m` `30` `s`.
    /// One of [`UNITS`].
    Unit(String),
    /// `@` on its own inside an `arrange { }` (or `arrange NAME: { }`) block: a one-shot mutation (mutate.rs); anywhere else a lone `@` is still a
    /// lexer error. Glued to a word and a number (`acid@2`) it is part of the word: a generation of a pattern the block
    /// mutates.
    At,
    Newline,
    Eof,
    /// `,` glued to a degree inside `deg( )` and followed by space, `)`, `]`, `>`, `*`, `!` or `^`: the degree an octave
    /// down (`1,`). Every other comma is [`Tok::Comma`].
    OctDown,
}

/// The unit marks a number may carry, glued to it (`6db`, `5ms`, `2s`, `3m`, `1h`, `440hz`). `h` is hours: a set's `until 1h`.
pub const UNITS: [&str; 6] = ["db", "ms", "hz", "s", "m", "h"];

impl Tok {
    /// How a diagnostic names it.
    pub fn describe(&self) -> String {
        match self {
            Tok::Word(w) => format!("`{w}`"),
            Tok::Num { .. } => "a number".into(),
            Tok::Colon => "`:`".into(),
            Tok::Comma => "`,`".into(),
            Tok::Pipe => "`|`".into(),
            Tok::LBrack => "`[`".into(),
            Tok::RBrack => "`]`".into(),
            Tok::Lt => "`<`".into(),
            Tok::Gt => "`>`".into(),
            Tok::LParen => "`(`".into(),
            Tok::RParen => "`)`".into(),
            Tok::Star => "`*`".into(),
            Tok::Eq => "`=`".into(),
            Tok::Tilde => "`~`".into(),
            Tok::At => "`@`".into(),
            Tok::Bang => "`!`".into(),
            Tok::Slash => "`/`".into(),
            Tok::Percent => "`%`".into(),
            Tok::Caret => "`^`".into(),
            Tok::Ghost => "`.` (a ghost note)".into(),
            Tok::Amp => "`&`".into(),
            Tok::LBrace => "`{`".into(),
            Tok::RBrace => "`}`".into(),
            Tok::Str(_) => "a string".into(),
            Tok::Dollar(n) => format!("`${n}`"),
            Tok::DotDot => "`..`".into(),
            Tok::Arrow => "`->`".into(),
            Tok::Minus => "`-`".into(),
            Tok::Plus => "`+`".into(),
            Tok::Question => "`?`".into(),
            Tok::Apostrophe => "`'`".into(),
            Tok::Dot => "`.`".into(),
            Tok::PlusMinus => "`±`".into(),
            Tok::Unit(u) => format!("the unit `{u}`"),
            Tok::Newline => "the end of the line".into(),
            Tok::Eof => "the end of the text".into(),
            Tok::OctDown => "`,` (an octave down)".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

/// Do `prev` and `t` mean something only when they touch in the text? A unit and its number (`6db`), `x?0.3`, `5'`, `A.kick`,
/// `hihat/12`, `a.out`, `e2!`, `e2^`, `x.`, `x&`, `x:3`, `c2~3`, `1,` in a `deg( )` (`OctDown`), `+-`, and the two marks of `==` `!=` `<=` `>=`. (`tests/common::glued_pair` is the
/// same list, for the tests.) A construct whose tokens must touch is added here.
pub fn touches(prev: &Tok, t: &Tok) -> bool {
    matches!(t, Tok::Unit(_) | Tok::Dot | Tok::Question | Tok::Apostrophe | Tok::Slash | Tok::PlusMinus | Tok::Bang | Tok::Caret | Tok::OctDown | Tok::Ghost | Tok::Amp)
        || matches!(prev, Tok::Dot | Tok::Question | Tok::Slash)
        // a ratchet `x:3` and an elongate `c2~3` (pattern sugar): the count glued to its mark, the mark glued to its step
        || (matches!(prev, Tok::Colon | Tok::Tilde) && matches!(t, Tok::Num { .. }))
        || (matches!(t, Tok::Colon | Tok::Tilde) && step_end(prev))
        || (matches!(t, Tok::Eq) && matches!(prev, Tok::Eq | Tok::Lt | Tok::Gt | Tok::Bang))
}

/// A token a step of a pattern can end with (the one before a glued `:3` or `~3`): the step, or one of its marks.
pub fn step_end(t: &Tok) -> bool {
    matches!(t, Tok::Word(_) | Tok::Num { .. } | Tok::RBrack | Tok::Gt | Tok::RParen | Tok::Tilde | Tok::Bang | Tok::Caret | Tok::Ghost | Tok::Amp)
}

/// `toks` standing where a piece of the text is `at` (the `$name` a macro put them in place of, the line of an `import`):
/// each token gets that span, so an error or a mark lands on the place that is in the text. Tokens that touched in the
/// source still touch (`x*8?0.7`, `hihat/12`, `e2!`, `==`): a token that must touch the one before it (`touches`) gets
/// the empty span at the end of `at`, which is where the one before it ends. The `touches` of the result is the same as
/// of the source, so it can be done again (an import of an import, a macro that calls a macro).
pub fn put_at(toks: &[Token], at: Span) -> Vec<Token> {
    let end = Span { start: at.end, end: at.end };
    toks.iter()
        .enumerate()
        .map(|(i, t)| {
            let glued = i > 0 && toks[i - 1].span.end == t.span.start && touches(&toks[i - 1].tok, &t.tok);
            Token { tok: t.tok.clone(), span: if glued { end } else { at } }
        })
        .collect()
}

/// Tokens (ending in `Eof`), the comments' spans, and what could not be read.
pub struct Lexed {
    pub tokens: Vec<Token>,
    pub comments: Vec<Span>,
    pub errors: Vec<Diagnostic>,
}

/// What the lexer keeps of an open parenthesis (or of a `[` inside a `deg(`).
#[derive(Clone, Copy, PartialEq, Debug)]
enum Paren {
    Plain,
    Deg,
    Group,
}

fn is_word_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// `°` ends a Roman numeral (`ii°`): it is part of the word it follows.
const DEGREE_SIGN: char = '\u{b0}';
/// `ø` (half-diminished) ends a Roman numeral too (`viiø`).
pub(crate) const HALF_DIM_SIGN: char = '\u{f8}';

/// The text the lexer reads: the text as given, except that a UTF-8 byte-order mark at its start (an editor's: invisible) is white
/// space and a carriage return that is not half of a CRLF is a newline (a file with old Mac line ends was one long line and a
/// comment that ate the program). The same length byte for byte, so every span still points into the text as it was given (R-19).
pub fn normalise_input(src: &str) -> std::borrow::Cow<'_, str> {
    let bom = src.starts_with('\u{feff}');
    let lone_cr = src.as_bytes().iter().enumerate().any(|(i, c)| *c == b'\r' && src.as_bytes().get(i + 1) != Some(&b'\n'));
    if !bom && !lone_cr {
        return std::borrow::Cow::Borrowed(src);
    }
    let mut out = String::with_capacity(src.len());
    let rest = if bom {
        out.push_str("   ");
        &src['\u{feff}'.len_utf8()..]
    } else {
        src
    };
    for (i, c) in rest.char_indices() {
        if c == '\r' && !rest[i + 1..].starts_with('\n') {
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    std::borrow::Cow::Owned(out)
}

pub fn lex(src: &str) -> Lexed {
    let norm = normalise_input(src);
    let src: &str = &norm;
    let b = src.as_bytes();
    let mut tokens: Vec<Token> = Vec::new();
    let mut comments = Vec::new();
    let mut errors = Vec::new();
    let mut i = 0;
    // the parentheses open on this line, each flagged when it is a `deg(`'s: a sharp degree (`#4`) is read inside
    // any of them, and the comma marks of a degree (`1,`) inside a `deg(`; the `[` groups inside a `deg(` are kept too
    // (`Group`), for there a comma is the stack comma
    let mut parens: Vec<Paren> = Vec::new();
    // inside an `arrange { }` block (the brace depth there): the only place a lone `@` is a token (mutate.rs)
    let mut arrange_depth: Option<usize> = None;
    let mut braces: usize = 0;
    while i < b.len() {
        let c = src[i..].chars().next().unwrap();
        let start = i;
        match c {
            ' ' | '\t' | '\r' => i += 1,
            '\n' => {
                i += 1;
                parens.clear();
                // a line that goes on with `|` is the same statement
                let mut j = i;
                loop {
                    let rest = &src[j..];
                    let t = rest.trim_start_matches([' ', '\t', '\r', '\n']);
                    j += rest.len() - t.len();
                    if t.starts_with('#') {
                        j += t.find('\n').unwrap_or(t.len());
                        continue;
                    }
                    break;
                }
                let continues = src[j..].starts_with('|');
                let last_is_nl = matches!(tokens.last(), Some(Token { tok: Tok::Newline, .. }) | None);
                if !continues && !last_is_nl {
                    tokens.push(Token { tok: Tok::Newline, span: Span::new(start, start + 1) });
                }
            }
            '#' => {
                // a sharp degree inside parentheses (`deg(1 #4)`), a sharp numeral after a step (`I #iv V`): a word
                // that begins with the `#`; anything else is a comment
                if let Some(len) = crate::scale::sharp_word_len(src, i, !parens.is_empty(), !parens.is_empty() || sharp_numeral_here(&tokens)) {
                    tokens.push(Token { tok: Tok::Word(src[i..i + len].to_string()), span: Span::new(i, i + len) });
                    i += len;
                    continue;
                }
                let end = src[i..].find('\n').map_or(src.len(), |n| i + n);
                // inside a `deg( )` a `#` that is no sharp degree is a mistake, not a comment that eats the rest of the line
                if matches!(parens.last(), Some(Paren::Deg | Paren::Group)) {
                    let word = src[i + 1..].chars().take_while(|c| c.is_ascii_alphanumeric()).count();
                    errors.push(Diagnostic::error(Span::new(i, i + 1 + word), "a `#` inside `deg( )` is a sharp degree (`#4`), and a comment cannot start inside the parentheses").help("a sharp is a `#` glued to its degree: `deg(1 #4 5)`; a flat is `b3`; write a comment after the `)`"));
                }
                comments.push(Span::new(i, end));
                i = end;
            }
            ':' => i = push(&mut tokens, Tok::Colon, i, 1),
            ',' => {
                // `1,` inside `deg( )`: glued to its degree and followed by space, `)`, `]`, `>`, `*`, `!` or `^` is an
                // octave down; any other comma is the comma it always was
                let top = parens.last().copied();
                let after_degree = matches!(top, Some(Paren::Deg | Paren::Group)) && matches!(tokens.last(), Some(t) if t.span.end == i && crate::scale::is_degree_tok(&t.tok));
                let mut j = i;
                while after_degree && b.get(j) == Some(&b',') {
                    j += 1;
                }
                // inside a `[ ]` group a comma is the stack comma, whatever follows it (`[1, 3, 5]`): it is a mark only when
                // nothing a stack could take comes after it (`[3 1,]`)
                let mut k = j;
                if top == Some(Paren::Group) {
                    while matches!(b.get(k), Some(b' ' | b'\t' | b'\r')) {
                        k += 1;
                    }
                }
                // (a stack comma has a degree after it, so a `*`, `!`, `^` or `>` after the comma makes it a mark too)
                let mark_follows = |at: usize| at >= b.len() || matches!(b[at], b'\n' | b')' | b']' | b'>' | b'*' | b'!' | b'^') || (top == Some(Paren::Group) && b[at] == b',') || (top == Some(Paren::Deg) && matches!(b[at], b' ' | b'\t' | b'\r'));
                if after_degree && mark_follows(k) {
                    for k in i..j {
                        tokens.push(Token { tok: Tok::OctDown, span: Span::new(k, k + 1) });
                    }
                    i = j;
                } else {
                    i = push(&mut tokens, Tok::Comma, i, 1);
                }
            }
            '|' => i = push(&mut tokens, Tok::Pipe, i, 1),
            '[' => {
                if matches!(parens.last(), Some(Paren::Deg | Paren::Group)) {
                    parens.push(Paren::Group);
                }
                i = push(&mut tokens, Tok::LBrack, i, 1);
            }
            ']' => {
                if parens.last() == Some(&Paren::Group) {
                    parens.pop();
                }
                i = push(&mut tokens, Tok::RBrack, i, 1);
            }
            '<' => i = push(&mut tokens, Tok::Lt, i, 1),
            '>' => i = push(&mut tokens, Tok::Gt, i, 1),
            '(' => {
                parens.push(if matches!(tokens.last(), Some(Token { tok: Tok::Word(w), .. }) if w == "deg") { Paren::Deg } else { Paren::Plain });
                i = push(&mut tokens, Tok::LParen, i, 1);
            }
            ')' => {
                // (a `[` that was never closed ends with its parenthesis)
                while parens.last() == Some(&Paren::Group) {
                    parens.pop();
                }
                parens.pop();
                i = push(&mut tokens, Tok::RParen, i, 1);
            }
            '*' => i = push(&mut tokens, Tok::Star, i, 1),
            '=' => i = push(&mut tokens, Tok::Eq, i, 1),
            '~' => i = push(&mut tokens, Tok::Tilde, i, 1),
            '@' if arrange_depth.is_some() => i = push(&mut tokens, Tok::At, i, 1),
            '!' => i = push(&mut tokens, Tok::Bang, i, 1),
            '/' => i = push(&mut tokens, Tok::Slash, i, 1),
            '%' => i = push(&mut tokens, Tok::Percent, i, 1),
            '^' => i = push(&mut tokens, Tok::Caret, i, 1),
            '&' => i = push(&mut tokens, Tok::Amp, i, 1),
            '{' => {
                braces += 1;
                // `arrange {`, or `arrange NAME: {` (a named arrangement with mutations)
                let n = tokens.len();
                let named = n >= 3 && matches!(&tokens[n - 3].tok, Tok::Word(w) if w == "arrange") && matches!(tokens[n - 2].tok, Tok::Word(_)) && tokens[n - 1].tok == Tok::Colon;
                if arrange_depth.is_none() && (named || matches!(tokens.last(), Some(Token { tok: Tok::Word(w), .. }) if w == "arrange")) {
                    arrange_depth = Some(braces);
                }
                i = push(&mut tokens, Tok::LBrace, i, 1);
            }
            '}' => {
                if arrange_depth == Some(braces) {
                    arrange_depth = None;
                }
                braces = braces.saturating_sub(1);
                i = push(&mut tokens, Tok::RBrace, i, 1);
            }
            '"' => {
                let mut j = i + 1;
                let mut out = String::new();
                let mut closed = false;
                while j < src.len() {
                    let ch = src[j..].chars().next().unwrap();
                    match ch {
                        '"' => {
                            closed = true;
                            j += 1;
                            break;
                        }
                        '\\' => {
                            let n = src[j + 1..].chars().next();
                            match n {
                                Some('n') => out.push('\n'),
                                Some('"') => out.push('"'),
                                Some('\\') => out.push('\\'),
                                Some(o) => {
                                    out.push('\\');
                                    out.push(o);
                                }
                                None => {}
                            }
                            j += 1 + n.map_or(0, char::len_utf8);
                        }
                        '\n' => break,
                        o => {
                            out.push(o);
                            j += o.len_utf8();
                        }
                    }
                }
                if closed {
                    tokens.push(Token { tok: Tok::Str(out), span: Span::new(i, j) });
                } else {
                    errors.push(Diagnostic::error(Span::new(i, j), "this string is never closed").help("a string is \"between quotes\" on one line"));
                }
                i = j;
            }
            '@' if i + 1 < b.len() && ((b[i + 1] as char).is_ascii_lowercase() || b[i + 1] == b'_') => {
                // `@hands_up`: a phrase of the phrasebook (`sing @hands_up on melody`), read as one word
                let mut j = i + 1;
                while j < b.len() && ((b[j] as char).is_ascii_lowercase() || (b[j] as char).is_ascii_digit() || b[j] == b'_') {
                    j += 1;
                }
                tokens.push(Token { tok: Tok::Word(src[i..j].to_string()), span: Span::new(i, j) });
                i = j;
            }
            '$' => {
                let mut j = i + 1;
                while j < b.len() && is_word_char(b[j] as char) {
                    j += 1;
                }
                if j == i + 1 {
                    errors.push(Diagnostic::error(Span::new(i, i + 1), "a `$` needs a parameter name after it").help("$deck, $bars"));
                    i += 1;
                } else {
                    tokens.push(Token { tok: Tok::Dollar(src[i + 1..j].to_string()), span: Span::new(i, j) });
                    i = j;
                }
            }
            '-' => {
                if src[i..].starts_with("->") {
                    i = push(&mut tokens, Tok::Arrow, i, 2);
                } else {
                    i = push(&mut tokens, Tok::Minus, i, 1);
                }
            }
            // `+-` is the ASCII spelling of `±` where a tolerance stands (after a unit: `3m30s +- 5s`); anywhere else it is a plus and a minus
            '+' if b.get(i + 1) == Some(&b'-') && matches!(tokens.last(), Some(Token { tok: Tok::Unit(_), .. })) => i = push(&mut tokens, Tok::PlusMinus, i, 2),
            '+' => i = push(&mut tokens, Tok::Plus, i, 1),
            '?' => i = push(&mut tokens, Tok::Question, i, 1),
            '\'' => i = push(&mut tokens, Tok::Apostrophe, i, 1),
            '\u{b1}' => i = push(&mut tokens, Tok::PlusMinus, i, 2),
            '.' => {
                if src[i..].starts_with("..") {
                    i = push(&mut tokens, Tok::DotDot, i, 2);
                } else if matches!(tokens.last(), Some(Token { tok: Tok::Word(_) | Tok::Dollar(_) | Tok::RParen, span }) if span.end == i) && b.get(i + 1).is_some_and(|c| (*c as char).is_ascii_alphabetic() || *c == b'_') {
                    i = push(&mut tokens, Tok::Dot, i, 1);
                } else if matches!(tokens.last(), Some(Token { tok: Tok::Word(_) | Tok::Num { .. } | Tok::Dollar(_) | Tok::RBrack | Tok::Gt | Tok::RParen | Tok::Bang | Tok::Caret | Tok::Amp, span }) if span.end == i)
                    && !b.get(i + 1).is_some_and(|c| (*c as char).is_ascii_alphanumeric() || matches!(*c, b'_' | b'.'))
                {
                    // `x.` `c3.` `[x x].`: a ghost note, glued to its step
                    i = push(&mut tokens, Tok::Ghost, i, 1);
                } else {
                    errors.push(Diagnostic::error(Span::new(i, i + 1), "a lone `.` is not part of the language").help("a range is `70..100`, a rest is `~`"));
                    i += 1;
                }
            }
            c if c.is_ascii_digit() => {
                let mut j = i;
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
                let mut frac = false;
                if j + 1 < b.len() && b[j] == b'.' && b[j + 1].is_ascii_digit() {
                    frac = true;
                    j += 1;
                    while j < b.len() && b[j].is_ascii_digit() {
                        j += 1;
                    }
                }
                let mut value: f64 = src[i..j].parse().unwrap_or(0.0);
                let mut marked = false;
                if j < b.len() && b[j] == b'k' && !src[j + 1..].starts_with(is_word_char) {
                    value *= 1000.0;
                    marked = true;
                    j += 1;
                } else if j < b.len() && b[j] == b'%' {
                    value /= 100.0;
                    marked = true;
                    j += 1;
                    // `7%2` is not 7 modulo 2: a `%` right after a number is the percent mark
                    if b.get(j).is_some_and(u8::is_ascii_digit) {
                        errors.push(
                            Diagnostic::error(Span::new(i, j), format!("`{}` is the percent mark: a number then `%` is that many hundredths (`50%` is 0.5)", &src[i..j]))
                                .help(format!("modulo is written with spaces: `{} % {}`", &src[i..j - 1], src[j..].chars().take_while(char::is_ascii_digit).collect::<String>())),
                        );
                    }
                }
                // a unit mark glued to the number: `6db`, `120ms`, `2s`, `3m30s`
                let unit = if marked {
                    None
                } else {
                    UNITS.iter().find(|u| {
                        src[j..].starts_with(**u) && {
                            let after = &src[j + u.len()..];
                            // `3m30s`, `1h30m`: minutes may run straight into seconds, hours into minutes
                            !after.starts_with(is_word_char) || ((**u == "m" || **u == "h") && after.starts_with(|c: char| c.is_ascii_digit()))
                        }
                    })
                };
                if let Some(u) = unit {
                    tokens.push(Token { tok: Tok::Num { value, int: !frac }, span: Span::new(i, j) });
                    tokens.push(Token { tok: Tok::Unit((*u).to_string()), span: Span::new(j, j + u.len()) });
                    i = j + u.len();
                    continue;
                }
                // a digit run that runs into letters is no number (`2602abc`)
                if j < b.len() && (b[j] as char).is_ascii_alphabetic() {
                    let mut e = j;
                    while e < b.len() && is_word_char(b[e] as char) {
                        e += 1;
                    }
                    // a unit spelled wrong (`6dB`, `120MS`, `6dbs`) names the one it was meant to be
                    let suffix = &src[j..e];
                    let lower = suffix.to_ascii_lowercase();
                    let near = UNITS.iter().find(|u| lower == **u || (u.len() >= 2 && crate::parse::edit_distance(&lower, u) <= 1));
                    let help = match near {
                        _ if lower == "khz" => format!("a cutoff is in Hz: `{}k` is {}000 (or `{}000hz`)", &src[i..j], &src[i..j], &src[i..j]),
                        Some(u) => format!("units are lower case and glued to the number: `{}{u}`", &src[i..j]),
                        None => "numbers take `k` (2k = 2000) or `%` (50% = 0.5), and a unit (`6db`, `120ms`, `2s`), and nothing else".to_string(),
                    };
                    errors.push(Diagnostic::error(Span::new(i, e), format!("`{}` is not a number or a word", &src[i..e])).help(help));
                    i = e;
                    continue;
                }
                tokens.push(Token { tok: Tok::Num { value, int: !frac && !marked }, span: Span::new(i, j) });
                i = j;
            }
            c if is_word_start(c) => {
                let mut j = i;
                while j < b.len() {
                    let ch = b[j] as char;
                    if is_word_char(ch) {
                        j += 1;
                    } else if src[j..].starts_with(DEGREE_SIGN) && j > i {
                        j += DEGREE_SIGN.len_utf8();
                    } else if src[j..].starts_with(HALF_DIM_SIGN) && j > i {
                        j += HALF_DIM_SIGN.len_utf8();
                    } else if ch == '@' && j > i && b.get(j + 1).is_some_and(|c| c.is_ascii_digit()) {
                        // `acid@2`: a generation of a pattern an `arrange { }` block mutates (mutate.rs)
                        j += 1;
                        while j < b.len() && (b[j] as char).is_ascii_digit() {
                            j += 1;
                        }
                        break;
                    } else if ch == '#' && j > i && (b[j - 1] as char).is_ascii_alphabetic() {
                        // a sharp: f#, a#4 (a comment's `#` has a space before it)
                        j += 1;
                    } else if ch == '-' && j + 1 < b.len() && (b[j + 1] as char).is_ascii_alphanumeric() && j > i && (b[j - 1] as char).is_ascii_alphanumeric() {
                        j += 1;
                    } else {
                        break;
                    }
                }
                tokens.push(Token { tok: Tok::Word(src[i..j].to_string()), span: Span::new(i, j) });
                i = j;
            }
            other => {
                let n = other.len_utf8();
                let help = match other {
                    '@' | ';' | '\\' => "that mark is not in the language (yet): see docs/BEATSCRIPT.md".to_string(),
                    '\u{201c}' | '\u{201d}' | '\u{201e}' | '\u{ab}' | '\u{bb}' => "that is a typographic quote: a string is written between straight quotes, \"like this\" (a word processor or a chat turns them curly)".to_string(),
                    '\u{2018}' | '\u{2019}' => "that is a typographic apostrophe: the octave mark is the straight `'` (`5'`)".to_string(),
                    '\u{a0}' | '\u{2000}'..='\u{200b}' | '\u{202f}' | '\u{3000}' | '\u{feff}' => "that is a space that is not a plain one (a non-breaking space, often from pasting): write an ordinary space".to_string(),
                    '\u{2192}' | '\u{27f6}' | '\u{279c}' | '\u{2794}' | '\u{21d2}' => "that is an arrow character: a ramp is written `->`".to_string(),
                    '\u{2013}' | '\u{2014}' | '\u{2212}' => "that is a dash that looks like a minus: write the plain `-` (or `->` for a ramp)".to_string(),
                    '\u{2026}' => "that is an ellipsis character: a range is `70..100`".to_string(),
                    '\u{d7}' => "that is a multiplication sign: write `*`".to_string(),
                    '\u{f7}' => "that is a division sign: write `/`".to_string(),
                    '\u{2264}' | '\u{2265}' | '\u{2260}' => "that is a comparison character: write `<=`, `>=` or `!=`".to_string(),
                    _ => "the language is lower-case words, numbers, strings and `: , | [ ] < > ( ) { } * = ~ -> .. $ + - / % ! ^ ? ' . # \u{b1}`; `\u{b0}` and `\u{f8}` only at the end of a Roman numeral (`ii\u{b0}`)".to_string(),
                };
                errors.push(Diagnostic::error(Span::new(i, i + n), format!("unexpected character `{other}`")).help(help));
                i += n;
            }
        }
    }
    while matches!(tokens.last(), Some(Token { tok: Tok::Newline, .. })) {
        tokens.pop();
    }
    // the end of the text is where the last token ends, not past the blank lines and comments after it: "found the end of the
    // text" points at the line that stopped short
    let eof = tokens.last().map_or(src.len(), |t| t.span.end);
    tokens.push(Token { tok: Tok::Eof, span: Span::new(eof, eof) });
    Lexed { tokens, comments, errors }
}

/// May a `#` here start a sharp numeral (`I #iv V`) rather than a comment? Only in a pattern of a pitched part (the nearest
/// role word before it on the line is `bass`, `melody`, `harmony` or `arp`; after `kick`, `snare`, `hihat`, `clap` and on a
/// line that is no part, `#v` is a comment as it was before numerals), or in a `let` or `emit` of numerals (after a numeral, a
/// bracket or the `=`). A trailing comment glued to its hash (`harmony I V #v`) is the one thing it cannot tell apart.
fn sharp_numeral_here(tokens: &[Token]) -> bool {
    let line_start = tokens.iter().rposition(|t| t.tok == Tok::Newline).map_or(0, |k| k + 1);
    let line = &tokens[line_start..];
    for t in line.iter().rev() {
        if let Tok::Word(w) = &t.tok {
            if matches!(w.as_str(), "bass" | "melody" | "harmony" | "arp") {
                return true;
            }
            if crate::parse::ROLES.contains(&w.as_str()) {
                return false;
            }
        }
    }
    match (line.first().map(|t| &t.tok), line.last().map(|t| &t.tok)) {
        (Some(Tok::Word(w)), Some(last)) if w == "let" || w == "emit" => match last {
            // (`emit #iv`, `emit chords #iv`: the port's name is a word too)
            Tok::Word(l) => crate::scale::is_roman(l.trim_start_matches('#')) || l.starts_with('#') || (w == "emit" && (l == "emit" || ["chords", "key", "bass", "melody", "drums", "rhythm", "bars", "tempo", "pattern"].contains(&l.as_str()))),
            Tok::LBrack | Tok::RBrack | Tok::Lt | Tok::Gt | Tok::Eq => true,
            _ => false,
        },
        _ => false,
    }
}

fn push(tokens: &mut Vec<Token>, tok: Tok, at: usize, n: usize) -> usize {
    tokens.push(Token { tok, span: Span::new(at, at + n) });
    at + n
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(src: &str) -> Vec<Tok> {
        let l = lex(src);
        assert!(l.errors.is_empty(), "{:?}", l.errors);
        l.tokens.into_iter().map(|t| t.tok).filter(|t| *t != Tok::Eof).collect()
    }

    fn num(v: f64) -> Tok {
        Tok::Num { value: v, int: v.fract() == 0.0 }
    }

    #[test]
    fn units_are_glued_to_their_number() {
        assert_eq!(toks("6db"), vec![num(6.0), Tok::Unit("db".into())]);
        assert_eq!(toks("5ms 120ms"), vec![num(5.0), Tok::Unit("ms".into()), num(120.0), Tok::Unit("ms".into())]);
        assert_eq!(toks("3m30s"), vec![num(3.0), Tok::Unit("m".into()), num(30.0), Tok::Unit("s".into())]);
        assert_eq!(toks("-3db"), vec![Tok::Minus, num(3.0), Tok::Unit("db".into())]);
        assert_eq!(toks("1.5s"), vec![Tok::Num { value: 1.5, int: false }, Tok::Unit("s".into())]);
        // a number then a word is still an error: `6 db` is a number and a word
        assert_eq!(toks("6 db"), vec![num(6.0), Tok::Word("db".into())]);
        assert!(!lex("6dbx").errors.is_empty());
        assert!(!lex("2602abc").errors.is_empty());
        // 2k and 50% are marks, not units
        assert_eq!(toks("2k"), vec![Tok::Num { value: 2000.0, int: false }]);
    }

    #[test]
    fn put_at_keeps_the_tokens_that_touch_touching_and_the_rest_apart() {
        let src = "x*8?0.7 ~ [x x]?50% hihat/12 e2! A.kick a <= b a == b";
        let l = lex(src);
        let before: Vec<Token> = l.tokens.iter().filter(|t| t.tok != Tok::Eof).cloned().collect();
        let at = Span::new(40, 44);
        let after = put_at(&before, at);
        assert_eq!(before.len(), after.len());
        // the kind of every token is kept, and every span lies in `at`
        for (b, a) in before.iter().zip(&after) {
            assert_eq!(b.tok, a.tok);
            assert!(a.span.start >= at.start && a.span.end == at.end, "{:?}", a.span);
        }
        // a pair touches in the result exactly when it touched in the source and has to touch
        for i in 1..before.len() {
            let was = before[i - 1].span.end == before[i].span.start && touches(&before[i - 1].tok, &before[i].tok);
            assert_eq!(after[i - 1].span.end == after[i].span.start, was, "{:?} {:?}", before[i - 1].tok, before[i].tok);
        }
        // doing it again changes nothing, and a different place keeps the same touching
        assert_eq!(put_at(&after, at), after);
        let again = put_at(&after, Span::new(7, 9));
        for i in 1..after.len() {
            assert_eq!(again[i - 1].span.end == again[i].span.start, after[i - 1].span.end == after[i].span.start);
        }
        assert!(touches(&Tok::Word("x".into()), &Tok::Question) && !touches(&Tok::Word("x".into()), &Tok::Pipe));
    }

    #[test]
    fn the_new_marks() {
        assert_eq!(toks("x?0.3"), vec![Tok::Word("x".into()), Tok::Question, Tok::Num { value: 0.3, int: false }]);
        assert_eq!(toks("5'"), vec![num(5.0), Tok::Apostrophe]);
        assert_eq!(toks("A.kick"), vec![Tok::Word("A".into()), Tok::Dot, Tok::Word("kick".into())]);
        assert_eq!(toks("a + b"), vec![Tok::Word("a".into()), Tok::Plus, Tok::Word("b".into())]);
        assert_eq!(toks("3m ± 5s"), vec![num(3.0), Tok::Unit("m".into()), Tok::PlusMinus, num(5.0), Tok::Unit("s".into())]);
        assert_eq!(toks("ii° V7"), vec![Tok::Word("ii\u{b0}".into()), Tok::Word("V7".into())]);
        // a lone dot is still an error; so is one with a space before the word
        assert!(!lex("1 . 2").errors.is_empty());
        assert!(!lex("a . b").errors.is_empty());
    }

    #[test]
    fn sharp_degrees_octave_marks_and_numerals_of_f5() {
        let w = |s: &str| Tok::Word(s.into());
        // `#4` is a degree inside parentheses and a comment outside them (also after a `)`)
        assert_eq!(toks("deg(1 #4)"), vec![w("deg"), Tok::LParen, num(1.0), w("#4"), Tok::RParen]);
        assert_eq!(toks("deg(1) #4"), vec![w("deg"), Tok::LParen, num(1.0), Tok::RParen]);
        assert_eq!(toks("kick x #4"), vec![w("kick"), w("x")]);
        // the comma marks of a degree, only inside `deg( )`
        assert_eq!(toks("deg(1, 3)"), vec![w("deg"), Tok::LParen, num(1.0), Tok::OctDown, num(3.0), Tok::RParen]);
        assert_eq!(toks("deg([1,3])")[2..7], [Tok::LBrack, num(1.0), Tok::Comma, num(3.0), Tok::RBrack]);
        assert_eq!(toks("e(1, 2)")[3], Tok::Comma);
        // a sharp numeral after a step, a comment at the head of a line or with a space
        assert_eq!(toks("harmony I #iv"), vec![w("harmony"), w("I"), w("#iv")]);
        assert_eq!(toks("#iv"), vec![]);
        assert_eq!(toks("harmony I # iv"), vec![w("harmony"), w("I")]);
        // the degree sign and the half-diminished sign end a numeral
        assert_eq!(toks("ii\u{b0} vii\u{f8}7"), vec![w("ii\u{b0}"), w("vii\u{f8}7")]);
    }

    #[test]
    fn a_def_parameter_can_take_a_dot_and_a_unit_spelled_wrong_names_the_right_one() {
        // `duck $b under $a.kick 6db` in a def
        assert_eq!(toks("$a.kick"), vec![Tok::Dollar("a".into()), Tok::Dot, Tok::Word("kick".into())]);
        assert!(!lex("$a . kick").errors.is_empty());
        let help = |src: &str| lex(src).errors[0].help.clone().unwrap();
        assert!(help("6dB").contains("`6db`"), "{}", help("6dB"));
        assert!(help("120MS").contains("`120ms`"));
        assert!(help("6dbs").contains("`6db`"));
        // a word that is no unit gets the general help
        assert!(help("2602abc").contains("nothing else"));
    }
}
