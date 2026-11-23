//! `template(name, phrase)`: a phrase compiled to a whole track (docs/BEATSCRIPT.md, "Phrase templates").
//!
//! A template is a BeatScript text with slots, kept in this crate (`templates/*.beatscript`). Applying one to a phrase fills the
//! slots and writes the result out as plain BeatScript before the program is lowered, exactly as `seed(...)` does
//! (seed.rs), so everything downstream (the compiler, the browser, the phone) only ever sees plain BeatScript:
//!
//! ```text
//! arrange from template(deep-house, @tempest_such_stuff)   a phrasebook phrase
//! arrange from template(deep-house, "robots on the road")   any words
//! ```
//!
//! **The phrase drives the vocal and nothing else**; the arrangement is the template's own, fixed.
//!
//! * the syllables set the vocal line's length: one syllable to an eighth note, so a line of up to 6 syllables is one bar,
//!   up to 14 two bars, up to about 30 four bars (the line ends at least a beat before its cycle does, to breathe; the
//!   triplet feel takes up to eight cycles of twelve steps);
//! * the stress sets the accents (`!`) and the beats: a stressed syllable sits on a beat, the unstressed ones after it
//!   on the eighths that follow, and a rest is left where an odd run would put the next stress off the beat;
//! * the meter sets the subdivision: **straight** is the sixteenth grid (a bar of eight eighths), **triplet** (dactyls
//!   and anapests: three syllables to the beat) is a cycle of twelve sixteenths, `/12`, six eighths, a 6/8 bar;
//! * the notes are degrees of the key: stressed syllables on chord tones (1 3 5 8), unstressed on passing degrees
//!   (2 4 6 7), along a contour (arch, fall, rise or wave) that a hash of the words picks, so two phrases are different
//!   and every phrase is well formed.
//!
//! Where the phrase comes from: a phrasebook id has its curated words, stress and feel; any other text has its stress
//! worked out ([`Profile::from_text`]: function words are unstressed, the first syllable of any other word is stressed;
//! straight feel unless the stresses fall three syllables apart).
//!
//! **Forms** (docs/beatscript/forms.md): six of the templates are classical forms (quodlibet, passacaglia, canon, rondo, fugue,
//! theme and variations) and one is a quodlibet of two sources (cross), each a section shape and a mutation plan ([`Form`])
//! built from themes of the public-domain theme library (`djengine::themes`, filled into the template as `$THEME_<ID>`
//! slots). The phrase is sung in the form's `sung` (or `voices`) section; the sections and the arrangement are the form's own,
//! and its roles take its own themes unless the reference binds them to other sources (`ground=gen@1 dubstep seed=54876`:
//! roles.rs, which rewrites the written-out text; a reference with no roles is written out exactly as before).
//!
//! Templates are versioned ([`VERSION`]): `#! templates: v1` (also what a program with no such line means), so a template
//! always means the same track and the CID of a program that names one (its text, as written) never moves.
//!
//! Pure: no I/O and no clock; the same template and phrase are the same text.

use crate::compile::fnv;
use crate::diag::{Diagnostic, Span};
use crate::env::{Env, Voices};
use crate::lex::{lex, normalise_input, Tok, Token};
use crate::parse::nearest;
use crate::seed::Expansion;
use crate::sing::{self, LyricWord};

/// The templates this BeatScript knows: `#! templates: v1` (also what a program with no such header means).
pub const VERSION: &str = "v1";

/// A template: a name, what it is, and its text with slots (`$SLOT`).
pub struct Template {
    pub name: &'static str,
    /// One line, for the lists (the docs, llms.txt, completion).
    pub summary: &'static str,
    /// The reference that shows it.
    pub example: &'static str,
    /// The example programs that use it (`examples/beatscript/NAME.beatscript`, and on beatscript.app `/examples/NAME.beatscript`).
    pub examples: &'static [&'static str],
    /// The classical form it is, for the six form templates (docs/beatscript/forms.md); None for a groove (deep-house).
    pub form: Option<Form>,
    text: &'static str,
}

/// A classical form as a template: its section shape and the mutation plan its arrangement follows (docs/beatscript/forms.md, one
/// source with this table: the page, the MCP tools and the ontology read it from here).
pub struct Form {
    /// What a person calls it (`theme and variations`).
    pub label: &'static str,
    /// The sections and the order they come in.
    pub shape: &'static str,
    /// What the arrangement does to them, pass on pass.
    pub plan: &'static str,
    /// The arrange mutation operators the plan uses: the template's `arrange` uses each of them (a test holds it).
    pub mutations: &'static [&'static str],
    /// The themes of the public-domain theme library it is built from (`djengine::themes`).
    pub themes: &'static [&'static str],
    /// A whole hand-written track in the form: its `examples/beatscript` name and its CID.
    pub reference: Option<(&'static str, &'static str)>,
}

/// Every template, in the order the docs list them: the groove, then the forms.
pub const TEMPLATES: [Template; 8] = [
    Template {
        name: "deep-house",
        summary: "deep house at 122: a night/spot cycle, the voice buried then spotlit, crossed onto the shaker; the phrase is the vocal line",
        example: "arrange from template(deep-house, @tempest_such_stuff)",
        examples: &["such-stuff-as-dreams-122", "assyrian-122", "commit-message-122"],
        form: None,
        text: include_str!("../templates/deep-house.beatscript"),
    },
    Template {
        name: "quodlibet",
        summary: "a quodlibet at 96 in D: public-domain themes take turns by section and cross-morph toward each other and away in cycles; the phrase sung over the ground",
        example: "arrange from template(quodlibet, @psalm42_deep_calleth)",
        examples: &["quodlibet-of-four"],
        form: Some(Form {
            label: "quodlibet",
            shape: "canon ode fate voices canon voices: one theme leads each section, the others under it",
            plan: "`~ a_* morph b_1 p` paired later with `~ b_* morph a_1 p`, the pair pulled away by a morph toward a third line, `@` one-shot borrowings; `~` persists and compounds pass on pass (loop 3)",
            mutations: &["morph", "xfer", "grow", "decay"],
            themes: &["pachelbel_ground", "pachelbel_violin", "ode_to_joy", "beethoven_fate"],
            reference: Some(("canon-of-fate", "z4EBG9j86dPTC9HH6EjtBJFAWZsJLiPum4np9zoJQC9PzWm9SoC")),
        }),
        text: include_str!("../templates/quodlibet.beatscript"),
    },
    Template {
        name: "passacaglia",
        summary: "a passacaglia at 100 in G minor: a ground that never changes (Holst's five-beat ostinato) under cumulative variations; the phrase sung over the ground",
        example: "arrange from template(passacaglia, @eccl3_season)",
        examples: &[],
        form: Some(Form {
            label: "passacaglia",
            shape: "ground variation sung variation sung variation sung: the ground under every section",
            plan: "cumulative variations above the ground, each `~` added on top of the last: `rot 2 | accent`, `thin`, `rev | drift`, `grow`, `up 1`, `down 1`, `decay`; the ground is never mutated",
            mutations: &["rot", "accent", "thin", "rev", "drift", "grow", "xfer", "up", "down", "decay"],
            themes: &["mars_ostinato"],
            reference: Some(("full-fathom-five", "z4EBG9j3xZiLH5Sa8EnFW5sg2WuSPRuMbBaTr3JHeQjfWWE7Ug9")),
        }),
        text: include_str!("../templates/passacaglia.beatscript"),
    },
    Template {
        name: "canon",
        summary: "a canon at 90 in D: Pachelbel's violin line chased by copies of itself, a bar and a beat behind, over his ground; the phrase sung over them",
        example: "arrange from template(canon, @mgm_pease_porridge_hot)",
        examples: &["canon-of-the-crossing"],
        form: Some(Form {
            label: "canon",
            shape: "lead chase round sung round sung: one voice, then two, then three",
            plan: "the theme chases its own delayed and rotated copies; they are made of its bars, so `~ theme_* rot 2` and `rev` move the leader and its followers together",
            mutations: &["rot", "rev", "accent", "grow", "xfer", "decay"],
            themes: &["pachelbel_ground", "pachelbel_violin"],
            reference: None,
        }),
        text: include_str!("../templates/canon.beatscript"),
    },
    Template {
        name: "rondo",
        summary: "a rondo at 116 in C: Joplin's Entertainer as the refrain, returning mutated between episodes (Bach's Prelude in C, the phrase sung)",
        example: "arrange from template(rondo, @shanty_drunken_sailor)",
        examples: &[],
        form: Some(Form {
            label: "rondo",
            shape: "refrain episode refrain sung refrain episode refrain sung refrain: A B A C A B A C A",
            plan: "the refrain mutated on each return: `~ rot 2 | accent`, `@ mirror | vel 2` for one return, `~ up 1` then `~ down 1`; the episode morphs toward the refrain",
            mutations: &["rot", "accent", "mirror", "vel", "morph", "up", "xfer", "down"],
            themes: &["the_entertainer", "bach_prelude_c"],
            reference: Some(("rag-rondo", "z4EBG9izzm3asNWWapjXtPFSZoPzgyj4cKjPELrw4jvrhdjPxrW")),
        }),
        text: include_str!("../templates/rondo.beatscript"),
    },
    Template {
        name: "fugue",
        summary: "a fugue at 104 in B minor: Grieg's Mountain King as the subject, answered a fifth up and re-entering transposed while Bach's Badinerie morphs toward it; the phrase sung over the subject",
        example: "arrange from template(fugue, @tyger_burning_bright)",
        examples: &[],
        form: Some(Form {
            label: "fugue",
            shape: "subject answer counter sung answer counter sung: the voices enter one by one",
            plan: "the subject re-enters transposed (`~ subject_* down 2`, later `~ up 2`) while the countersubject morphs toward it (`~ counter_* morph subject_1 p`)",
            mutations: &["morph", "down", "grow", "up", "xfer", "decay"],
            themes: &["mountain_king", "badinerie"],
            reference: None,
        }),
        text: include_str!("../templates/fugue.beatscript"),
    },
    Template {
        name: "theme-and-variations",
        summary: "a theme and variations at 84 in D-flat: Dvorak's New World Largo, a different transform in each section and no cross-morphs; the phrase sung in one",
        example: "arrange from template(theme-and-variations, @dickinson_chariot_death)",
        examples: &[],
        form: Some(Form {
            label: "theme and variations",
            shape: "theme variation variation variation sung variation theme",
            plan: "one source, a different transform for each section as a one-shot `@` (`rev`, `invert`, `stutter 2`, `oct -1`, `half | accent`), so the theme comes back unchanged; no cross-morphs",
            mutations: &["rev", "invert", "stutter", "oct", "half", "accent", "grow"],
            themes: &["new_world_largo"],
            reference: None,
        }),
        text: include_str!("../templates/theme-and-variations.beatscript"),
    },
    Template {
        name: "cross",
        summary: "a cross at 120 in D: two sources (two seeds, in any genres: `a=gen@1 dubstep seed=54876, b=gen@1 house seed=7`) take turns and overlap, and morph toward each other and back in cycles; the phrase sung over the crossing",
        example: "arrange from template(cross, @psalm42_deep_calleth)",
        examples: &["cross-two-seeds"],
        form: Some(Form {
            label: "cross",
            shape: "a_leads b_leads both both sung a_leads b_leads sung: one source leads, then the other, then both at once",
            plan: "a quodlibet of two: `~ a_melody_* morph b_melody_1 p` paired with `~ b_bass_* morph a_bass_1 p`, an `@ ... xfer` one-shot in the overlap, then the pull turned round (`~ b_melody_* morph a_melody_1 p`, `~ a_bass_* morph b_bass_1 p`); `~` persists and compounds pass on pass (loop 3)",
            mutations: &["morph", "xfer", "grow", "decay"],
            themes: &["ode_to_joy", "pachelbel_ground", "badinerie"],
            reference: None,
        }),
        text: include_str!("../templates/cross.beatscript"),
    },
];

/// The templates that are forms, in the docs' order.
pub fn forms() -> impl Iterator<Item = (&'static Template, &'static Form)> {
    TEMPLATES.iter().filter_map(|t| t.form.as_ref().map(|f| (t, f)))
}

impl Template {
    /// The template's text with its slots (what `expand` fills).
    pub fn text(&self) -> &'static str {
        self.text
    }
}

/// The templates' names.
pub fn names() -> Vec<String> {
    TEMPLATES.iter().map(|t| t.name.to_string()).collect()
}

/// The template called `name`.
pub fn find(name: &str) -> Option<&'static Template> {
    TEMPLATES.iter().find(|t| t.name == name)
}

// ---------------------------------------------------------------- the reference

/// The phrase argument: a phrasebook id (`@tempest_such_stuff`) or words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PhraseRef {
    Id(String),
    Text(String),
    /// A name a `use phrase @id as NAME` line gave (uses.rs): the use stage writes the `@id` in its place before this one.
    Name(String),
}

/// `template(name, phrase, role=source ...)`, as read.
#[derive(Clone, Debug, PartialEq)]
pub struct TemplateRef {
    pub name: String,
    pub name_span: Span,
    pub phrase: PhraseRef,
    pub phrase_span: Span,
    /// The roles bound after the phrase (roles.rs): `ground=gen@1 dubstep seed=54876 part=bass`.
    pub roles: Vec<crate::roles::Binding>,
    pub span: Span,
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n"))
}

impl TemplateRef {
    /// The canonical spelling: `template(deep-house, @tempest_such_stuff)`, `template(deep-house, "robots on the road")`, and
    /// the roles after the phrase in the order the form names them: `template(passacaglia, @eccl3_season, ground=ode_to_joy)`.
    pub fn text(&self) -> String {
        let roles = crate::roles::bindings_text(&self.name, &self.roles);
        match &self.phrase {
            PhraseRef::Id(id) => format!("template({}, @{id}{roles})", self.name),
            PhraseRef::Text(t) => format!("template({}, {}{roles})", self.name, quote(t)),
            PhraseRef::Name(n) => format!("template({}, {n}{roles})", self.name),
        }
    }
}

/// A template reference in a token stream: tokens `start..end`.
pub(crate) struct Run {
    pub start: usize,
    pub end: usize,
    pub span: Span,
    pub r: Result<TemplateRef, Diagnostic>,
}

fn touches(a: &Token, b: &Token) -> bool {
    a.span.end == b.span.start
}

/// Every `template(` that stands after `from` (nowhere else is a reference, so `template` stays a name anywhere else:
/// `let template = 3`, `def template(x) { ... }` and its call are what they were), read as far as it goes.
pub(crate) fn runs(toks: &[Token]) -> Vec<Run> {
    let mut out = Vec::new();
    let mut i = 1;
    while i < toks.len() {
        let is_ref = matches!(&toks[i].tok, Tok::Word(w) if w == "template")
            && matches!(&toks[i - 1].tok, Tok::Word(f) if f == "from")
            && toks.get(i + 1).is_some_and(|t| t.tok == Tok::LParen && touches(&toks[i], t));
        if !is_ref {
            i += 1;
            continue;
        }
        let (end, r) = read_ref(toks, i);
        let span = toks[i].span.to(toks[end - 1].span);
        out.push(Run {
            start: i,
            end,
            span,
            r: r.map(|mut t| {
                t.span = span;
                t
            }),
        });
        i = end;
    }
    out
}

const SHAPE: &str = "template(deep-house, @tempest_such_stuff) or template(deep-house, \"words in quotes\"); a form's roles after the phrase: template(passacaglia, @eccl3_season, ground=gen@1 dubstep seed=54876 part=bass)";

/// Read one reference from `toks[i]` (the word `template`): where it ends, and the reference or what is wrong with it.
pub(crate) fn read_ref(toks: &[Token], i: usize) -> (usize, Result<TemplateRef, Diagnostic>) {
    let mut j = i + 2;
    let at = |j: usize| toks.get(j).map(|t| &t.tok);
    let here = |j: usize| toks.get(j).or(toks.last()).map_or(Span::new(0, 0), |t| t.span);
    let fail = |j: usize, msg: String| Diagnostic::error(toks[i].span.to(here(j.saturating_sub(1).max(i))), msg).help(SHAPE);
    let close = |_: usize| {
        // the rest of the parentheses go with the error (to the one that closes `template(`), so that one mistake is one error
        let mut depth = 0usize;
        let mut j = i + 1;
        while j < toks.len() && !matches!(toks[j].tok, Tok::Newline | Tok::Eof) {
            match toks[j].tok {
                Tok::LParen => depth += 1,
                Tok::RParen => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return j + 1;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        j
    };
    let Some(Tok::Word(name)) = at(j).cloned().filter(|t| !matches!(t, Tok::Word(w) if w.starts_with('@'))) else {
        let d = fail(j + 1, "`template(` takes a template's name first: template(deep-house, @tempest_such_stuff)".into());
        return (close(j), Err(d));
    };
    let name_span = toks[j].span;
    j += 1;
    if at(j) != Some(&Tok::Comma) {
        let d = fail(j + 1, format!("`template({name}` needs a phrase after the name: template({name}, @tempest_such_stuff)"));
        return (close(j), Err(d));
    }
    j += 1;
    let (phrase, phrase_span) = match at(j).cloned() {
        Some(Tok::Word(w)) if w.starts_with('@') && w.len() > 1 => (PhraseRef::Id(w[1..].to_string()), toks[j].span),
        Some(Tok::Str(s)) => (PhraseRef::Text(s), toks[j].span),
        // a phrase a `use phrase` line named (a word that is no role's: no `=` after it)
        Some(Tok::Word(w)) if crate::roles::is_name(&w) && at(j + 1) != Some(&Tok::Eq) => (PhraseRef::Name(w), toks[j].span),
        _ => {
            let d = fail(j + 1, "the phrase of a template is a phrasebook id (@tempest_such_stuff) or words in quotes (\"robots on the road\")".into());
            return (close(j), Err(d));
        }
    };
    j += 1;
    // a form's roles, bound after the phrase (roles.rs)
    let (j, roles) = match crate::roles::Reader::new(toks).bindings(j, 1) {
        Ok(x) => x,
        Err(d) => return (close(j), Err(d)),
    };
    if at(j) != Some(&Tok::RParen) {
        // a `gen` call the parser could not read took the rest of its line with its error (gen.rs): that is the one error
        if j > 0 && matches!(&toks[j - 1].tok, Tok::Word(w) if w == "gen@0 error") {
            return (j, Ok(TemplateRef { name, name_span, phrase, phrase_span, roles, span: Span::new(0, 0) }));
        }
        let d = fail(j + 1, "expected `)` to close template(...)".into());
        return (close(j), Err(d));
    }
    (j + 1, Ok(TemplateRef { name, name_span, phrase, phrase_span, roles, span: Span::new(0, 0) }))
}

/// `src` with every template reference blanked (its text made spaces, the places of everything else kept): what the stages
/// after the template's read, so that a source bound to a role (`ground=gen@1 ...`) is the template's, not a reference of
/// its own until the template is written out (refs.rs).
pub fn masked(src: &str) -> String {
    let toks = lex(src).tokens;
    let rs = runs(&toks);
    if rs.is_empty() {
        return src.to_string();
    }
    let mut out = src.to_string().into_bytes();
    for r in rs {
        for b in &mut out[r.span.start..r.span.end.min(src.len())] {
            if *b != b'\n' {
                *b = b' ';
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| src.to_string())
}

/// Each reference made one word token (its canonical spelling) for the parser, and what is wrong with any of them.
pub(crate) fn collapse(toks: Vec<Token>) -> (Vec<Token>, Vec<Diagnostic>) {
    let rs = runs(&toks);
    if rs.is_empty() {
        return (toks, Vec::new());
    }
    let mut out = Vec::with_capacity(toks.len());
    let mut diags = Vec::new();
    let mut i = 0;
    for r in rs {
        out.extend_from_slice(&toks[i..r.start]);
        let text = match r.r {
            Ok(t) => t.text(),
            Err(d) => {
                diags.push(d);
                "template()".to_string()
            }
        };
        out.push(Token { tok: Tok::Word(text), span: r.span });
        i = r.end;
    }
    out.extend_from_slice(&toks[i..]);
    (out, diags)
}

// ---------------------------------------------------------------- the phrase

/// The subdivision a phrase's meter asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feel {
    /// The sixteenth grid: a bar of eight eighths.
    Straight,
    /// Three syllables to the beat (dactyls, anapests): a cycle of twelve sixteenths, `/12`.
    Triplet,
}

impl Feel {
    pub fn name(self) -> &'static str {
        match self {
            Feel::Straight => "straight",
            Feel::Triplet => "triplet",
        }
    }

    /// A beat, in sixteenth steps: a quarter note, or a dotted quarter (three eighths) in the 12-step cycle.
    fn beat(self) -> u32 {
        match self {
            Feel::Straight => 4,
            Feel::Triplet => 6,
        }
    }

    /// The cycle of the vocal part, in sixteenth steps: the bar, or twelve.
    fn cycle(self) -> u32 {
        match self {
            Feel::Straight => 16,
            Feel::Triplet => 12,
        }
    }

    /// The most cycles a line takes: four bars, or eight cycles of twelve steps (six bars). A straight line is no longer
    /// because the shaker the voice is crossed onto must have a note for each syllable (`xfer`), and it has five hits a bar.
    fn max_cycles(self) -> u32 {
        match self {
            Feel::Straight => 4,
            Feel::Triplet => 8,
        }
    }

    /// The section's length in bars: a whole number of the vocal cycles and of the four-bar harmony.
    fn section_bars(self) -> u32 {
        match self {
            Feel::Straight => 8,
            Feel::Triplet => 12,
        }
    }
}

/// What a phrase is to a template: the words as sung, which syllables are stressed, and the feel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    /// The lyric as the `sing` line is given it: words, and phoneme escapes where a phrase has them.
    pub lyric: String,
    /// One flag for each syllable the line sings: stressed or not. At least one is stressed.
    pub stress: Vec<bool>,
    pub feel: Feel,
    /// The meter's name, as far as the stresses show one: `iambic`, `trochaic`, `dactylic`, `anapestic`,
    /// `amphibrachic`, or `free`.
    pub meter: &'static str,
    /// The stress and feel are a phrasebook's own, not worked out.
    pub curated: bool,
}

/// Words a phrase does not stress (the closed classes of English: articles, prepositions, auxiliaries, pronouns,
/// conjunctions). Every other word is stressed on its first syllable.
const FUNCTION_WORDS: &[&str] = &[
    "a", "about", "above", "after", "against", "am", "an", "and", "are", "as", "at", "be", "been", "but", "by", "can", "could", "did", "do", "does", "for", "from", "had", "has", "have",
    "he", "her", "him", "his", "i", "i'm", "if", "in", "into", "is", "it", "it's", "its", "me", "my", "no", "nor", "not", "of", "on", "or", "our", "she", "so", "than", "that", "the",
    "their", "them", "then", "there", "these", "they", "this", "those", "through", "to", "up", "us", "was", "we", "were", "what", "when", "where", "which", "who", "will", "with", "would",
    "you", "your", "you're", "we're", "they're", "don't",
];

impl Profile {
    /// A profile written by hand: `stress` is one flag for each syllable. With none stressed the first is (a line has a
    /// beat to start from); the meter is read off the stresses.
    pub fn new(lyric: &str, stress: Vec<bool>, feel: Feel) -> Profile {
        let mut stress = stress;
        if !stress.is_empty() && !stress.iter().any(|s| *s) {
            stress[0] = true;
        }
        let meter = classify(&stress);
        Profile { lyric: lyric.to_string(), stress, feel, meter, curated: true }
    }

    /// The profile of any text: its stress and feel worked out. Err says what is wrong with the phrase.
    pub fn from_text(text: &str, voices: Option<&dyn Voices>) -> Result<Profile, String> {
        let lyric = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let words = sing::syllables(&lyric, voices);
        let stress = heuristic_stress(&words);
        if stress.is_empty() {
            return Err("there is nothing to sing in this phrase: it has no words".to_string());
        }
        let mut p = Profile::new(&lyric, stress, Feel::Straight);
        p.curated = false;
        if matches!(p.meter, "dactylic" | "anapestic" | "amphibrachic") {
            p.feel = Feel::Triplet;
        }
        Ok(p)
    }

    /// The profile of a phrasebook phrase: its words and its stress and feel as the book has them (`stress` is `x` for a
    /// stressed syllable and `~` for an unstressed one, one for each syllable; a short one is padded with unstressed).
    pub fn curated(lyric: &str, stress: &str, feel: Feel, voices: Option<&dyn Voices>) -> Result<Profile, String> {
        let n: usize = sing::syllables(lyric, voices).iter().map(LyricWord::syllables).sum();
        if n == 0 {
            return Err("there is nothing to sing in this phrase: it has no words".to_string());
        }
        let mut flags: Vec<bool> = stress.chars().filter(|c| matches!(c, 'x' | '~')).map(|c| c == 'x').collect();
        flags.resize(n, false);
        Ok(Profile::new(lyric, flags, feel))
    }

    /// How many syllables the line sings.
    pub fn syllables(&self) -> usize {
        self.stress.len()
    }

    /// The stress as written in the phrasebook: `~x~x~x~x~`.
    pub fn stress_text(&self) -> String {
        self.stress.iter().map(|s| if *s { 'x' } else { '~' }).collect()
    }
}

/// One stress flag for each syllable of the words, by the plain rule.
fn heuristic_stress(words: &[LyricWord]) -> Vec<bool> {
    let mut out = Vec::new();
    for w in words {
        let n = w.syllables();
        if w.rest {
            out.push(false);
            continue;
        }
        let text: String = w.chunks.iter().map(|c| c.0.as_str()).collect::<Vec<_>>().join("");
        if let Some(esc) = w.chunks.first().filter(|c| c.0.starts_with('/')) {
            out.extend(escape_stress(&esc.0, n));
        } else if FUNCTION_WORDS.contains(&text.to_lowercase().as_str()) {
            out.extend(std::iter::repeat_n(false, n));
        } else {
            out.extend((0..n).map(|k| k == 0));
        }
    }
    out
}

/// The stress of a word written in phonemes (`/ay! r uh/`): the syllable a `!` accents, else the first.
fn escape_stress(escape: &str, n: usize) -> Vec<bool> {
    let inner = escape.trim_matches('/');
    let mut flags: Vec<bool> = Vec::new();
    let mut accented = false;
    for tok in inner.split_whitespace() {
        let sym = tok.trim_end_matches(['!', ':']);
        if !djengine::vocal::phoneme::phoneme(sym).is_some_and(|p| matches!(p.class, djengine::vocal::phoneme::Class::Vowel | djengine::vocal::phoneme::Class::Diphthong)) {
            continue;
        }
        let accent = tok.contains('!');
        accented |= accent;
        flags.push(accent);
        if tok.contains(':') {
            // a held syllable takes one more note
            flags.push(false);
        }
    }
    flags.resize(n, false);
    if !accented {
        if let Some(f) = flags.first_mut() {
            *f = true;
        }
    }
    flags
}

/// The meter the stresses show, if they show one: all (nearly) on one position of a foot of two or three syllables, and
/// most of that position's places stressed.
pub fn classify(stress: &[bool]) -> &'static str {
    let n = stress.len();
    let total = stress.iter().filter(|s| **s).count();
    if n < 4 || total == 0 {
        return "free";
    }
    for (p, r, name) in [(2usize, 1usize, "iambic"), (2, 0, "trochaic"), (3, 2, "anapestic"), (3, 0, "dactylic"), (3, 1, "amphibrachic")] {
        let places = (0..n).filter(|i| i % p == r).count();
        let hits = (0..n).filter(|i| i % p == r && stress[*i]).count();
        if hits * 5 >= total * 4 && hits * 4 >= places * 3 {
            return name;
        }
    }
    "free"
}

// ---------------------------------------------------------------- the vocal line

/// One eighth note's place in a vocal cycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell {
    Rest,
    /// A degree of the key and whether it is accented.
    Note(u8, bool),
}

/// The vocal line a phrase makes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub feel: Feel,
    /// How many cycles (bars, or 12-step cycles) the line takes: 1, 2, 4 or (in the triplet feel) 8.
    pub cycles: u32,
    /// The cycles that have a note in them (the rest of the line is its breath): the first `used` of `cycles`.
    pub used: u32,
    /// Each used cycle's eighth notes: eight in a bar, six in a cycle of twelve.
    pub cells: Vec<Vec<Cell>>,
    /// The step (sixteenths from the line's start) each syllable sits on.
    pub steps: Vec<u32>,
}

/// An eighth note, in steps.
const SLOT: u32 = 2;

/// The degrees a stressed syllable sits on (the chord: root, third, fifth, octave) and the ones an unstressed syllable
/// passes through.
const CHORD: [i32; 4] = [1, 3, 5, 8];
const PASSING: [i32; 4] = [2, 4, 6, 7];

fn nearest_of(set: &[i32], target: i32) -> i32 {
    // a tie goes to the lower degree
    set.iter().copied().min_by_key(|d| ((d - target).abs(), *d)).unwrap_or(1)
}

/// How high the line is at syllable `i` of `n` (0 to 6 degrees above the root), along one of four contours.
fn contour(shape: u32, i: usize, n: usize) -> i32 {
    let m = n.saturating_sub(1).max(1) as i32;
    let i = i as i32;
    match shape % 4 {
        // an arch: up to the middle of the line and down again
        0 => i.min(m - i) * 2 * 6 / m,
        // a fall: from the top to the root
        1 => (m - i) * 6 / m,
        // a rise
        2 => i * 6 / m,
        // a wave: up four syllables and down four
        _ => {
            let t = i % 8;
            (if t < 4 { t } else { 8 - t }) * 3 / 2
        }
    }
}

/// Lay a profile out as a vocal line, or say why it does not fit.
pub fn layout(p: &Profile) -> Result<Line, String> {
    let n = p.syllables();
    if n == 0 {
        return Err("there is nothing to sing in this phrase: it has no words".to_string());
    }
    let beat = p.feel.beat();
    // a pickup: the unstressed syllables before the first stress start early, so that the first stress is on a beat
    let pickup = p.stress.iter().take_while(|s| !**s).count() as u32;
    let mut pos = (beat - (pickup * SLOT) % beat) % beat;
    let mut steps = Vec::with_capacity(n);
    for s in &p.stress {
        if *s {
            while pos % beat != 0 {
                pos += SLOT;
            }
        }
        steps.push(pos);
        pos += SLOT;
    }
    // the line ends at least a beat before its cycle does
    let need = pos + beat;
    let Some(cycles) = [1, 2, 4, 8].into_iter().filter(|k| *k <= p.feel.max_cycles()).find(|k| k * p.feel.cycle() >= need) else {
        let room = p.feel.max_cycles() * p.feel.cycle();
        return Err(format!(
            "this phrase takes {pos} steps to sing ({n} syllables) and a vocal line is at most {} of them with a beat to breathe: {} at the {} feel",
            room - beat,
            if p.feel == Feel::Straight { "four bars".to_string() } else { "eight cycles of twelve steps".to_string() },
            p.feel.name()
        ));
    };
    let per = (p.feel.cycle() / SLOT) as usize;
    let used = (pos).div_ceil(p.feel.cycle());
    let mut cells = vec![vec![Cell::Rest; per]; used as usize];
    let shape = fnv(&p.lyric);
    let last = n - 1;
    let mut before = 0;
    for (i, step) in steps.iter().enumerate() {
        let target = 1 + contour(shape, i, n);
        let degree = if i == last {
            nearest_of(&[1, 5, 8], target)
        } else if p.stress[i] {
            nearest_of(&CHORD, target)
        } else {
            // a passing degree, and not the one just sung: the line moves
            let d = nearest_of(&PASSING, target);
            if d == before {
                nearest_of(&PASSING.iter().copied().filter(|x| *x != d).collect::<Vec<_>>(), target)
            } else {
                d
            }
        };
        before = degree;
        let slot = (step / SLOT) as usize;
        cells[slot / per][slot % per] = Cell::Note(degree as u8, p.stress[i]);
    }
    Ok(Line { feel: p.feel, cycles, used, cells, steps })
}

fn letter(i: usize) -> char {
    (b'a' + (i % 26) as u8) as char
}

impl Line {
    /// The name of cycle `i`'s `let`: `vox_a`, `vox_b` ...
    fn let_name(i: usize) -> String {
        format!("vox_{}", letter(i))
    }

    /// The `let` lines of the line: a pattern for each cycle, each eighth a degree (`!` accented) or a rest.
    pub fn lets(&self) -> String {
        self.cells
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let terms: Vec<String> = c
                    .iter()
                    .map(|x| match x {
                        Cell::Rest => "~".to_string(),
                        Cell::Note(d, true) => format!("{d}!"),
                        Cell::Note(d, false) => d.to_string(),
                    })
                    .collect();
                format!("let {} = {}", Line::let_name(i), terms.join(" "))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// What the part plays: the one cycle, or the cycles in turn, the breath at the end of the line a `~` (a bar of rests
    /// is no `let`: the arrangement's mutations would have nothing to move there).
    pub fn part(&self) -> String {
        if self.cycles == 1 {
            return Line::let_name(0);
        }
        let mut names: Vec<String> = (0..self.used as usize).map(Line::let_name).collect();
        names.extend((self.used..self.cycles).map(|_| "~".to_string()));
        format!("<{}>", names.join(" "))
    }

    /// The part's cycle, glued to its role: nothing for the bar, `/12` for the triplet feel.
    pub fn cycle_mark(&self) -> &'static str {
        match self.feel {
            Feel::Straight => "",
            Feel::Triplet => "/12",
        }
    }

    /// The line's length as a person says it: `4 bars`, `1 1/2 bars`, `3/4 bar`.
    pub fn length(&self) -> String {
        match self.feel {
            Feel::Straight => format!("{} bar{}", self.cycles, if self.cycles == 1 { "" } else { "s" }),
            Feel::Triplet => match self.cycles {
                1 => "3/4 bar".to_string(),
                2 => "1 1/2 bars".to_string(),
                k => format!("{} bars", k * 3 / 4),
            },
        }
    }

    /// The shaker the vocal is crossed onto: eight terms in a bar, six in a cycle of twelve.
    fn shaker(&self) -> &'static str {
        match self.feel {
            Feel::Straight => "x ~ x x ~ x x ~",
            Feel::Triplet => "x ~ x x ~ x",
        }
    }
}

// ---------------------------------------------------------------- the phrase's source

/// A phrase as the phrasebook or the program gives it.
struct Source {
    profile: Profile,
    /// The words, for the comment at the head of the written-out program.
    words: String,
    /// Where it is from, when the book says (`The Tempest 4.1`).
    cite: Option<String>,
}

/// A phrasebook phrase by its id: the public-domain lines (their words as sung, their stress and feel as curated) and
/// the stock phrases (their stress worked out from the words they say).
fn phrasebook(id: &str, voices: Option<&dyn Voices>) -> Option<Result<Source, String>> {
    use djengine::vocal::phoneme;
    if let Some(p) = phoneme::pd_phrase(id) {
        let feel = if p.feel == "triplet" { Feel::Triplet } else { Feel::Straight };
        return Some(Profile::curated(&p.lyric, &p.stress, feel, voices).map(|profile| Source { profile, words: p.text.clone(), cite: Some(format!("{}, {}, {}", p.work, p.author, p.year)) }));
    }
    let p = phoneme::phrase(id)?;
    // the stock phrases have no curated stress: it is worked out from the words they say, when those are as many
    // syllables as the lyric sings (else the plain trochee)
    let lyric = p.lyric();
    let n = syllable_count(&lyric, voices);
    let from_words = Profile::from_text(&p.words, voices).ok().filter(|w| w.syllables() == n);
    let stress = match &from_words {
        Some(w) => w.stress_text(),
        None => (0..n).map(|i| if i % 2 == 0 { 'x' } else { '~' }).collect(),
    };
    Some(Profile::curated(&lyric, &stress, from_words.map_or(Feel::Straight, |w| w.feel), voices).map(|profile| Source { profile, words: p.words, cite: None }))
}

// ---------------------------------------------------------------- writing it out

/// Fill the slots (`$NAME`, upper case and underscores) of a template's text. A slot no one fills is a bug in the template.
fn fill(text: &str, slots: &[(String, String)]) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'$' && b.get(i + 1).is_some_and(u8::is_ascii_uppercase) {
            let mut j = i + 1;
            while j < b.len() && (b[j].is_ascii_uppercase() || b[j] == b'_') {
                j += 1;
            }
            let name = &text[i + 1..j];
            match slots.iter().find(|s| s.0 == name) {
                Some((_, v)) => out.push_str(v),
                None => out.push_str(&text[i..j]),
            }
            i = j;
        } else {
            let c = text[i..].chars().next().unwrap_or(' ');
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

/// A phrase on one line, for a comment.
fn one_line(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The slots a line fills: the phrase's, and `$THEME_<ID>` for every theme of the library (its `let` lines).
fn slots(t: &TemplateRef, src: &Source, line: &Line) -> Vec<(String, String)> {
    let p = &src.profile;
    let phrase = match &t.phrase {
        PhraseRef::Id(id) => format!("@{id}, \"{}\"{}", one_line(&src.words), src.cite.as_ref().map_or(String::new(), |c| format!(" ({c})"))),
        PhraseRef::Text(_) | PhraseRef::Name(_) => format!("\"{}\"", one_line(&src.words)),
    };
    let stress = p.stress_text();
    let shown: String = if stress.chars().count() > 40 { format!("{}...", stress.chars().take(40).collect::<String>()) } else { stress };
    vec![
        ("REF", t.text()),
        ("VERSION", VERSION.to_string()),
        ("PHRASE", phrase),
        (
            "VOCAL",
            format!("{} syllables, {}, {} feel, stress {shown}: a vocal line of {}, sung over sections of {} bars", p.syllables(), p.meter, p.feel.name(), line.length(), p.feel.section_bars()),
        ),
        ("SHAKER", line.shaker().to_string()),
        ("VOX_LETS", line.lets()),
        ("VOX", line.part()),
        ("CYCLE", line.cycle_mark().to_string()),
        ("BARS", p.feel.section_bars().to_string()),
        ("LYRIC", quote(&p.lyric)),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .chain(djengine::themes::themes().iter().map(|th| (format!("THEME_{}", th.id.to_ascii_uppercase()), th.lets())))
    .collect()
}

/// Does the program say this setting (`tempo`, `key`, `swing`) at the head of a line?
fn says(toks: &[Token], word: &str) -> bool {
    toks.iter().enumerate().any(|(i, t)| matches!(&t.tok, Tok::Word(w) if w == word) && (i == 0 || toks[i - 1].tok == Tok::Newline) && !matches!(toks.get(i + 1).map(|t| &t.tok), Some(Tok::Colon)))
}

/// A template written out for a program: the settings the program does not say itself (its own `tempo`, `key` and `swing`
/// win), then the lets, sections and arrange.
pub(crate) fn write_out(tpl: &Template, slots: &[(String, String)], keep: impl Fn(&str) -> bool) -> String {
    let text = fill(tpl.text, slots);
    let mut out = String::with_capacity(text.len());
    for l in text.lines() {
        let head = l.split_whitespace().next().unwrap_or("");
        if matches!(head, "tempo" | "key" | "swing") && !keep(head) {
            continue;
        }
        out.push_str(l);
        out.push('\n');
    }
    while out.ends_with('\n') {
        out.pop();
    }
    out
}

/// The `#! templates:` version a program asks for, if it says one: an error when it is not one this BeatScript has.
fn version_check(src: &str, comments: &[Span]) -> Option<Diagnostic> {
    for c in comments {
        let line = c.text(src).trim();
        let Some(rest) = line.strip_prefix("#!") else { continue };
        let Some(v) = rest.trim().strip_prefix("templates:") else { continue };
        let v = v.trim();
        if v != VERSION {
            return Some(Diagnostic::error(*c, format!("this BeatScript has templates {VERSION}, not `{v}`")).help(format!("#! templates: {VERSION} (a template always means the same track in one version)")));
        }
    }
    None
}

/// The index of the first token of the line `toks[at]` is on, when the tokens before it on that line are `arrange from`.
fn arrange_from(toks: &[Token], at: usize) -> Option<usize> {
    let line_start = toks[..at].iter().rposition(|t| t.tok == Tok::Newline).map_or(0, |i| i + 1);
    let head: Vec<&Tok> = toks[line_start..at].iter().map(|t| &t.tok).collect();
    match head.as_slice() {
        [Tok::Word(a), Tok::Word(f)] if a == "arrange" && f == "from" => Some(line_start),
        _ => None,
    }
}

/// `src` with every template reference written out as BeatScript, or None when it has none. Errors are at the references.
/// `env` gives the host's syllable counts (the same ones the `sing` line counts with), so that the line has a note for
/// each syllable the voice sings.
pub fn expand(src: &str, env: &Env) -> Option<Result<Expansion, Vec<Diagnostic>>> {
    expand_drawing(src, env).map(|r| r.map(|(e, _)| e))
}

/// [`expand`], and the programs a role drew on that the written-out text does not name (a stored set, whose tracks it does
/// name), (cid, text): what a link carries with the program (compile.rs `plays`).
pub fn expand_drawing(src: &str, env: &Env) -> Option<Result<(Expansion, Vec<(String, String)>), Vec<Diagnostic>>> {
    let norm = normalise_input(src);
    let src: &str = &norm;
    let lexed = lex(src);
    let toks = &lexed.tokens;
    let rs = runs(toks);
    if rs.is_empty() {
        return None;
    }
    let mut errors: Vec<Diagnostic> = Vec::new();
    if let Some(d) = version_check(src, &lexed.comments) {
        errors.push(d);
    }
    let known = names();
    let mut warnings = Vec::new();
    let mut drawn: Vec<(String, String)> = Vec::new();
    let mut pieces: Vec<(Span, String, String)> = Vec::new();
    let has = |w: &str| says(toks, w);
    // a seed that makes the whole arrangement too
    let seeded = crate::seed::runs(toks).iter().any(|r| arrange_from(toks, r.start).is_some());
    let mut whole_seen = false;
    for run in &rs {
        let t = match &run.r {
            Ok(t) => t.clone(),
            Err(d) => {
                errors.push(d.clone());
                continue;
            }
        };
        let Some(line_start) = arrange_from(toks, run.start) else {
            errors.push(Diagnostic::error(run.span, "template(...) goes after `arrange from`: it makes the whole track").help("arrange from template(deep-house, @tempest_such_stuff)"));
            continue;
        };
        if whole_seen || seeded {
            errors.push(Diagnostic::error(toks[line_start].span.to(run.span), "a program has one `arrange from`").help("a template or a seed makes the whole arrangement, not both; `section NAME from seed(...)` makes one section"));
            continue;
        }
        whole_seen = true;
        let Some(tpl) = find(&t.name) else {
            let mut d = Diagnostic::error(t.name_span, format!("`{}` is not a template", t.name)).expected(&known.iter().map(String::as_str).collect::<Vec<_>>()).found(format!("`{}`", t.name));
            d = match nearest(&t.name, known.iter().map(String::as_str)) {
                Some(n) => d.help(format!("did you mean `{n}`? The templates: {}", known.join(", "))),
                None => d.help(format!("the templates: {}", known.join(", "))),
            };
            errors.push(d);
            continue;
        };
        let src_phrase = match &t.phrase {
            PhraseRef::Id(id) => match phrasebook(id, env.voices) {
                Some(Ok(s)) => s,
                Some(Err(e)) => {
                    errors.push(Diagnostic::error(t.phrase_span, e));
                    continue;
                }
                None => {
                    let ids = djengine::vocal::phoneme::phrase_ids();
                    let mut d = Diagnostic::error(t.phrase_span, format!("`@{id}` is not a phrase of the phrasebook")).found(format!("`@{id}`"));
                    d = match crate::env::suggest(id, &ids, 3).first() {
                        Some(n) => d.help(format!("did you mean `@{n}`? or give the words in quotes: template({}, \"words\")", t.name)),
                        None => d.help(format!("phrases: {}; or give the words in quotes: template({}, \"words\")", ids.iter().take(6).map(|i| format!("@{i}")).collect::<Vec<_>>().join(", "), t.name)),
                    };
                    errors.push(d);
                    continue;
                }
            },
            PhraseRef::Name(n) => {
                errors.push(
                    Diagnostic::error(t.phrase_span, format!("no `use phrase` line names `{n}`: the phrase of a template is a phrasebook id or words in quotes"))
                        .help(format!("template({}, @tempest_such_stuff), or name the phrase first: use phrase @tempest_such_stuff as {n}", t.name)),
                );
                continue;
            }
            PhraseRef::Text(text) => match Profile::from_text(text, env.voices) {
                Ok(profile) => Source { profile, words: text.clone(), cite: None },
                Err(e) => {
                    errors.push(Diagnostic::error(t.phrase_span, e).help(format!("template({}, \"we are such stuff as dreams are made on\")", t.name)));
                    continue;
                }
            },
        };
        let line = match layout(&src_phrase.profile) {
            Ok(l) => l,
            Err(e) => {
                errors.push(Diagnostic::error(t.phrase_span, e).help("a shorter phrase, or one line of it"));
                continue;
            }
        };
        let p = &src_phrase.profile;
        warnings.push(Diagnostic::info(
            run.span,
            format!(
                "template {}: {} syllables, {}, {} feel, a vocal line of {} in sections of {} bars{}",
                tpl.name,
                p.syllables(),
                p.meter,
                p.feel.name(),
                line.length(),
                p.feel.section_bars(),
                if p.curated { "" } else { " (the stress is worked out from the words)" }
            ),
        ));
        let filled = slots(&t, &src_phrase, &line);
        let mut body = write_out(tpl, &filled, |w| !has(w));
        if !t.roles.is_empty() {
            // the roles bound (roles.rs): in the key and at the tempo the written-out program says at its top level
            let own = crate::gen::host_of(toks);
            let says_line = |w: &str| tpl.text().lines().find_map(|l| l.strip_prefix(w)).map(str::trim);
            let host = crate::gen::Host {
                key: own.key.or_else(|| says_line("key ").map(crate::music::canonical_pitch_word)),
                tempo: own.tempo.or_else(|| says_line("tempo ").and_then(|t| t.parse().ok())),
            };
            let key = host.key.clone().unwrap_or_else(|| "c".into());
            let mut cx = crate::roles::Ctx::new(env, host, &filled);
            match crate::roles::bind_form(&mut cx, tpl, &body, &t.roles, "", &key, 1) {
                Some(b) if cx.errors.is_empty() => {
                    body = b;
                    warnings.extend(cx.notes);
                    for d in cx.drawn {
                        if !drawn.contains(&d) {
                            drawn.push(d);
                        }
                    }
                }
                _ => {
                    errors.extend(cx.errors);
                    continue;
                }
            }
        }
        pieces.push((toks[line_start].span.to(run.span), body, t.text()));
    }
    if !errors.is_empty() {
        errors.sort_by_key(|d| d.span.start);
        return Some(Err(errors));
    }
    let mut text = String::with_capacity(src.len() * 4);
    let mut edits = Vec::new();
    let mut refs = Vec::new();
    let mut at = 0;
    for (sp, rep, r) in pieces {
        text.push_str(&src[at..sp.start]);
        let s = text.len();
        text.push_str(&rep);
        edits.push((sp, Span::new(s, text.len())));
        refs.push(r);
        at = sp.end;
    }
    text.push_str(&src[at..]);
    Some(Ok((Expansion { text, edits, refs, warnings }, drawn)))
}

/// Every reference of the program written out, the templates and then the seeds they leave: what `beatscript expand` and the
/// MCP `expand` tool give. Errors are at the program as given.
pub fn expand_all(src: &str, env: &Env) -> Option<Result<(String, Vec<String>, Vec<Diagnostic>), Vec<Diagnostic>>> {
    let Some(first) = expand(src, env) else {
        return crate::seed::expand(src).map(|r| r.map(|e| (e.text, e.refs, e.warnings)));
    };
    let first = match first {
        Ok(e) => e,
        Err(ds) => return Some(Err(ds)),
    };
    match crate::seed::expand(&first.text) {
        None => Some(Ok((first.text, first.refs, first.warnings))),
        Some(Err(ds)) => Some(Err(ds
            .into_iter()
            .map(|mut d| {
                d.span = first.back(d.span);
                d
            })
            .collect())),
        Some(Ok(second)) => {
            let mut refs = first.refs.clone();
            refs.extend(second.refs);
            let mut warnings = first.warnings.clone();
            warnings.extend(second.warnings.into_iter().map(|mut d| {
                d.span = first.back(d.span);
                d
            }));
            Some(Ok((second.text, refs, warnings)))
        }
    }
}

/// How many syllables a text sings (as the `sing` line counts them).
#[doc(hidden)]
pub fn syllable_count(text: &str, voices: Option<&dyn Voices>) -> usize {
    sing::syllables(text, voices).iter().map(LyricWord::syllables).sum()
}
