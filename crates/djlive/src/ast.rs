//! What the parser makes: statements, parts, mini-notation terms, control actions. Every node
//! that can be wrong carries its [`Span`]. Meaning (a word that is a note, a key, a chord) is
//! decided later, in [`crate::compile`], where the type of the place says what is expected.

use crate::diag::Span;
use crate::dur::{Dur, Pos};
use crate::lex::Token;

#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    pub stmts: Vec<Stmt>,
    /// The language version the text says it is in (its version line, version.rs): BeatScript 2's canonical text keeps the
    /// line, BeatScript 1's (the old spelling, and a text with no version line) does not.
    pub version: crate::version::Version,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stmt {
    pub span: Span,
    pub kind: StmtKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StmtKind {
    Setting(Setting),
    Line(Line),
    /// `let name = value`.
    Let(LetDef),
    /// `def name(a, b) { ... }`.
    Def(DefDef),
    /// `repeat 4 { ... }`.
    Repeat(RepeatBlock),
    /// `section drop 16 bars { ... }`.
    Section(SectionBlock),
    /// `arrange intro build drop`.
    Arrange(Vec<Word>),
    /// `import name-or-cid`.
    Import(ImportRef),
    /// `end`, `end at bar 64`, `end after outro`: where the program stops.
    End(EndWhen),
    /// `loop`: a song starts again from its first section when it reaches its end.
    Loop,
    /// `sample harp from "..." license restricted-private else bright-pluck`, `loop "name" as NAME`: a local file or an
    /// Apple Loop the program names by its hash (assets.rs).
    Sample(SampleDecl),
    /// `use restricted`: the program says it is private and may use restricted assets.
    UseRestricted,
    /// `set { play X  mix 8 bars with crossfade  play Y }`: programs of programs (set.rs).
    Set(SetBlock),
    /// `station { from gen@2 techno seed=42  next seed=~  forms seeded rondo }`: radio, an endless and deterministic run of
    /// tracks, position N always the same track (station.rs).
    Station(StationBlock),
    /// `evolve every 8 bars { melody: rotate 1 ... stop when gen == 12 }`.
    Evolve(EvolveBlock),
    /// `wire changes.out -> walker.chords`: an output of one def glued to an input of another (ports.rs).
    Wire(crate::ports::WireStmt),
    /// `arrange a stack(b, c)`: an arrange whose normal form is more than a list of names (algebra.rs). A plain
    /// `arrange a b c` stays `Arrange`.
    ArrangeExpr(Vec<crate::algebra::SExpr>),
    /// `let both = stack(a, b)`: a name for a composition of sections (algebra.rs).
    LetSection(crate::algebra::LetSection),
    /// `arrange open: intro build then set`: a named arrangement and how it ends (flow.rs).
    ArrangeNamed(ArrDecl),
    /// `start open`: the named arrangement that plays first (default: the first written).
    Start(Word),
    /// `launch drop on pad(2) next phrase 16`: a pad brings a scene in on a bar line (launch.rs).
    Launch(LaunchStmt),
    /// `assert length == 3m30s ± 5s`: a statement the program makes about itself (assertion.rs).
    Assert(crate::assertion::AssertStmt),
    /// `arrange { intro drop  ~ acid rot 2  drop } loop 4 then end`: an arrangement with mutations (mutate.rs). Lowered
    /// to a plain `Arrange` by [`crate::mutate::apply`] before expansion.
    ArrangeBlock(crate::mutate::ArrangeBlock),
    /// `section intro from seed(techno, 42)[1..8]`, `arrange from seed(techno, 42)`: sections of a generated track
    /// (seed.rs). Written out before the program is lowered, so the lowering never sees one.
    Seed(SeedStmt),
}

/// `launch NAME on pad(N) [next bar | next phrase [8 | 16 | 32]]`: the scene (a section or a named arrangement), the pad and the bar line.
#[derive(Clone, Debug, PartialEq)]
pub struct LaunchStmt {
    pub scene: Word,
    pub pad: u32,
    /// `pad(2)`, for an error about the pad.
    pub pad_span: Span,
    pub quantum: LaunchQuantum,
}

/// The bar line a launch lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchQuantum {
    /// `next bar`: the next bar line the engine has time to load.
    Bar,
    /// `next phrase`, `next phrase 16`: every 8, 16 or 32 bars of the song (8 when it says no more).
    Phrase(u32),
}

impl LaunchQuantum {
    /// Every this many bars: 1 for the next bar line.
    pub fn bars(self) -> u32 {
        match self {
            LaunchQuantum::Bar => 1,
            LaunchQuantum::Phrase(n) => n,
        }
    }
}

/// A statement made from a seed reference (seed.rs): `section NAME from REF` (`section` set) or `arrange from REF`.
#[derive(Clone, Debug, PartialEq)]
pub struct SeedStmt {
    pub section: Option<Word>,
    /// The reference, in its canonical spelling (`seed(techno, 42)[1..8]`).
    pub what: Word,
}

/// A named arrangement: `arrange NAME: a b c` and its ending.
#[derive(Clone, Debug, PartialEq)]
pub struct ArrDecl {
    pub name: Word,
    pub items: Vec<crate::algebra::SExpr>,
    pub ends: ArrEnd,
}

/// How a named arrangement ends.
#[derive(Clone, Debug, PartialEq)]
pub enum ArrEnd {
    /// Nothing said: on to the next arrangement written, or the end of the song if it is the last.
    Fall,
    /// `then NAME`.
    Then(Word),
    /// `loop`: itself again, for ever.
    Loop,
    /// `loop N`: N passes in all, then as `Fall`.
    Times(u32, crate::diag::Span),
    /// `end`.
    End,
}

/// A program that applies transforms to itself over time: generation n is the rules applied n times to
/// generation 0 (the program as written).
#[derive(Clone, Debug, PartialEq)]
pub struct EvolveBlock {
    /// A new generation every this many bars.
    pub every: u32,
    pub rules: Vec<EvolveRule>,
    /// The last generation: `stop when gen == 12` or `for 12 gens`.
    pub last: Option<(u32, Span)>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EvolveRule {
    pub span: Span,
    pub target: EvolveTarget,
    pub ops: Vec<EvolveOp>,
    pub when: Option<GenCond>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EvolveTarget {
    /// `melody:`: that role on every deck.
    Role(Word),
    /// `deck A:`: every part of the deck.
    Deck(Deck),
}

#[derive(Clone, Debug, PartialEq)]
pub struct EvolveOp {
    pub name: Word,
    /// Numbers: a plain one, or `<0 5 7 0>` (one per generation, round).
    pub args: Vec<EvolveArg>,
    pub seed: Option<Num>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EvolveArg {
    Num(Num),
    Alt(Vec<Num>),
}

/// `gen == 12`, `gen % 4 == 3`, `gen >= 8`.
#[derive(Clone, Debug, PartialEq)]
pub struct GenCond {
    pub modulo: Option<u32>,
    pub op: CmpOp,
    pub value: u32,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl CmpOp {
    pub fn text(self) -> &'static str {
        match self {
            CmpOp::Eq => "==",
            CmpOp::Ne => "!=",
            CmpOp::Lt => "<",
            CmpOp::Le => "<=",
            CmpOp::Gt => ">",
            CmpOp::Ge => ">=",
        }
    }

    pub fn holds(self, a: u32, b: u32) -> bool {
        match self {
            CmpOp::Eq => a == b,
            CmpOp::Ne => a != b,
            CmpOp::Lt => a < b,
            CmpOp::Le => a <= b,
            CmpOp::Gt => a > b,
            CmpOp::Ge => a >= b,
        }
    }
}

impl GenCond {
    pub fn holds(&self, gen: u32) -> bool {
        let g = self.modulo.map_or(gen, |m| gen % m.max(1));
        self.op.holds(g, self.value)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum EndWhen {
    /// After the last section.
    Song,
    At(u32),
    After(Word),
}

/// `let name = tokens`: a value (one token) or a pattern (several), kept as written and read where
/// it is used.
#[derive(Clone, Debug, PartialEq)]
pub struct LetDef {
    pub name: Word,
    pub value: Vec<Token>,
    /// The value is a number that came of arithmetic and is a ratio no short decimal spells (`let third = 1/3`): it is
    /// printed as the fraction, and so is every use of the name that was put in place of it.
    pub ratio: bool,
}

/// `def name(params) { body }`: the body is tokens, parsed when a call expands it.
#[derive(Clone, Debug, PartialEq)]
pub struct DefDef {
    pub name: Word,
    pub params: Vec<Word>,
    pub body: Vec<Token>,
    /// `in: chords, key out: bass`: the def's boundary (empty for a plain macro).
    pub ports: crate::ports::Ports,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RepeatBlock {
    pub n: u32,
    /// `repeat 4 every 2 bars { ... }`: copy `i` is shifted `i * every` bars.
    pub every: Option<u32>,
    pub body: Vec<Token>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SectionBlock {
    pub name: Word,
    pub bars: u32,
    pub bars_span: Span,
    pub body: Vec<Stmt>,
    /// Never written: the lowering of an `arrange { }` with a switch of the control deck (`~ control off`, mutate.rs) sets it on
    /// the copy of a section a play reads: the calls (`move`, `freeze` ...) whose control-deck lines do not fire in that copy.
    /// expand.rs holds those lines back (checked, never fired). Empty for every section as written.
    pub quiet: Vec<String>,
}

/// `set { ... }`: tracks and the mixes between them, in order.
#[derive(Clone, Debug, PartialEq)]
pub struct SetBlock {
    pub items: Vec<SetItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SetItem {
    Play(PlayItem),
    Mix(MixItem),
    /// `next seed=+1`: the track before it with its seed stepped (station.rs).
    Next(NextItem),
}

/// `next seed=+N [until WHEN]` in a set: the next track is the track before it (a generator call) with its seed stepped.
#[derive(Clone, Debug, PartialEq)]
pub struct NextItem {
    pub step: SeedStep,
    pub until: Option<(UntilWhen, Span)>,
    pub span: Span,
}

/// A step from a track to the next: `seed=+N` counts on, `seed=~` draws a seed from a hash of the CID of the track before.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeedStep {
    By(u32),
    Hash,
}

/// `station { ... }`: its lines, in order.
#[derive(Clone, Debug, PartialEq)]
pub struct StationBlock {
    pub items: Vec<StationItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StationItem {
    /// `from gen@2 techno seed=42`: the first track's call (one word, as the reader collapsed it).
    From(Word, Span),
    /// `next seed=~`.
    Next(SeedStep, Span),
    /// `forms seeded passacaglia rondo`: the forms, in turn.
    Forms(Vec<Word>, Span),
}

/// `play NAME [until WHEN]`: a saved program. Canonical text holds its CID.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayItem {
    pub what: Word,
    pub until: Option<(UntilWhen, Span)>,
    pub span: Span,
}

/// Where a track is cut.
#[derive(Clone, Debug, PartialEq)]
pub enum UntilWhen {
    /// `20m`, `90s`, `1h`: seconds, read at the track's tempo.
    Seconds(f64),
    /// `bar 96`.
    Bar(u32),
    /// `drop`: where that section starts.
    Section(String),
    /// `after drop`: where it ends.
    After(String),
}

/// `mix 8 bars with filter_exit, tempo 140 -> 130`.
#[derive(Clone, Debug, PartialEq)]
pub struct MixItem {
    pub bars: u32,
    pub bars_span: Span,
    pub with: Word,
    pub tempo: Option<(f64, f64)>,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleKind {
    /// `sample NAME from "SRC"`: a file of the user's.
    Sample,
    /// `loop "NAME-OR-HASH" as NAME`: an Apple Loop.
    Loop,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SampleDecl {
    pub kind: SampleKind,
    /// What the program calls it: `inst=NAME`.
    pub name: Word,
    /// A path or `sha256:HEX` (a sample), a name or hash (a loop). Canonical text holds `sha256:HEX`.
    pub src: Word,
    /// `license TAG` (a sample only).
    pub license: Option<Word>,
    /// `else VOICE`: an original stand-in voice.
    pub otherwise: Option<Word>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImportRef {
    /// A snippet's name, a CID, or a part of one.
    pub what: Word,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Setting {
    Tempo(Num),
    Swing(Num),
    Key(Word),
    /// `tempo 100 -> 130 over 8 bars [shape]`: a tempo that moves (F3, tempo.rs).
    TempoRamp(crate::tempo::TempoRamp),
}

/// A number as written, with where. A number that was arithmetic (`60 + 6`, `(2 + 1) * 4`) carries its value
/// folded and, for the canonical text, the expression as [`crate::expr::Expr::canonical`] prints it
/// (`canon`; `None` for a single literal, whose spelling is the number's own).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Num {
    pub value: f64,
    pub int: bool,
    pub span: Span,
    /// The canonical text of the expression this number came from (interned: see [`crate::expr::intern`]).
    pub canon: Option<&'static str>,
}

impl Num {
    /// A number written as a literal: its spelling is its value's.
    pub fn lit(value: f64, int: bool, span: Span) -> Num {
        Num { value, int, span, canon: None }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub text: String,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Deck {
    A,
    B,
}

impl Deck {
    pub fn name(self) -> &'static str {
        match self {
            Deck::A => "A",
            Deck::B => "B",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum When {
    Every(u32),
    /// `every 4 bars from 8`.
    EveryFrom(u32, u32),
    Phrase,
    At(u32),
    /// `on next bar`.
    NextBar,
    /// `on next phrase [8|16|32]`.
    NextPhrase(u32),
    /// `after drop`: the bar the section ends on.
    After(Word),
    /// `before drop 4 bars`: that many bars ahead of the section.
    Before(Word, u32),
    /// `for 4 bars:` alone: now, held.
    Now,
    /// `every 2 beats`, `every 1/2 bar from 3`, `every 2 bars from bar 1 beat 2`: a step or a start that is not whole
    /// bars (the whole-bar forms are [`When::Every`] and [`When::EveryFrom`]). With no `from` it starts one step in.
    EveryPos(Dur, Option<Pos>),
    /// `at bar 16 beat 3`: a position that is not on a bar line.
    AtPos(Pos),
    /// `before drop 2 beats`: a distance that is not whole bars.
    BeforeDur(Word, Dur),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    Deck(Deck),
    Master,
}

/// `[when:] [target:] body`.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub when: Option<(When, Span)>,
    /// `for 4 bars`, `for 2 beats`: undone after that long.
    pub hold: Option<(Dur, Span)>,
    pub target: Option<(Target, Span)>,
    /// `control:`, the control deck (docs/beatscript/ext-control.md): the head of a line of engine calls only, in the place a target
    /// takes. The line has no target, and it is lowered as the same line without the head (expand.rs drops it), so
    /// `control: move echo_out on B` compiles to what `move echo_out on B` does. The span is the head's, `control` to `:`.
    pub control: Option<Span>,
    pub body: Body,
    /// `when $i % 4 == 3` after the line: it is kept when the condition holds (docs/beatscript/ext-arith.md).
    pub guard: Option<Guard>,
}

/// A `when <condition>` guard after a line: evaluated where the line is expanded, so inside a `repeat` or a
/// `def` it sees `$i`, `$n` and the `$params` (already put in place as tokens).
#[derive(Clone, Debug, PartialEq)]
pub struct Guard {
    pub expr: crate::expr::Expr,
    /// From the `when` to the end of the condition.
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    Parts(Vec<Part>),
    Action(Action),
    /// `do noun verb key=value ...`, or the sugar for it (`vocoder on`, `freeze on deck=A`).
    Do(DoCall),
    /// `drop(A, 8)`: a call of a `def`.
    Invoke(Invocation),
    /// `duck B under A.kick 6db`: the engine's sidechain (mixmoves.rs).
    Duck(crate::mixmoves::DuckStmt),
    /// `mix { A level 0.7, B eq low -3 }`: the mixer set, and put back at the end of its section (mixmoves.rs).
    Mix(crate::mixmoves::MixBlock),
    /// `sing "robots on the road" on melody voice=alto`: typed lyrics sung on a part's notes (sing.rs).
    Sing(crate::sing::SingCall),
}

impl Body {
    /// The name an `arrange { }` switch gives a control-deck line by (`~ control freeze off`, docs/beatscript/ext-control.md): the
    /// call's first word as the line writes it (`move`, `swap`, `load`, `queue`, `crossfader`, `tempo`, `duck`, `mix`), or the
    /// registry noun of a `do` and of its sugar (`do vocoder ...` and `vocoder ...` are both `vocoder`). None for a body that
    /// is no engine call (parts, a def's call, a `sing`).
    pub fn call_name(&self) -> Option<&str> {
        match self {
            Body::Action(a) => Some(match a {
                Action::Move { .. } => "move",
                Action::SwapPairs { .. } => "swap",
                Action::LoadPair { .. } | Action::Load { .. } => "load",
                Action::Queue { .. } => "queue",
                Action::Tempo(_) => "tempo",
                Action::Transport(w) => w.text.as_str(),
                Action::Change { name, .. } | Action::Shaped { name, .. } => name.text.as_str(),
            }),
            Body::Do(d) => Some(d.noun.text.as_str()),
            Body::Duck(_) => Some("duck"),
            Body::Mix(_) => Some("mix"),
            Body::Parts(_) | Body::Invoke(_) | Body::Sing(_) => None,
        }
    }
}

/// A call on the control-language registry, written out.
#[derive(Clone, Debug, PartialEq)]
pub struct DoCall {
    /// Written with no `do`: the first word was the noun.
    pub sugar: bool,
    pub noun: Word,
    pub verb: Option<Word>,
    /// Strings written before the keys: they fill the verb's first required text param.
    pub positional: Vec<(String, Span)>,
    pub params: Vec<DoParam>,
    /// `over 8 bars`, `over 2 beats`: the ramped param moves that long.
    pub over: Option<(Dur, Span)>,
    /// `exp`, `s-curve`, `step 4` after the `over`: how the ramp goes (F4, shape.rs).
    pub shape: Option<(crate::shape::Shape, Span)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DoParam {
    pub span: Span,
    pub name: Word,
    pub value: DoValue,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DoValue {
    Num(Num),
    /// `0 -> 1`: a ramp (needs `over N bars`).
    Ramp(Num, Num),
    Word(Word),
    Str(String, Span),
    List(Vec<DoValue>, Span),
    /// `lfo tri 1 bar depth=0.2 center=0.3`: a value that keeps moving (F4, shape.rs).
    Lfo(crate::shape::LfoSpec),
}

impl DoValue {
    pub fn span(&self) -> Span {
        match self {
            DoValue::Num(n) => n.span,
            DoValue::Ramp(a, b) => a.span.to(b.span),
            DoValue::Word(w) => w.span,
            DoValue::Str(_, s) | DoValue::List(_, s) => *s,
            DoValue::Lfo(l) => l.span,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Invocation {
    pub name: Word,
    /// Each argument as the tokens that were written.
    pub args: Vec<Vec<Token>>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub span: Span,
    pub role: Word,
    /// `hihat/12`: the part's own cycle is this many sixteenth steps (F6, meter.rs); None is the bar (16).
    pub cycle: Option<(u32, Span)>,
    pub pattern: Seq,
    pub params: Vec<Param>,
    /// Never written: the lowering of an `arrange { }` with a `jux` (mutate.rs) puts the right copy of a part right
    /// after the part it copies, as a part of the same role with this set. The compiler plays it as that part's twin.
    pub right: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub span: Span,
    pub name: Word,
    pub value: ParamValue,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ParamValue {
    Num(Num),
    Range(Num, Num),
    Word(Word),
}

/// A sequence of terms sharing their slot equally.
#[derive(Clone, Debug, PartialEq)]
pub struct Seq {
    pub span: Span,
    pub terms: Vec<Term>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Term {
    pub span: Span,
    pub atom: Atom,
    /// `*n`.
    pub fast: Option<u32>,
    /// `e2!`: an accent.
    pub accent: bool,
    /// `e3^`: a slide into the next step.
    pub slide: bool,
    /// `x.` a ghost, `x&` a flam, `x:3` a ratchet: how the hit is played (the expression marks).
    pub marks: Marks,
}

/// The expression marks of a term beyond accent and slide. All off is a term as it always was:
/// it prints, plays and hashes the same.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Marks {
    /// `x.`: a ghost note, played at [`GHOST_PERCENT`] of its velocity.
    pub ghost: bool,
    /// `x&`: a flam, a soft grace hit just before the note.
    pub flam: bool,
    /// `x:3`: a ratchet, the hit struck this many times (2 to 8) inside its sixteenth step.
    pub ratchet: Option<u8>,
}

/// A ghost note's velocity, as a percentage of the velocity it would have had.
pub const GHOST_PERCENT: i32 = 40;
/// The fewest and the most strikes a ratchet (`x:N`) divides its step into.
pub const RATCHET_RANGE: (u8, u8) = (2, 8);

impl Marks {
    pub fn is_plain(&self) -> bool {
        *self == Marks::default()
    }

    /// The marks as the canonical text writes them after `!` / `^`: `.`, `&`, `:N`.
    pub fn text(&self) -> String {
        let mut s = String::new();
        if self.ghost {
            s.push('.');
        }
        if self.flam {
            s.push('&');
        }
        if let Some(n) = self.ratchet {
            s.push_str(&format!(":{n}"));
        }
        s
    }

    /// An outer term's marks laid over an inner one's (a mark on a group marks every hit in it).
    pub fn or(self, o: Marks) -> Marks {
        Marks { ghost: self.ghost || o.ghost, flam: self.flam || o.flam, ratchet: o.ratchet.or(self.ratchet) }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Atom {
    Hit,
    Rest,
    /// A note, a chord symbol or anything else word-shaped; read when it is evaluated.
    Note(Word),
    Degree(u32),
    /// `[a b, c d]`: each `Seq` is squeezed into the slot; several are stacked.
    Group(Vec<Seq>),
    /// `<a b c>`: one term per cycle.
    Alt(Vec<Term>),
    Call(Call),
    /// `x*8?0.7`: the term with each of its hits kept by chance (F7, chance.rs).
    Chance(Box<crate::chance::ChanceTerm>),
    /// `choose [a b c] weights 3 1 1 seed=4`: one option a bar (F7, chance.rs).
    Choose(Box<crate::chance::Choose>),
    /// `deg(1 b3 5' 1,)`: a group of degrees, one slot like `[..]` (the sequences are stacked). F5, scale.rs.
    Deg(Vec<Seq>),
    /// One degree inside `deg( )`: a number, an accidental, octave marks. F5, scale.rs.
    ScaleDeg(crate::scale::ScaleDeg),
    /// `c2~3` (elongate): pattern sugar, written out before it is evaluated (sugar.rs).
    Sized(Box<crate::sugar::Sized>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub name: Word,
    pub args: Vec<Arg>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    Num(Num),
    /// A key, a chord, a mode: `am`, `am7`, `up`, `d:dorian`.
    Word(Word),
    Named(Word, Box<Arg>),
    /// `[c2 ~ e2]`: a pattern given to a transform (`reverse([c2 ~ e2])`), or a `let` pattern's name once read.
    Pat(Seq),
    /// A generator or transform inside another: `reverse(transpose(riff, 5))`.
    Call(Box<Call>),
}

impl Arg {
    pub fn span(&self) -> Span {
        match self {
            Arg::Num(n) => n.span,
            Arg::Word(w) => w.span,
            Arg::Named(n, v) => n.span.to(v.span()),
            Arg::Pat(s) => s.span,
            Arg::Call(c) => c.span,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    SwapPairs { with: Option<Word> },
    Move { kind: Word, deck: Option<Deck>, over: Option<(Dur, Span)> },
    LoadPair { spec: Word, group: Option<Num> },
    Load { spec: Word },
    Queue { verb: Word, arg: Option<Word>, n: Option<Num> },
    Transport(Word),
    /// `filter lp 2k -> 8k over 8 bars`, `delay 0.4`, `level 0.8`: a name, its mode word (`lp` / `hp` / `off` /
    /// `open`), the value and where it goes, and how long it takes.
    Change { name: Word, mode: Option<Word>, from: Option<Num>, to: Option<Num>, over: Option<(Dur, Span)> },
    /// A change that is a ramp with a shape or an lfo (F4, shape.rs): `filter lp 300 -> 2k over 8 bars exp`,
    /// `level lfo sine 2 bars depth=0.2 center=0.7`.
    Shaped { name: Word, mode: Option<Word>, motion: crate::shape::Motion },
    /// `tempo 128 -> 140 over 4 bars` on a timed line: `at bar 16: tempo 128 -> 140 over 4 bars` (F3, tempo.rs).
    Tempo(crate::tempo::TempoSpec),
    /// `macro riser on A over 8 bars`, `macro@1 breakdown over 4 bars on A`: an entry of the DJ macro catalogue by name
    /// (macros.rs). `version` is 1 for a bare `macro`; `head` is the span of `macro` / `macro@N`.
    Macro { version: u32, head: Span, id: Word, deck: Option<Deck>, over: Option<(Dur, Span)> },
}
