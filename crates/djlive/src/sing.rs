//! F9 `sing "lyrics" on melody voice=alto` (docs/beatscript/ext-sing.md): typed lyrics sung on the notes of one of the
//! program's own parts, through the `vocal` noun (docs/VOICE.md).
//!
//! ```text
//! deck B: melody [c4 e4 g4 e4] | harmony chord(am)
//! deck B: sing "robots on the road" on melody voice=alto
//! ```
//!
//! What it is, in four moves:
//!
//! 1. **The part's notes** ([`sung_notes`]): the part's onsets over the bars the line covers (the section, else
//!    the part's own cycle, or `bars=`), one note per onset (the highest, when several sound together), each
//!    lasting until the next onset as the voice's own fit does (a sixteenth of air, at most half a bar). The line is
//!    timed to the part, never to a rhythm of its own.
//! 2. **The syllables** ([`syllables`], [`layout`]): the lyric's words counted in vowel groups. A hyphen is a
//!    syllable boundary the person wrote (`ro-bots`). The lyric wraps while there are notes left and stops when
//!    the notes run out.
//! 3. **The voice** (`pick_voice`): `voice=` names a formant preset or a neural voice; a neural voice that is not
//!    installed falls back to the formant voice, with a warning ([`FALLBACK`]) at check and a word in the log when the
//!    line fires.
//! 4. **The calls**: `vocal create lyrics=.. notes=[{start,dur,midi}] bars=.. preset=..`, then `vocal to_row
//!    play=true` (the line loops on the loop row, as `vocal hook` does). A control line like any other: it fires at
//!    its bar, keeps its span and its `via`.
//!
//! Pure: the host's voices come in through [`crate::env::Voices`].

use std::collections::BTreeMap;

use djengine::json::{JsonObject, JsonValue};

use crate::ast::*;
use crate::compile::{fnv, Call, Compiler, Control, DeckIr, Effect, PartIr, Segment};
use crate::diag::{Diagnostic, Span};
use crate::env::{suggest, Voices};
use crate::expand::FlatLine;
use crate::lex::Tok;
use crate::music::{self, Key};
use crate::parse::{self, Parser, ROLES};
use crate::print;

/// What a line says when it asked for the neural voice and the host has none (at check, and in the log when it fires).
pub const FALLBACK: &str = "the neural voice is not installed: falling back to the formant voice";

/// The most bars a sung line covers (`vocal create bars=` is 1 to 32).
pub const MAX_BARS: u32 = 32;

/// The parameters `sing` takes, in the order the canonical form prints them (sorted by name).
pub const PARAMS: [&str; 3] = ["bars", "key", "voice"];

/// The roles with no pitch of their own: a drum's note is the drum.
const DRUMS: [&str; 4] = ["kick", "snare", "hihat", "clap"];

// ---- the statement

/// `sing "words" on ROLE [bars=N] [key=K] [voice=NAME]`.
#[derive(Clone, Debug, PartialEq)]
pub struct SingCall {
    /// From `sing` to the last parameter.
    pub span: Span,
    /// The lyric as typed (quotes off, escapes read); for a phrase (`@hands_up`), the phrase's phonemes as one
    /// `/…/` escape, or a public-domain line's own lyric.
    pub lyric: String,
    pub lyric_span: Span,
    /// `@hands_up`: a phrase of the phrasebook (`djengine::vocal::phoneme::phrase_lyric`: the stock phrases, then the
    /// public-domain lines), by its id.
    pub phrase: Option<String>,
    /// The part it is sung on.
    pub role: Word,
    /// `bars=N`: how many bars the sung line covers (the first N of the part).
    pub bars: Option<(u32, Span)>,
    /// `key=am`: the key the line is labelled with (its notes are the part's, whatever the key says).
    pub key: Option<Word>,
    /// `voice=alto`: a formant preset or a neural voice.
    pub voice: Option<Word>,
}

/// The canonical text of a `sing` statement (what follows the target): parameters sorted by name.
pub fn print_call(s: &SingCall) -> String {
    let said = match &s.phrase {
        Some(id) => format!("@{id}"),
        None => quote(&s.lyric),
    };
    let mut out = format!("sing {said} on {}", s.role.text);
    if let Some((n, _)) = &s.bars {
        out.push_str(&format!(" bars={n}"));
    }
    if let Some(k) = &s.key {
        out.push_str(&format!(" key={}", music::canonical_pitch_word(&k.text)));
    }
    if let Some(v) = &s.voice {
        out.push_str(&format!(" voice={}", v.text));
    }
    out
}

/// Escape problems as diagnostics, each at its symbol when the lyric is written in the source as it reads
/// (no `\"` or `\n` in it to shift the offsets), else at the whole string.
pub fn lyric_diagnostics(src: &str, lyric_span: Span, lyric: &str, issues: &[djengine::vocal::phoneme::EscIssue]) -> Vec<Diagnostic> {
    let exact = lyric_span.end <= src.len() && src.get(lyric_span.start + 1..lyric_span.end.saturating_sub(1)) == Some(lyric);
    issues
        .iter()
        .map(|i| {
            let at = if exact { Span::new(lyric_span.start + 1 + i.start, lyric_span.start + 1 + i.end.max(i.start + 1)) } else { lyric_span };
            let mut d = Diagnostic::error(at, i.message.clone()).found(format!("`{}`", i.token));
            d = match i.did_you_mean {
                Some(m) => d.help(format!("did you mean `{m}`? the symbols are in docs/beatscript/phonemes.md")),
                None => d.help("the BeatScript phonemes: aa ae uh aw eh er ii ee uu oo ay ie oy ow oh, p b t d k g f v s z sh zh th dh h m n ng l r w y ch j, and _ (docs/beatscript/phonemes.md)"),
            };
            d
        })
        .collect()
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n"))
}

impl Parser<'_> {
    /// `sing "words" on melody [bars=N] [key=K] [voice=NAME]` (the `sing` is the next word).
    pub(crate) fn sing_call(&mut self) -> Result<SingCall, Diagnostic> {
        let start = self.expect_word("sing")?;
        let mut phrase = None;
        let (lyric, lyric_span) = match self.peek().clone() {
            Tok::Str(t) => {
                let sp = self.bump().span;
                (t, sp)
            }
            Tok::Word(w) if w.starts_with('@') => {
                let sp = self.bump().span;
                let id = &w[1..];
                // a stock phrase sings its phonemes, a public-domain line its lyric
                match djengine::vocal::phoneme::phrase_lyric(id) {
                    Some(lyric) => {
                        phrase = Some(id.to_string());
                        (lyric, sp)
                    }
                    None => {
                        let ids = djengine::vocal::phoneme::phrase_ids();
                        let mut d = Diagnostic::error(sp, format!("`{w}` is not a phrase of the phrasebook")).found(format!("`{w}`"));
                        d = match suggest(id, &ids, 3).first() {
                            Some(n) => d.help(format!("did you mean `@{n}`? (docs/beatscript/phonemes.md, \"The phrasebook\")")),
                            None => d.help(format!(
                                "phrases: {}, and lines of verse and speech from before 1929: @tempest_such_stuff, @gettysburg_four_score ... (docs/beatscript/phonemes.md)",
                                ids.iter().take(6).map(|i| format!("@{i}")).collect::<Vec<_>>().join(", ")
                            )),
                        };
                        return Err(d);
                    }
                }
            }
            _ => {
                return Err(self
                    .expected(&["the words in quotes", "a phrase (`@hands_up`)"], &format!("`sing` needs the words in quotes or a phrase, found {}", self.found()))
                    .help("sing \"robots on the road\" on melody"))
            }
        };
        if lyric.trim().is_empty() {
            return Err(Diagnostic::error(lyric_span, "there is nothing to sing: the lyric is empty").help("sing \"robots on the road\" on melody"));
        }
        if !self.word_is("on") {
            let help = match self.peek() {
                Tok::Word(w) if parse::nearest(w, ["on"]).is_some() => "did you mean `on`? sing \"robots on the road\" on melody",
                _ => "sing \"robots on the road\" on melody",
            };
            return Err(self.expected(&["`on`"], &format!("`sing` says which part to sing on: expected `on`, found {}", self.found())).help(help));
        }
        self.bump();
        let role = match self.peek().clone() {
            Tok::Word(_) => self.word("a part to sing on")?,
            _ => return Err(self.expected(&ROLES.map(|r| r), &format!("expected the part to sing on, found {}", self.found())).help("sing \"robots on the road\" on melody")),
        };
        if !ROLES.contains(&role.text.as_str()) {
            let mut d = Diagnostic::error(role.span, format!("`{}` is not a part to sing on", role.text)).expected(&ROLES.map(|r| r)).found(format!("`{}`", role.text));
            if let Some(n) = parse::nearest(&role.text, ROLES) {
                d = d.help(format!("did you mean `{n}`?"));
            }
            return Err(d);
        }
        let (mut bars, mut key, mut voice): (Option<(u32, Span)>, Option<Word>, Option<Word>) = (None, None, None);
        while let Tok::Word(w) = self.peek().clone() {
            if self.peek_n(1) != &Tok::Eq {
                if PARAMS.contains(&w.as_str()) {
                    let after = self.peek_n(1).describe();
                    return Err(self
                        .expected(&["`=`"], &format!("`{w}` takes a value: expected `=` after it, found {after}"))
                        .help(format!("{w}={}", match w.as_str() { "bars" => "4", "key" => "am", _ => "alto" })));
                }
                break;
            }
            let name = self.word("a parameter")?;
            self.bump();
            if !PARAMS.contains(&name.text.as_str()) {
                let mut d = Diagnostic::error(name.span, format!("`{}` is not a parameter of `sing`", name.text)).expected(&PARAMS.map(|p| p)).found(format!("`{}`", name.text));
                if let Some(n) = parse::nearest(&name.text, PARAMS) {
                    d = d.help(format!("did you mean `{n}`?"));
                }
                return Err(d);
            }
            let dup = match name.text.as_str() {
                "bars" => bars.is_some(),
                "key" => key.is_some(),
                _ => voice.is_some(),
            };
            if dup {
                return Err(Diagnostic::error(name.span, format!("`{}` is given twice", name.text)));
            }
            match name.text.as_str() {
                "bars" => {
                    let (n, sp) = self.int("a number of bars")?;
                    if !(1..=MAX_BARS).contains(&n) {
                        return Err(Diagnostic::error(sp, format!("a sung line is 1 to {MAX_BARS} bars")).found(n.to_string()).help("bars=4"));
                    }
                    bars = Some((n, sp));
                }
                "key" => key = Some(self.key_word()?),
                _ => voice = Some(self.word("a voice name")?),
            }
        }
        Ok(SingCall { span: start.to(self.prev_span()), lyric, lyric_span, phrase, role, bars, key, voice })
    }
}

// ---- the syllables

/// A word of a lyric: the stretches a hyphen cut it into, each with its syllable count; a rest (`_`) is a word of
/// one stretch that is one syllable and sounds nothing.
#[derive(Clone, Debug, PartialEq)]
pub struct LyricWord {
    pub chunks: Vec<(String, usize)>,
    pub rest: bool,
}

impl LyricWord {
    /// How many notes the word takes.
    pub fn syllables(&self) -> usize {
        self.chunks.iter().map(|c| c.1).sum()
    }

    /// The word as the voice is given it: `ro-bots`, `road`, `_`.
    pub fn text(&self) -> String {
        if self.rest {
            return "_".to_string();
        }
        self.chunks.iter().map(|c| c.0.as_str()).collect::<Vec<_>>().join("-")
    }
}

/// How many syllables a stretch of letters has, counted in vowel groups: a run of `a e i o u` is one syllable
/// (`road` 1, `robots` 2, `beautiful` 3), `y` is a vowel unless it begins the word or sits between two vowels, a
/// silent final `e` does not count (`make`, `fire`), nor an `-ed` or `-es` that is not a syllable of its own
/// (`walked`, `makes`), while a consonant and `le` is one (`table`). A stretch with no vowel is one syllable (`hmm`) and
/// so is a number. Rough by design: where the voice's own dictionary knows better (`fire` is two), the host says so
/// ([`Voices::sung_syllables`]) and its count is the one used; a hyphen is the way to say it yourself.
pub fn vowel_groups(chunk: &str) -> usize {
    let c: Vec<char> = chunk.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '\'').map(|c| c.to_ascii_lowercase()).collect();
    let n = c.len();
    if n == 0 {
        return 0;
    }
    if c.iter().all(|x| x.is_ascii_digit() || *x == '\'') {
        return 1;
    }
    let plain = |ch: char| matches!(ch, 'a' | 'e' | 'i' | 'o' | 'u');
    let is_v = |i: usize| -> bool {
        match c[i] {
            'a' | 'e' | 'i' | 'o' | 'u' => true,
            'y' => {
                let prev_v = i > 0 && plain(c[i - 1]);
                let next_v = i + 1 < n && plain(c[i + 1]);
                !((i == 0 && next_v) || (prev_v && next_v))
            }
            _ => false,
        }
    };
    let mut groups: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < n {
        if is_v(i) {
            let s = i;
            while i < n && is_v(i) {
                i += 1;
            }
            groups.push((s, i));
        } else {
            i += 1;
        }
    }
    let mut k = groups.len();
    if k == 0 {
        return 1;
    }
    if k >= 2 {
        let (gs, ge) = groups[k - 1];
        let lone_e = ge - gs == 1 && c[gs] == 'e';
        if lone_e && ge == n {
            // a final `e` is silent, except the syllabic `le` after a consonant (table, people)
            let syllabic_le = n >= 3 && c[n - 2] == 'l' && !is_v(n - 3);
            if !syllabic_le {
                k -= 1;
            }
        } else if lone_e && ge + 1 == n && gs > 0 && !is_v(gs - 1) {
            // `-ed` is a syllable after t or d (wanted), `-es` after a sibilant (boxes); otherwise it is not (walked, makes)
            let before = c[gs - 1];
            if (c[n - 1] == 'd' && !matches!(before, 't' | 'd')) || (c[n - 1] == 's' && !matches!(before, 's' | 'x' | 'z' | 'c' | 'g' | 'h')) {
                k -= 1;
            }
        }
    }
    k.max(1)
}

/// A lyric's words. A word is letters, digits and apostrophes; `-` cuts one into stretches, `_` is a rest, and anything
/// else (a space, a comma, a full stop, a line break) ends a word, as the voice's own reading of text does
/// (`djengine::vocal::lexicon::words`). `voices` gives the voice's own syllable counts.
pub fn syllables(lyric: &str, voices: Option<&dyn Voices>) -> Vec<LyricWord> {
    fn end_chunk(cur: &mut String, chunks: &mut Vec<String>) {
        let t = cur.trim_matches('\'').to_string();
        if !t.is_empty() {
            chunks.push(t);
        }
        cur.clear();
    }
    let count = |chunk: &str| -> usize {
        match voices.and_then(|v| v.sung_syllables(chunk)).filter(|k| *k > 0) {
            Some(k) => k,
            None => vowel_groups(chunk),
        }
    };
    let flush = |chunks: &mut Vec<String>, cur: &mut String, out: &mut Vec<LyricWord>| {
        end_chunk(cur, chunks);
        if !chunks.is_empty() {
            let cs: Vec<(String, usize)> = chunks.drain(..).map(|c| (c.clone(), count(&c))).collect();
            out.push(LyricWord { chunks: cs, rest: false });
        }
    };
    let mut out: Vec<LyricWord> = Vec::new();
    let mut chunks: Vec<String> = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = lyric.chars().collect();
    // the escapes (`/…/`), by char index: each is words of phonemes, a syllable a vowel (a held one two)
    let byte_of: Vec<usize> = lyric.char_indices().map(|(b, _)| b).collect();
    let escapes = djengine::vocal::phoneme::escape_ranges(lyric);
    let mut skip_to = 0usize;
    for (i, ch) in chars.iter().enumerate() {
        if i < skip_to {
            continue;
        }
        if let Some(&(bs, be)) = escapes.iter().find(|r| r.0 == byte_of[i]) {
            flush(&mut chunks, &mut cur, &mut out);
            let (words, _) = djengine::vocal::phoneme::read_escape(&lyric[bs + 1..be - 1], bs + 1);
            for w in words {
                match w {
                    djengine::vocal::phoneme::EscWord::Rest => out.push(LyricWord { chunks: vec![("_".to_string(), 1)], rest: true }),
                    djengine::vocal::phoneme::EscWord::Phones(p) => {
                        let vowels = p.iter().filter(|x| x.sym.is_vowel()).count();
                        let n = if vowels == 0 { 1 } else { vowels + p.iter().filter(|x| x.hold).count() };
                        let text = p.iter().map(|x| format!("{}{}{}", djengine::vocal::phoneme::symbol_of(x.sym), if x.hold { ":" } else { "" }, if x.accent { "!" } else { "" })).collect::<Vec<_>>().join(" ");
                        out.push(LyricWord { chunks: vec![(format!("/{text}/"), n)], rest: false });
                    }
                }
            }
            skip_to = byte_of.iter().position(|b| *b >= be).unwrap_or(chars.len());
            continue;
        }
        // an annotated escape (`jingle=/j ii ng g uh l/`): the word is the escape's label and is not sung
        // (`djengine::vocal::lexicon::words`)
        if *ch == '=' && djengine::vocal::lexicon::annotates(&cur, &escapes, byte_of[i] + 1) {
            cur.clear();
            continue;
        }
        match *ch {
            c if c.is_ascii_alphanumeric() => cur.push(c.to_ascii_lowercase()),
            '\'' | '\u{2019}' => {
                if !cur.is_empty() && chars.get(i + 1).is_some_and(|n| n.is_ascii_alphanumeric()) {
                    cur.push('\'');
                }
            }
            '-' => end_chunk(&mut cur, &mut chunks),
            '_' => {
                flush(&mut chunks, &mut cur, &mut out);
                out.push(LyricWord { chunks: vec![("_".to_string(), 1)], rest: true });
            }
            _ => flush(&mut chunks, &mut cur, &mut out),
        }
    }
    flush(&mut chunks, &mut cur, &mut out);
    out
}

/// The lyrics a program's text writes, in order, as the lexer reads its strings (escapes read): each `sing "..."` and
/// each `lyrics="..."` (of `do vocal create`). A phrase (`sing @hands_up`) is not a lyric of the text. For the lexicon
/// harvest (docs/beatscript/lexicon-harvest.md): a quick read of the tokens, with no check.
pub fn lyrics(src: &str) -> Vec<String> {
    use crate::lex::Tok;
    let toks = crate::lex::lex(src).tokens;
    let mut out = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        let s = match (&t.tok, toks.get(i + 1).map(|t| &t.tok), toks.get(i + 2).map(|t| &t.tok)) {
            (Tok::Word(w), Some(Tok::Str(s)), _) if w == "sing" => s,
            (Tok::Word(w), Some(Tok::Eq), Some(Tok::Str(s))) if w == "lyrics" => s,
            _ => continue,
        };
        out.push(s.clone());
    }
    out
}

/// How a lyric lies on a number of notes: it wraps while there are notes left and stops when they run out.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    /// The syllables in one pass of the lyric.
    pub syllables: usize,
    /// How many times the lyric starts (1 when it is not shorter than the notes).
    pub passes: usize,
    /// Syllables the notes have no room for (a lyric longer than the notes stops: the last of it is not sung).
    pub dropped: usize,
    /// The lyric as the voice is given it: whole words, repeated, up to the word that reaches the last note.
    pub text: String,
}

/// Lay `words` on `notes` notes. None when there is nothing to sing or nothing to sing it on.
pub fn layout(words: &[LyricWord], notes: usize) -> Option<Layout> {
    let per: usize = words.iter().map(LyricWord::syllables).sum();
    if per == 0 || notes == 0 {
        return None;
    }
    let passes = notes.div_ceil(per);
    let mut text = Vec::new();
    let mut have = 0usize;
    'pass: for _ in 0..passes {
        for w in words {
            if have >= notes {
                break 'pass;
            }
            text.push(w.text());
            have += w.syllables();
        }
    }
    Some(Layout { syllables: per, passes, dropped: per.saturating_sub(notes), text: text.join(" ") })
}

/// Which syllable of the lyric each of `notes` notes sings: in order, wrapping. (The voice makes the same
/// assignment from the text [`layout`] gives it; this is the rule, written once.)
pub fn assign(syllables: usize, notes: usize) -> Vec<usize> {
    if syllables == 0 {
        return Vec::new();
    }
    (0..notes).map(|i| i % syllables).collect()
}

// ---- the part's notes

/// One sung note: where it starts, how long it lasts (sixteenths from the line's start) and its pitch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sung {
    pub start: u32,
    pub len: u32,
    pub midi: u8,
}

/// The notes of `part` over `cover` bars (its cycles in turn, round), one per onset, timed as the voice's own fit times
/// a note: until the next onset with a sixteenth of air, at most half a bar, and the last one until the line's end (at
/// most three beats).
pub fn sung_notes(part: &PartIr, cover: u32) -> Vec<Sung> {
    sung_notes_over(&[(Some(part), cover)])
}

/// The notes of a section that plays in several stretches (F10: a stack of sections of unequal lengths): each stretch
/// gives its own part's first `bars` bars in turn (None: the part is not in that stretch, so those bars are silent), so
/// the line follows what the section plays, whatever the stretch's own chance or alternations make of it.
pub fn sung_notes_over(stretches: &[(Option<&PartIr>, u32)]) -> Vec<Sung> {
    let mut onsets: BTreeMap<u32, u8> = BTreeMap::new();
    let mut offset = 0u32;
    for (part, bars) in stretches {
        if let Some(part) = part {
            let n = part.cycles.len().max(1);
            for bar in 0..*bars {
                if let Some(hits) = part.cycles.get(bar as usize % n) {
                    for h in hits {
                        let at = (offset + bar) * 16 + u32::from(h.step);
                        let e = onsets.entry(at).or_insert(h.note);
                        *e = (*e).max(h.note);
                    }
                }
            }
        }
        offset += bars;
    }
    let end = offset * 16;
    let starts: Vec<(u32, u8)> = onsets.into_iter().collect();
    let mut out = Vec::with_capacity(starts.len());
    for (i, (start, midi)) in starts.iter().enumerate() {
        let len = match starts.get(i + 1) {
            Some((next, _)) => {
                let gap = next - start;
                (if gap >= 3 { gap - 1 } else { gap }).clamp(1, 8)
            }
            None => (end - start).clamp(1, 12),
        };
        out.push(Sung { start: *start, len, midi: *midi });
    }
    out
}

/// The key as the vocal noun reads it: `A minor`, `C major`. A mode takes its relative major or minor.
pub fn vocal_key(k: Key) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let minor = matches!(k.mode, 1 | 2 | 4 | 5 | 7 | 8);
    format!("{} {}", NAMES[usize::from(k.tonic)], if minor { "minor" } else { "major" })
}

fn first_words(lyric: &str) -> String {
    let t: String = lyric.split_whitespace().take(5).collect::<Vec<_>>().join(" ");
    t.trim_matches(|c: char| !c.is_alphanumeric()).to_string()
}

// ---- the voice

/// The voice a line sings in, and what to say about it.
struct Pick {
    /// The `preset` the calls carry (None: the host's own default).
    preset: Option<String>,
    /// Said when the line fires: a neural voice was asked for and is not there.
    notice: Option<String>,
}

fn quoted_list(names: &[String]) -> String {
    names.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ")
}

/// The error for a name that is no voice, with the nearest names.
fn not_a_voice(at: Span, written: &str, bare: &str, names: &[String]) -> Diagnostic {
    let d = Diagnostic::error(at, format!("`{written}` is not a voice")).expected(&names.iter().map(String::as_str).collect::<Vec<_>>()).found(format!("`{written}`"));
    let near = suggest(bare, names, 3);
    d.help(if near.is_empty() { format!("the voices: {}", quoted_list(names)) } else { format!("did you mean {}?", quoted_list(&near)) })
}

/// Why a part cannot be sung on: `always` when the reason does not depend on the part having compiled.
struct NoPart {
    why: Diagnostic,
    always: bool,
}

impl Compiler<'_> {
    /// The voice a line asks for, against the host's voices: an unknown name is an error with the nearest names, a neural
    /// voice that is not installed is a warning and the formant fallback.
    fn pick_voice(&mut self, named: Option<&Word>, at: Span) -> Option<Pick> {
        let Some(v) = self.env.voices else {
            self.diags.push(
                Diagnostic::warning(named.map_or(at, |w| w.span), "cannot check the voice: nothing here knows the voices, or whether the neural voice is installed")
                    .help(format!("if the neural voice is missing the line plays on the formant voice: {FALLBACK}")),
            );
            return Some(Pick { preset: named.map(|w| w.text.clone()), notice: None });
        };
        let names = v.names();
        if let Some(w) = named {
            let bare = w.text.strip_prefix("voice-").unwrap_or(&w.text);
            if !names.iter().any(|n| n == bare) {
                self.err(not_a_voice(w.span, &w.text, bare, &names));
                return None;
            }
        }
        let voice = named.map_or_else(|| v.default_voice(), |w| w.text.strip_prefix("voice-").unwrap_or(&w.text).to_string());
        if v.is_neural(&voice) && !v.neural_installed() {
            let fallback = v.fallback_voice();
            self.diags.push(Diagnostic::warning(named.map_or(at, |w| w.span), FALLBACK).help(format!("it plays on `{fallback}`; name a formant voice to say so plainly: voice={fallback}")));
            return Some(Pick { preset: Some(fallback), notice: Some(FALLBACK.to_string()) });
        }
        Some(Pick { preset: Some(voice), notice: None })
    }

    /// `vocal create ... preset=NAME` (and `hook`): the same voice check as a `sing` line, so a line that names a
    /// neural voice (or, on a host whose default is the neural one, none) says the same thing and plays on the formant
    /// voice when it is not installed. A host with no voices to check against leaves the line as it is. Answers what to
    /// say when the line fires.
    pub(crate) fn vocal_voice_hook(&mut self, d: &DoCall, effect: &mut Effect) -> Option<String> {
        let v = self.env.voices?;
        let Effect::Calls(calls) = effect else { return None };
        let call = calls.iter_mut().find(|c| c.noun == "vocal" && matches!(c.params.get("action").and_then(JsonValue::as_str), Some("create" | "hook")))?;
        let named = call.params.get("preset").and_then(JsonValue::as_str).map(str::to_string);
        let at = d.params.iter().find(|p| p.name.text == "preset").map_or(d.noun.span, |p| p.value.span());
        let voice = match &named {
            Some(n) => n.strip_prefix("voice-").unwrap_or(n).to_string(),
            None => v.default_voice(),
        };
        let names = v.names();
        if let Some(w) = &named {
            if !names.iter().any(|n| *n == voice) {
                self.err(not_a_voice(at, w, &voice, &names));
                return None;
            }
        }
        if !v.is_neural(&voice) {
            return None;
        }
        if !v.neural_installed() {
            let fallback = v.fallback_voice();
            self.diags.push(Diagnostic::warning(at, FALLBACK).help(format!("it plays on `{fallback}`; name a formant voice to say so plainly: preset={fallback}")));
            if let JsonValue::Object(o) = &mut call.params {
                o.set("preset", JsonValue::str(fallback));
            }
            return Some(FALLBACK.to_string());
        }
        // the neural voice is the default and it is there: a line that names none sings in it
        if named.is_none() {
            if let JsonValue::Object(o) = &mut call.params {
                o.set("preset", JsonValue::str(voice));
            }
        }
        None
    }

    // ---- the line

    /// Every `sing` line of `lines`, compiled against `decks` (the nets of the scope the lines are in): the lines of a program
    /// with no timeline, which sing over the part's own cycle.
    pub(crate) fn sing_controls(&mut self, lines: &[FlatLine], decks: &BTreeMap<Deck, DeckIr>) {
        for fl in lines {
            if let Body::Sing(s) = &fl.line.body {
                self.sing_line(fl, s, decks, None, &[]);
            }
        }
    }

    /// The `sing` lines of every section, once the stretches' nets are there. A line sings over its whole section: a section
    /// that plays in several stretches (F10: stacked with sections of other lengths) has the line in the first, once, and a
    /// copy in each later one (`FlatLine::phase`); the notes are those each stretch plays.
    pub(crate) fn sing_sections(&mut self, lines: &[crate::expand::SegmentLines], segments: &[Segment]) {
        for (i, seg) in lines.iter().enumerate() {
            for fl in &seg.lines {
                let (Body::Sing(s), None) = (&fl.line.body, fl.phase) else { continue };
                let mut later: Vec<(&BTreeMap<Deck, DeckIr>, u32)> = Vec::new();
                for k in i + 1..lines.len() {
                    let carries = lines[k].chain && lines[k].lines.iter().any(|c| c.phase.is_some() && c.span == fl.span && c.path == fl.path && c.via == fl.via && c.timing == fl.timing);
                    if !carries {
                        break;
                    }
                    later.push((&segments[k].decks, lines[k].bars));
                }
                self.sing_into = Some((i, seg.start));
                self.sing_line(fl, s, &segments[i].decks, Some(seg.bars), &later);
                self.sing_into = None;
            }
        }
    }

    /// The `sing` lines that stand outside every section of a song: each is sung on the first section that has its part.
    pub(crate) fn sing_globals(&mut self, lines: &[FlatLine], segments: &[Segment]) {
        let empty = BTreeMap::new();
        for fl in lines {
            let Body::Sing(s) = &fl.line.body else { continue };
            let at = segments.iter().position(|seg| Self::sing_part(fl, s, &seg.decks).is_ok()).or(if segments.is_empty() { None } else { Some(0) });
            let seg = at.map(|i| &segments[i]);
            self.sing_into = at.map(|i| (i, segments[i].start));
            self.sing_line(fl, s, seg.map_or(&empty, |g| &g.decks), seg.map(|g| g.bars), &[]);
            self.sing_into = None;
        }
    }

    /// Which deck's part the line sings on: the deck it is under, or the only deck that has the role.
    fn sing_part<'d>(fl: &FlatLine, s: &SingCall, decks: &'d BTreeMap<Deck, DeckIr>) -> Result<(Deck, &'d PartIr, &'d DeckIr), NoPart> {
        let role = s.role.text.as_str();
        let always = |why: Diagnostic| NoPart { why, always: true };
        let missing = |why: Diagnostic| NoPart { why, always: false };
        if DRUMS.contains(&role) {
            return Err(always(Diagnostic::error(s.role.span, format!("`{role}` is a drum: it has no pitch to sing on")).help("sing on a tonal part: bass, melody, harmony or arp")));
        }
        let deck = match &fl.line.target {
            Some((Target::Deck(d), _)) => *d,
            Some((Target::Master, sp)) => return Err(always(Diagnostic::error(*sp, "`sing` is sung on a deck's part, and the master has none").help(format!("deck B: sing \"...\" on {role}")))),
            None => {
                let with: Vec<Deck> = decks.iter().filter(|(_, ir)| ir.parts.iter().any(|p| p.role == role)).map(|(d, _)| *d).collect();
                match with.as_slice() {
                    [one] => *one,
                    [] => {
                        let mut roles: Vec<&str> = decks.values().flat_map(|ir| ir.parts.iter().map(|p| p.role.as_str())).collect();
                        roles.sort_unstable();
                        roles.dedup();
                        let mut d = Diagnostic::error(s.role.span, format!("no deck has a `{role}` part to sing on"));
                        if let Some(n) = parse::nearest(role, roles.iter().copied()) {
                            d = d.help(format!("did you mean `{n}`?"));
                        } else if roles.is_empty() {
                            d = d.help(format!("write the part first: deck B: {role} c4 e4 g4 e4"));
                        } else {
                            d = d.help(format!("the parts here: {}", roles.join(", ")));
                        }
                        return Err(missing(d));
                    }
                    _ => {
                        return Err(always(
                            Diagnostic::error(s.role.span, format!("both decks have a `{role}` part: which one is sung on?")).help(format!("deck A: sing \"...\" on {role}   or   deck B: sing \"...\" on {role}")),
                        ));
                    }
                }
            }
        };
        let Some(ir) = decks.get(&deck) else {
            return Err(missing(Diagnostic::error(s.role.span, format!("deck {} has no parts here, so there is no `{role}` to sing on", deck.name())).help(format!("write the part first: deck {}: {role} c4 e4 g4 e4", deck.name()))));
        };
        match ir.parts.iter().find(|p| p.role == role) {
            Some(p) => Ok((deck, p, ir)),
            None => {
                let have: Vec<&str> = ir.parts.iter().map(|p| p.role.as_str()).collect();
                let mut d = Diagnostic::error(s.role.span, format!("deck {} has no `{role}` part to sing on", deck.name())).found(format!("`{role}`")).expected(&have);
                if let Some(n) = parse::nearest(role, have.iter().copied()) {
                    d = d.help(format!("did you mean `{n}`? (deck {} has {})", deck.name(), have.join(", ")));
                } else {
                    d = d.help(format!("deck {} has: {}", deck.name(), have.join(", ")));
                }
                Err(missing(d))
            }
        }
    }

    /// One `sing` line. `section` is the bars of the stretch it is in (None: no timeline) and `later` the stretches after it
    /// that the same section plays in, with their nets and lengths.
    fn sing_line(&mut self, fl: &FlatLine, s: &SingCall, decks: &BTreeMap<Deck, DeckIr>, section: Option<u32>, later: &[(&BTreeMap<Deck, DeckIr>, u32)]) {
        let l = &fl.line;
        // what can be wrong without a deck: the voice, the key, the words
        let pick = self.pick_voice(s.voice.as_ref(), s.span);
        let key_text = match &s.key {
            Some(k) => match Key::parse(&k.text) {
                Some(key) => Some(vocal_key(key)),
                None => {
                    self.err(Diagnostic::error(k.span, format!("`{}` is not a key", k.text)).expected(&["a key like am, c, f#m or d:dorian"]).found(format!("`{}`", k.text)).help(format!("modes: {}", music::MODES.join(", "))));
                    None
                }
            },
            None => self.key.map(vocal_key),
        };
        // the escapes: every symbol in the set, each mistake at its own line:col
        if s.phrase.is_none() {
            let issues = djengine::vocal::phoneme::check_escapes(&s.lyric);
            if !issues.is_empty() {
                for d in lyric_diagnostics(self.src, s.lyric_span, &s.lyric, &issues) {
                    self.err(d);
                }
                return;
            }
        }
        let words = syllables(&s.lyric, self.env.voices);
        if words.iter().map(LyricWord::syllables).sum::<usize>() == 0 {
            self.err(Diagnostic::error(s.lyric_span, "there is nothing to sing: the lyric has no words").help("sing \"robots on the road\" on melody"));
            return;
        }
        if words.iter().all(|w| w.rest) {
            self.err(Diagnostic::error(s.lyric_span, "there is nothing to sing: the lyric is all rests").help("write words between the `_`: sing \"hush _ lantern\" on melody"));
            return;
        }
        if let Some((_, sp)) = l.hold {
            self.err(
                Diagnostic::error(sp, "`for N bars` needs something that can be put back, and a sung line loops on the loop row with no off of its own here")
                    .help("let it play, or stop the loop row with a line of its own: `at bar 16: loops stop_all`"),
            );
            return;
        }
        // the part
        let (deck, part, ir) = match Self::sing_part(fl, s, decks) {
            Ok(x) => x,
            Err(no) => {
                // a part that did not compile is already an error: say nothing more about it
                if no.always || !self.diags.iter().any(Diagnostic::is_error) {
                    self.err(no.why);
                }
                return;
            }
        };
        let Some(pick) = pick else { return };
        // a sung line follows one copy of the notes
        if part.twin.is_some() {
            self.diags.push(
                Diagnostic::warning(s.role.span, format!("`{}` has a right copy (`jux`): the lyric is sung on the left copy's notes only", part.role))
                    .help("the right copy plays on with its own voice"),
            );
        }
        // the bars it covers: `bars=`, else the section, else the part's own cycle
        let cycle = ir.period.max(1) as u32;
        let whole = section.map(|b| b.max(1) + later.iter().map(|l| l.1).sum::<u32>());
        let (cover, why) = match (&s.bars, whole) {
            (Some((n, _)), _) => (*n, String::new()),
            (None, Some(b)) => (b, format!("the section is {b} bars")),
            (None, None) => (cycle, format!("the part takes {cycle} bars to come round")),
        };
        let cover = if cover > MAX_BARS {
            self.diags.push(Diagnostic::warning(s.span, format!("{why}: a sung line is at most {MAX_BARS} bars, so only the first {MAX_BARS} are sung (and it loops)")).help("`bars=` says how many bars to sing over"));
            MAX_BARS
        } else {
            cover
        };
        let plural = |n: usize| if n == 1 { "" } else { "s" };
        // the notes the section plays, stretch by stretch; the last stretch to sing takes what is left (the part's own cycle goes round)
        let mut stretches: Vec<(Option<&PartIr>, u32)> = Vec::new();
        let mut left = cover;
        let mut here: (Option<&PartIr>, u32) = (Some(part), section.map_or(cover, |b| b.max(1)));
        let mut rest = later.iter();
        loop {
            let more = if left > here.1 { rest.next() } else { None };
            let take = if more.is_some() { here.1 } else { left };
            stretches.push((here.0, take));
            left -= take;
            match more {
                Some((d, b)) => here = (Self::sing_part(fl, s, d).ok().map(|x| x.1), *b),
                None => break,
            }
        }
        let notes = sung_notes_over(&stretches);
        if notes.is_empty() {
            self.err(
                Diagnostic::error(s.role.span, format!("the `{}` part of deck {} has no notes in its first {cover} bar{} to sing on", part.role, deck.name(), plural(cover as usize))).help("a rest is `~`; give the part a note"),
            );
            return;
        }
        let Some(lay) = layout(&words, notes.len()) else { return };
        // a word no lexicon knows is sounded out by the rules: say which, and how they spelled it, so it can be
        // respelled or written in phonemes
        // (a word cut with hyphens, `lan-tern`, is the author spelling it out already: its pieces are not reported)
        let cut: Vec<String> = s
            .lyric
            .split(|c: char| c.is_whitespace() || matches!(c, ',' | '.' | ';' | ':' | '!' | '?'))
            .filter(|w| w.contains('-') && !w.contains('/'))
            .flat_map(|w| w.split('-').map(|x| x.to_lowercase()))
            .collect();
        let ruled: Vec<String> = djengine::vocal::fit::ruled_words(&s.lyric).into_iter().filter(|w| !cut.contains(w)).collect();
        if !ruled.is_empty() {
            let shown: Vec<String> = ruled
                .iter()
                .take(6)
                .map(|w| format!("`{w}` /{}/", djengine::vocal::phoneme::spell(&djengine::vocal::lexicon::rules(w))))
                .collect();
            let more = if ruled.len() > 6 { format!(" and {} more", ruled.len() - 6) } else { String::new() };
            self.diags.push(
                Diagnostic::warning(s.lyric_span, format!("not in the BeatScript lexicon, sounded out by the rules: {}{more}", shown.join(", ")))
                    .help("if one sounds wrong, respell it or write its phonemes between slashes: sing \"we are /dh uh/ rave\" on melody (docs/beatscript/phonemes.md); or sing a phrasebook line, whose words come with their escapes and a meter chosen for the groove: sing @tempest_such_stuff on melody (beatscript.app/phrasebook, docs/beatscript/found-phrases.md)"),
            );
        }
        if lay.dropped > 0 {
            self.diags.push(
                Diagnostic::warning(
                    s.lyric_span,
                    format!(
                        "the lyric has {} syllables and the part {} note{} in {cover} bar{}: the last {} {} not sung",
                        lay.syllables,
                        notes.len(),
                        plural(notes.len()),
                        plural(cover as usize),
                        lay.dropped,
                        if lay.dropped == 1 { "is" } else { "are" }
                    ),
                )
                .help("`bars=` covers more of the part; a shorter lyric fits"),
            );
        }
        // the calls
        let note_list: Vec<JsonValue> = notes
            .iter()
            .map(|n| {
                JsonValue::Object(JsonObject::from_pairs([
                    ("start", JsonValue::Number(f64::from(n.start) / 4.0)),
                    ("dur", JsonValue::Number(f64::from(n.len) / 4.0)),
                    ("midi", JsonValue::int(i64::from(n.midi))),
                ]))
            })
            .collect();
        let mut create = JsonObject::new();
        create.set("action", JsonValue::str("create"));
        create.set("lyrics", JsonValue::str(lay.text.clone()));
        create.set("notes", JsonValue::Array(note_list));
        create.set("bars", JsonValue::int(i64::from(cover)));
        create.set("name", JsonValue::str(first_words(&s.lyric)));
        if let Some(p) = &pick.preset {
            create.set("preset", JsonValue::str(p.clone()));
        }
        if let Some(k) = &key_text {
            create.set("key", JsonValue::str(k.clone()));
        }
        if let Some(t) = self.tempo {
            create.set("bpm", JsonValue::Number(t));
        }
        let to_row = JsonObject::from_pairs([("action", JsonValue::str("to_row")), ("play", JsonValue::Bool(true))]);
        let calls = vec![Call { noun: "vocal".into(), params: JsonValue::Object(create) }, Call { noun: "vocal".into(), params: JsonValue::Object(to_row) }];
        // the control's identity is the line and what it is sung on: the same line over other notes is another control
        let fp = fnv(&format!("{}|{}", calls[0].params.canonical_js(), deck.name()));
        let key = format!("{}{} [notes {fp:08x}]", fl.path, print::line(l));
        // a `when held(key)` after the line is read while the program plays (F13): the line is the singing, decided when it is due
        let Some(effect) = self.performed(fl, Effect::Calls(calls)) else { return };
        // when and where it sings, for `assert never ... during sing` (constraint.rs): the line's own meaning, whatever the host
        self.sing_lines.push(SingLine::of(fl.span, fl.timing, deck, &part.role, &notes, cover, self.sing_into, whole, !matches!(effect, Effect::Calls(_))));
        // a host that sings in the nets (the browser): the line is a part of the deck's own net, through its strip and fader
        if let Some(sp) = self.sung_part(fl, &effect, &pick, &notes, &lay, cover, deck, ir, key_text.as_deref(), later.is_empty()) {
            self.sung.push(sp);
            self.controls.push(Control { key, timing: fl.timing, effect: Effect::Calls(Vec::new()), span: fl.span, hold: None, undo: Vec::new(), restore: None, via: fl.via.clone(), notice: pick.notice, window: None });
            return;
        }
        self.controls.push(Control { key, timing: fl.timing, effect, span: fl.span, hold: None, undo: Vec::new(), restore: None, via: fl.via.clone(), notice: pick.notice, window: None });
    }
}

// ---- when a line sings

/// One `sing` line as the constraints read it (`assert never B during sing`, constraint.rs): when it starts, how far it goes
/// and the notes it is sung on. It is the line's own meaning, the same whatever the host (a browser sings it in the nets, the
/// desktop on a loop row).
#[derive(Clone, Debug, PartialEq)]
pub struct SingLine {
    /// The line in the text.
    pub span: Span,
    /// The deck and role of the part it is sung on.
    pub deck: Deck,
    pub role: String,
    /// The bar it starts on; None when that cannot be told (see `why`).
    pub start: Option<u32>,
    /// The bar its section ends on: it sings no further (None: a program with no sections, so as long as the program).
    pub until: Option<u32>,
    /// The bars of one pass of the line: it starts over every `cover` bars.
    pub cover: u32,
    /// Its notes, as (first sixteenth, sixteenths it lasts) from the start of a pass.
    pub notes: Vec<(u32, u32)>,
    /// Why its start cannot be told: `it is timed every 4 bars`, `its `when` reads a key`.
    pub why: Option<String>,
}

impl SingLine {
    /// The line at `span`, timed `timing`, sung on `role` of `deck` over `notes` for `cover` bars. `into` is the section it
    /// was compiled in (its index and first bar) and `whole` that section's bars; `performed`: its effect reads an input.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn of(span: Span, timing: crate::compile::Timing, deck: Deck, role: &str, notes: &[Sung], cover: u32, into: Option<(usize, u32)>, whole: Option<u32>, performed: bool) -> SingLine {
        use crate::compile::Timing;
        let first = into.map_or(0, |(_, start)| start);
        let (start, why) = match (performed, timing) {
            (true, _) => (None, Some("its `when` reads a key or a pad, so it sings when that is held".to_string())),
            (_, Timing::Now) => (Some(first), None),
            (_, Timing::At(b)) => (Some(b), None),
            (_, Timing::Every(n)) | (_, Timing::EveryFrom(n, _)) => (None, Some(format!("it is timed `every {n} bars`"))),
            (_, Timing::NextMult(n)) => (None, Some(format!("it is timed `on next phrase {n}`"))),
            (_, Timing::AtPos(_)) | (_, Timing::EveryPos { .. }) => (None, Some("it is timed inside a bar".to_string())),
        };
        SingLine { span, deck, role: role.to_string(), start, until: into.zip(whole).map(|((_, s), w)| s + w), cover: cover.max(1), notes: notes.iter().map(|n| (n.start, n.len)).collect(), why }
    }
}

// ---- in the nets

/// A sung line as a part of its deck's net (a host that sings in the nets: the browser, `Voices::sings_in_nets`): its
/// notes, each with its syllable, on a `voice-<preset>` instrument, and the cluster consonants (the `s` and `t` of
/// "street") on a part of their own beside it. In the net the line is heard through the deck's strip, level and
/// crossfader like any part, loops with the net, and stops with it: when the program stops or ends, when an edit takes
/// the line out (the next net has no such part), and where a looped song's lap turns (the section's net starts over).
#[derive(Clone, Debug, PartialEq)]
pub struct SungPart {
    /// The section's index among the segments (None: a program with no sections, its decks).
    pub segment: Option<usize>,
    pub deck: Deck,
    pub instrument: String,
    /// How many bars the line covers (the part's cycles are its bars).
    pub cover: u32,
    pub notes: Vec<Vec<crate::compile::Hit>>,
    pub clusters: Vec<Vec<crate::compile::Hit>>,
    pub text: String,
}

/// The most bars a deck's net may come round in once a sung part is in it.
const MAX_SUNG_PERIOD: usize = 64;

fn gcd(a: usize, b: usize) -> usize {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// Put a sung part into its deck's net: the part, its clusters, and the net's period made a multiple of the line's bars.
pub fn add_sung_part(decks: &mut BTreeMap<Deck, DeckIr>, sp: SungPart) {
    let Some(ir) = decks.get_mut(&sp.deck) else { return };
    let cover = sp.cover.max(1) as usize;
    let period = ir.period.max(1);
    ir.period = period / gcd(period, cover) * cover;
    let free = |ir: &DeckIr, base: &str| -> String {
        let mut k = 1;
        loop {
            let name = if k == 1 { base.to_string() } else { format!("{base}{k}") };
            if !ir.parts.iter().any(|p| p.role == name) {
                return name;
            }
            k += 1;
        }
    };
    let make = |role: String, cycles: Vec<Vec<crate::compile::Hit>>, text: &str| PartIr {
        role,
        instrument: Some(sp.instrument.clone()),
        sound: None,
        kit: None,
        route: None,
        vel_range: None,
        spans: Vec::new(),
        via: Vec::new(),
        text: text.to_string(),
        cycle_steps: 16,
        cycles,
        chance: false,
        structural_bars: cover as u64,
        expr: crate::compile::PartExpr::default(),
        may: Default::default(),
        twin: None,
        seed: None,
    };
    let role = free(ir, "vox");
    ir.parts.push(make(role, sp.notes.clone(), &sp.text));
    if sp.clusters.iter().any(|c| !c.is_empty()) {
        let role = free(ir, "voxc");
        ir.parts.push(make(role, sp.clusters.clone(), &sp.text));
    }
}

impl Compiler<'_> {
    /// The line as a sung part, when the host sings in the nets and the line can be one: it starts with its section (or the
    /// program), has no performed value, covers one stretch, and its deck's net still comes round in at most
    /// [`MAX_SUNG_PERIOD`] bars. None: it stays a `vocal` call.
    #[allow(clippy::too_many_arguments)]
    fn sung_part(&mut self, fl: &FlatLine, effect: &Effect, pick: &Pick, notes: &[Sung], lay: &Layout, cover: u32, deck: Deck, ir: &DeckIr, key_text: Option<&str>, one_stretch: bool) -> Option<SungPart> {
        use crate::compile::{Hit, Timing};
        use djengine::fx::formant::Syllable;
        if !self.env.voices.is_some_and(|v| v.sings_in_nets()) || !matches!(effect, Effect::Calls(_)) || !one_stretch {
            return None;
        }
        let starts_with_it = match (fl.timing, self.sing_into) {
            (Timing::Now, _) => true,
            (Timing::At(b), Some((_, start))) => b == start,
            _ => false,
        };
        if !starts_with_it {
            return None;
        }
        let period = ir.period.max(1);
        if period / gcd(period, cover as usize) * cover as usize > MAX_SUNG_PERIOD {
            return None;
        }
        let key = key_text.and_then(djengine::key::Key::parse).or_else(|| djengine::key::Key::parse("C major"))?;
        let mut params = djengine::vocal::Params::new(&lay.text, key, self.tempo.unwrap_or(120.0).clamp(20.0, 400.0), cover.clamp(1, MAX_BARS));
        params.preset = pick.preset.clone().unwrap_or_else(|| djengine::vocal::DEFAULT_PRESET.to_string());
        params.places = Some(notes.iter().map(|n| djengine::vocal::fit::Place { start: f64::from(n.start) / 4.0, dur: f64::from(n.len) / 4.0, midi: n.midi }).collect());
        let (line, _) = djengine::vocal::VocalLine::create(params).ok()?;
        let steps = cover as i64 * 16;
        let mut bars: Vec<Vec<Hit>> = vec![Vec::new(); cover as usize];
        let mut clusters: Vec<Vec<Hit>> = vec![Vec::new(); cover as usize];
        let hit = |at: i64, midi: u8, dur: i64, vel: u8, syl: u32| -> (usize, Hit) {
            let at = at.rem_euclid(steps);
            ((at / 16) as usize, Hit { step: (at % 16) as u8, note: midi, dur: dur.clamp(1, 255) as u8, vel, accent: false, slide: false, marks: Marks::default(), sid: u32::MAX, syl })
        };
        for n in &line.notes {
            let at = (n.start * 4.0).round() as i64;
            let dur = ((n.dur * 4.0).round() as i64).max(1);
            let (b, h) = hit(at, n.midi, dur, if n.accent { 127 } else { 100 }, n.syl.code());
            bars[b].push(Hit { accent: n.accent, ..h });
            // the cluster consonants: the ones before the note a step ahead of it, the ones after on its last step (each sounds:
            // a consonant alone is a burst, a hiss or a hum)
            for c in &n.pre {
                let (b, h) = hit(at - 1, n.midi, 1, 90, Syllable::consonant(*c).code());
                clusters[b].push(h);
            }
            for c in &n.post {
                let (b, h) = hit(at + dur - 1, n.midi, 1, 90, Syllable::consonant(*c).code());
                clusters[b].push(h);
            }
        }
        for v in bars.iter_mut().chain(clusters.iter_mut()) {
            v.sort_by_key(|h| (h.step, h.note));
            v.dedup_by_key(|h| (h.step, h.note));
        }
        Some(SungPart { segment: self.sing_into.map(|x| x.0), deck, instrument: format!("voice-{}", line.preset), cover, notes: bars, clusters, text: print::line(&fl.line) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::StaticVoices;

    fn counts(lyric: &str) -> Vec<usize> {
        syllables(lyric, None).iter().map(LyricWord::syllables).collect()
    }

    #[test]
    fn the_lyrics_of_a_text_are_its_sing_strings_and_lyrics_params() {
        let src = "# sing \"not this\"\ntempo 120\ndeck B: melody c4 e4\ndeck B: sing \"jingle=/j ii ng g uh l/ \\\"all\\\"\" on melody\ndeck B: sing @hands_up on melody\ndo vocal create lyrics=\"hey\" notes=[]\n";
        assert_eq!(lyrics(src), vec!["jingle=/j ii ng g uh l/ \"all\"".to_string(), "hey".to_string()]);
        assert!(lyrics("tempo 120\ndeck A: kick x...").is_empty());
    }

    #[test]
    fn a_vowel_group_is_a_syllable() {
        for (w, n) in [
            ("road", 1),
            ("robots", 2),
            ("the", 1),
            ("on", 1),
            ("night", 1),
            ("melody", 3),
            ("beautiful", 3),
            ("hmm", 1),
            ("you", 1),
            ("yes", 1),
            ("my", 1),
            ("boy", 1),
            ("player", 2),
            ("don't", 1),
            ("140", 1),
        ] {
            assert_eq!(vowel_groups(w), n, "{w}");
        }
    }

    #[test]
    fn a_silent_e_and_an_ed_or_es_that_is_not_a_syllable_do_not_count() {
        for (w, n) in [("make", 1), ("made", 1), ("fire", 1), ("walked", 1), ("makes", 1), ("tunes", 1), ("wanted", 2), ("boxes", 2), ("table", 2), ("people", 2), ("be", 1), ("he", 1), ("cities", 2)] {
            assert_eq!(vowel_groups(w), n, "{w}");
        }
    }

    #[test]
    fn a_lyric_is_words_a_hyphen_cuts_and_a_rest_is_a_syllable_with_no_sound() {
        let w = syllables("Robots on the road, ro-bo-tic _ don't", None);
        let texts: Vec<String> = w.iter().map(LyricWord::text).collect();
        assert_eq!(texts, ["robots", "on", "the", "road", "ro-bo-tic", "_", "don't"]);
        assert_eq!(w.iter().map(LyricWord::syllables).collect::<Vec<_>>(), [2, 1, 1, 1, 3, 1, 1]);
        assert!(w[5].rest && !w[0].rest);
        // punctuation, line breaks and anything not a letter end a word; case does not matter
        assert_eq!(counts("One,\ntwo; THREE! four?"), [1, 1, 1, 1]);
        assert!(syllables("... , -- !", None).is_empty());
        assert!(syllables("", None).is_empty());
    }

    #[test]
    fn the_voices_own_count_wins_where_it_knows() {
        let mut v = StaticVoices::default();
        v.syllables.insert("fire".into(), 2);
        let w = syllables("fire on the road", Some(&v));
        assert_eq!(w.iter().map(LyricWord::syllables).collect::<Vec<_>>(), [2, 1, 1, 1]);
        // and the language's own count where it does not
        assert_eq!(counts("fire on the road"), [1, 1, 1, 1]);
    }

    #[test]
    fn a_short_lyric_wraps_and_a_long_one_stops() {
        let w = syllables("robots on the road", None);
        // five syllables: ro bots on the road
        assert_eq!(assign(5, 8), [0, 1, 2, 3, 4, 0, 1, 2]);
        let l = layout(&w, 8).unwrap();
        assert_eq!((l.syllables, l.passes, l.dropped), (5, 2, 0));
        // whole words, up to the one that reaches the last note: ro bots on the road | ro bots on
        assert_eq!(l.text, "robots on the road robots on");
        // exactly as long: once
        let l = layout(&w, 5).unwrap();
        assert_eq!((l.passes, l.dropped, l.text.as_str()), (1, 0, "robots on the road"));
        // longer than the notes: it stops, and says how many it left out
        let l = layout(&w, 3).unwrap();
        assert_eq!((l.passes, l.dropped), (1, 2));
        assert_eq!(l.text, "robots on");
        assert_eq!(assign(5, 3), [0, 1, 2]);
        // a lot of notes: the lyric goes round as often as it takes
        let l = layout(&w, 23).unwrap();
        assert_eq!(l.passes, 5);
        assert_eq!(l.text.split(' ').count(), 4 * 4 + 2);
        assert!(layout(&w, 0).is_none() && layout(&[], 4).is_none());
    }

    #[test]
    fn a_hyphenated_word_keeps_its_hyphens_and_a_rest_takes_a_note() {
        let w = syllables("ro-bo-tic _ road", None);
        let l = layout(&w, 8).unwrap();
        assert_eq!(l.syllables, 5);
        assert_eq!(l.text, "ro-bo-tic _ road ro-bo-tic");
    }

    #[test]
    fn the_canonical_form_sorts_its_parameters_and_spells_the_key_one_way() {
        let c = crate::compile::check("deck B: melody c4 e4 g4 e4\ndeck B: sing   \"robots  on\\\"the road\" on   melody voice=robot key=Am bars=2\n");
        let text = c.compiled.expect("compiles").text;
        assert!(text.contains("deck B: sing \"robots  on\\\"the road\" on melody bars=2 key=am voice=robot"), "{text}");
    }

    #[test]
    fn notes_are_the_parts_onsets_each_lasting_until_the_next_with_a_sixteenth_of_air() {
        let c = crate::compile::check("deck B: melody c4 ~ e4 ~ g4 ~ ~ ~\n").compiled.unwrap();
        let part = &c.decks[&Deck::B].parts[0];
        let n = sung_notes(part, 1);
        // eight terms in a bar: the notes fall on steps 0, 4, 8
        assert_eq!(n.iter().map(|x| x.start).collect::<Vec<_>>(), [0, 4, 8]);
        assert_eq!(n.iter().map(|x| x.midi).collect::<Vec<_>>(), [60, 64, 67]);
        // until the next onset less a sixteenth; the last until the end of the bar, at most three beats
        assert_eq!(n.iter().map(|x| x.len).collect::<Vec<_>>(), [3, 3, 8]);
        // two bars: the cycle goes round
        let n2 = sung_notes(part, 2);
        assert_eq!(n2.len(), 6);
        assert_eq!(n2[3].start, 16);
    }

    #[test]
    fn a_chord_sings_its_top_note() {
        let c = crate::compile::check("deck B: harmony [c3,e3,g3] ~ [a2,c3,e3] ~\n").compiled.unwrap();
        let n = sung_notes(&c.decks[&Deck::B].parts[0], 1);
        assert_eq!(n.iter().map(|x| x.midi).collect::<Vec<_>>(), [55, 52]);
    }

    #[test]
    fn the_key_is_told_to_the_voice_as_major_or_minor() {
        assert_eq!(vocal_key(Key::parse("am").unwrap()), "A minor");
        assert_eq!(vocal_key(Key::parse("c").unwrap()), "C major");
        assert_eq!(vocal_key(Key::parse("f#m").unwrap()), "F# minor");
        assert_eq!(vocal_key(Key::parse("d:dorian").unwrap()), "D minor");
        assert_eq!(vocal_key(Key::parse("g:mixolydian").unwrap()), "G major");
        for t in ["am", "c", "f#m", "bbm", "d:dorian", "e:blues"] {
            let s = vocal_key(Key::parse(t).unwrap());
            assert!(djengine::key::Key::parse(&s).is_some(), "{s}");
        }
    }
}
