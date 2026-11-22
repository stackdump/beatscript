//! From the surface language to flat lines: `let` patterns put where they are used, `def` calls and
//! `repeat` blocks expanded, `section`s laid on the `arrange` timeline, every time word turned into
//! the bar it means. What comes out is the plain language of docs/BTW.md's first pages (parts, controls) plus
//! when each control fires, and which part of the song each line is in.
//!
//! Everything here is total. There is no recursion (a `def` that calls itself, even through
//! another, is an error found by [`call_cycles`]), a `repeat` count is a number written out, a
//! `def`'s body is expanded once per call, and the expansion is capped ([`MAX_LINES`]). So a
//! program always ends, and the same text always expands to the same lines.

use std::collections::{BTreeMap, BTreeSet};

use crate::ast::*;
use crate::compile::Timing;
use crate::diag::{Diagnostic, Span};
use crate::dur::Dur;
use crate::lex::{Tok, Token};
use crate::parse::{self, parse_seq_tokens, parse_tokens, Parsed};
use crate::print;
use crate::rat::Rat;

/// The most flat lines a program may expand to.
pub const MAX_LINES: usize = 4096;
/// How deep calls and repeats may nest.
pub const MAX_DEPTH: usize = 8;
/// The most text (in tokens, with a fixed cost for each copy) that repeats and calls may write out in all. Lines are
/// capped by [`MAX_LINES`], but a copy that produces no line (a false `when`, a body of comments) costs time all the
/// same, and nested repeats multiply: this is the cap on that work. A program that fits the line cap fits this.
pub const MAX_WORK: usize = 1 << 21;
/// What starting a copy of a repeat, or a call, costs before its text is counted.
const COPY_COST: usize = 32;
/// The most terms a pattern may have once the `let`s in it are put in place, and the deepest it may nest then. A `let`
/// that names two copies of the one before doubles each time (`let b = a a`, `let c = b b`, ...: 2^40 terms in forty
/// lines), and one that wraps the one before in a group deepens it by one each time; both are counted from the text,
/// before anything is built.
pub const MAX_PATTERN_NODES: usize = 16_384;
pub const MAX_PATTERN_DEPTH: usize = 128;
/// The most errors a `repeat` says about its copies.
const MAX_COPY_ERRORS: usize = 16;
/// The longest a song may be, in bars.
pub const MAX_SONG_BARS: u32 = crate::compile::MAX_SONG_BARS as u32;

/// One statement, expanded: the line, and when it fires.
#[derive(Clone, Debug)]
pub struct FlatLine {
    pub span: Span,
    pub line: Line,
    pub timing: Timing,
    /// `for N bars`, `for 2 beats`: undone after that long.
    pub hold: Option<Dur>,
    /// Where it came from, for the control's identity: `[intro] [drop(A, 8)] `.
    pub path: String,
    /// The calls and repeats that expanded it, innermost first: where in the text to light when it is playing.
    pub via: Vec<Span>,
    /// F10: this line is a copy carried into a later stretch of a stacked section, `Some(k)` the bar of its section
    /// the stretch begins at. Its parts play from there (their alternations are phased); its controls were fired in
    /// the section's first stretch, so they are not added again.
    pub phase: Option<u32>,
    /// No time word anywhere in its way: not its own, not a call's that expanded it, not a repeat's. A performed value on such a
    /// line follows its input (perform.rs, "Follow").
    pub untimed: bool,
}

/// One stretch of the arrangement.
#[derive(Clone, Debug)]
pub struct SegmentLines {
    pub name: String,
    pub start: u32,
    pub bars: u32,
    pub lines: Vec<FlatLine>,
    /// The `section ... {` statement: its first line is the section's header in the text.
    pub header: Option<Span>,
    /// The section's own `key` (the first statements of its body); None: the program's. F5, scale.rs.
    pub key: Option<Word>,
    /// F10: a later stretch of one item of the `arrange` (a stack of unequal lengths): the runtime reads it as the
    /// same section as the stretch before.
    pub chain: bool,
    /// F10: the headers of the other sections playing in this stretch (the playhead sits on `header`).
    pub header_more: Vec<Span>,
    /// F10: how many bars into its item of the `arrange` this stretch begins (0 unless `chain`): the song's own parts
    /// (outside every section) go on from there, as the parts of a section do.
    pub phase: u32,
}

/// What `check` says of a program's named arrangements.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FlowReport {
    /// (name, how it ends: `then X`, `loop`, `loop N`, `end` or `` for falling through).
    pub arrangements: Vec<(String, String)>,
    /// The arrangements in the order they play (until the song ends or a lap begins).
    pub order: Vec<String>,
    /// The index in `order` the song goes back to, if it loops.
    pub loop_from: Option<usize>,
    pub unreachable: Vec<String>,
    /// Plays `then` back to an earlier arrangement with no `end`: the song never finishes.
    pub cycle: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Lowered {
    /// There is an `arrange` (or sections, played in order): the song has a timeline.
    pub arranged: bool,
    /// Lines outside every section: they apply the whole song.
    pub globals: Vec<FlatLine>,
    pub segments: Vec<SegmentLines>,
    /// The bar the program ends on (of the song when it has a timeline, else the program's), if it does.
    pub end: Option<u32>,
    /// A song starts again from its first section at its end.
    pub looped: bool,
    /// The bar a looping song goes back to at its end (0: its first; more only for a flow of named arrangements).
    pub loop_from: u32,
    /// The flow of named arrangements, when the program has them (flow.rs).
    pub flow: Option<FlowReport>,
    /// `evolve` blocks: the segment each is in, and the block.
    pub evolves: Vec<(usize, EvolveBlock)>,
    /// A top-level `evolve` with no `stop`: it runs the orbit of its parts, and its timeline is made when that is known.
    pub top_unbounded: bool,
    /// `end` was not said: it is the song's end, which an unbounded evolve moves.
    pub end_default: bool,
    /// Every section the arrangement places, with its start bar and the `key` it sets (F14, linked.rs): what the JSON-LD
    /// lists as the song's sections, where the segments are stretches (a stack cuts a song where its parts start and end).
    pub placements: Vec<crate::linked::Placed>,
    /// The `launch` statements, checked (launch.rs).
    pub launches: Vec<crate::launch::Launch>,
}

pub(crate) struct DefInfo {
    pub(crate) def: DefDef,
    /// A call's trace names the import it came from.
    pub(crate) imported: bool,
}

pub(crate) struct SectionInfo<'a> {
    pub(crate) block: &'a SectionBlock,
    pub(crate) span: Span,
}

pub(crate) struct Lower<'a> {
    pub(crate) src: &'a str,
    pub(crate) diags: Vec<Diagnostic>,
    pub(crate) defs: BTreeMap<String, DefInfo>,
    /// Patterns named by `let`, with their own lets already put in.
    pub(crate) pats: BTreeMap<String, Seq>,
    /// (terms, depth) of each of them once put in place ([`MAX_PATTERN_NODES`]).
    pub(crate) dims: BTreeMap<String, (usize, usize)>,
    /// section name -> every (start, bars) it has on the timeline.
    pub(crate) places: BTreeMap<String, Vec<(u32, u32)>>,
    pub(crate) arranged: bool,
    pub(crate) cyclic: BTreeSet<String>,
    /// Names a `def`, a `let` or a `section` was refused (a word of the language, a note, a second one of the same
    /// name): the error is said at the definition, and not again at each of its uses (`there is no section`, `is not a def`).
    pub(crate) declined: BTreeSet<String>,
    pub(crate) produced: usize,
    /// The text repeats and calls have written out so far ([`MAX_WORK`]).
    pub(crate) work: usize,
    pub(crate) over_budget: bool,
    pub(crate) evolves: Vec<(usize, EvolveBlock)>,
    /// Typed ports on defs and the wires between them (ports.rs).
    pub(crate) ports: crate::ports::PortState,
}

#[derive(Clone)]
pub(crate) struct Scope {
    /// The bar of the timeline this code starts at (None: the song's own times).
    pub(crate) base: Option<u32>,
    pub(crate) depth: usize,
    pub(crate) path: String,
    /// Which segment the lines go to (None: the globals).
    pub(crate) segment: Option<usize>,
    /// The calls and repeats this code was expanded from, innermost first.
    pub(crate) via: Vec<Span>,
    /// A time word of a call, or a repeat, put this code where it is: the lines in it are timed whatever they say themselves.
    pub(crate) timed: bool,
}

/// Whether a line says no time: none written, or the `now` and `on next bar` that mean the same.
fn says_no_time(l: &Line) -> bool {
    matches!(&l.when, None | Some((When::Now, _)) | Some((When::NextBar, _)))
}

pub fn lower(parsed: &Parsed, src: &str, diags: &mut Vec<Diagnostic>) -> Lowered {
    let mut lw = Lower {
        src,
        diags: Vec::new(),
        defs: BTreeMap::new(),
        pats: BTreeMap::new(),
        dims: BTreeMap::new(),
        places: BTreeMap::new(),
        arranged: false,
        cyclic: BTreeSet::new(),
        declined: BTreeSet::new(),
        produced: 0,
        work: 0,
        over_budget: false,
        evolves: Vec::new(),
        ports: Default::default(),
    };
    let out = lw.run(parsed);
    diags.extend(lw.diags);
    out
}

impl<'a> Lower<'a> {
    pub(crate) fn err(&mut self, d: Diagnostic) {
        self.diags.push(d);
    }

    pub(crate) fn run(&mut self, parsed: &Parsed) -> Lowered {
        // ---- names: lets and defs (imported ones first)
        let mut names: BTreeMap<String, Span> = BTreeMap::new();
        let note_name = |lw: &mut Lower, name: &Word, what: &str, names: &mut BTreeMap<String, Span>| -> bool {
            if let Some(why) = crate::parse::reserved_as(&name.text, what) {
                lw.err(Diagnostic::error(name.span, format!("`{}` cannot name a {what}: {why}", name.text)).help(crate::parse::rename_hint(&name.text)));
                lw.declined.insert(name.text.clone());
                return false;
            }
            if let Some(prev) = names.get(&name.text) {
                let at = crate::diag::line_col(lw.src, prev.start);
                lw.err(Diagnostic::error(name.span, format!("`{}` is already a name (line {})", name.text, at.0)).help("a let, a def and a section each need a name of their own"));
                return false;
            }
            names.insert(name.text.clone(), name.span);
            true
        };
        let mut lets: Vec<&LetDef> = Vec::new();
        for imp in &parsed.imports {
            for l in &imp.lets {
                lets.push(l);
            }
            for d in &imp.defs {
                if note_name(self, &d.name, "def", &mut names) {
                    self.defs.insert(d.name.text.clone(), DefInfo { def: d.clone(), imported: true });
                }
            }
        }
        for st in &parsed.program.stmts {
            match &st.kind {
                StmtKind::Let(l) => lets.push(l),
                StmtKind::Def(d) => {
                    if note_name(self, &d.name, "def", &mut names) {
                        self.defs.insert(d.name.text.clone(), DefInfo { def: d.clone(), imported: false });
                    }
                }
                _ => {}
            }
        }
        // the lets: a one-token value was put in place before the text was read; a longer one is a pattern
        let mut scalar_seen: BTreeSet<String> = BTreeSet::new();
        for l in lets {
            // (a value of a number and a unit, `5ms`, is a scalar too: it was put in place of the name as both tokens)
            if l.value.len() == 1 || crate::parse::is_unit_value(&l.value) {
                if !scalar_seen.insert(l.name.text.clone()) || names.contains_key(&l.name.text) {
                    let d = Diagnostic::error(l.name.span, format!("`{}` is already a name", l.name.text));
                    self.err(d.help("a let, a def and a section each need a name of their own: a let of one value is put in place of its name wherever it stands, so rename one"));
                }
                continue;
            }
            if !note_name(self, &l.name, "let", &mut names) {
                continue;
            }
            match parse_seq_tokens(l.value.clone(), self.src) {
                Ok(seq) => {
                    let (nodes, depth) = self.dims_seq(&seq);
                    if nodes > MAX_PATTERN_NODES || depth > MAX_PATTERN_DEPTH {
                        self.err(self.pattern_too_big(l.name.span, &format!("`{}`", l.name.text), nodes, depth));
                        // it stands for a rest, so that each use does not say again that it cannot be read
                        let rest = Seq { span: seq.span, terms: vec![Term { span: seq.span, atom: Atom::Rest, fast: None, accent: false, slide: false, marks: Marks::default() }] };
                        self.dims.insert(l.name.text.clone(), (1, 1));
                        self.pats.insert(l.name.text.clone(), rest);
                        continue;
                    }
                    let seq = self.put_lets(&seq);
                    self.dims.insert(l.name.text.clone(), (nodes, depth));
                    self.pats.insert(l.name.text.clone(), seq);
                }
                Err(d) => {
                    self.err(Diagnostic { help: Some(d.help.clone().unwrap_or_else(|| format!("`let {} = ...` takes a pattern (or a single value)", l.name.text))), ..d });
                    // it stands for a rest, so that each use does not say again that it is not a step
                    let at = l.name.span;
                    let rest = Seq { span: at, terms: vec![Term { span: at, atom: Atom::Rest, fast: None, accent: false, slide: false, marks: Marks::default() }] };
                    self.dims.insert(l.name.text.clone(), (1, 1));
                    self.pats.insert(l.name.text.clone(), rest);
                }
            }
        }
        // ---- recursion
        self.call_cycles();
        // ---- ports and wires
        self.ports_setup(parsed);
        // ---- sections and the arrangement
        let mut sections: BTreeMap<String, SectionInfo> = BTreeMap::new();
        let mut section_order: Vec<String> = Vec::new();
        // F10: the items of the `arrange` (names, `stack(...)`, `then(...)`), and the `let NAME = stack(...)` compositions
        let mut arrange: Option<(Vec<crate::algebra::SExpr>, Span)> = None;
        let mut section_lets: Vec<&crate::algebra::LetSection> = Vec::new();
        let mut arrs: Vec<(&crate::ast::ArrDecl, Span)> = Vec::new();
        let mut start_word: Option<Word> = None;
        for st in &parsed.program.stmts {
            match &st.kind {
                StmtKind::ArrangeNamed(d) => {
                    if note_name(self, &d.name, "arrangement", &mut names) {
                        arrs.push((d, st.span));
                    }
                }
                StmtKind::Start(w) => {
                    if start_word.is_some() {
                        self.err(Diagnostic::error(st.span, "`start` is said once"));
                    } else {
                        start_word = Some(w.clone());
                    }
                }
                StmtKind::Section(b) => {
                    if !note_name(self, &b.name, "section", &mut names) {
                        continue;
                    }
                    section_order.push(b.name.text.clone());
                    sections.insert(b.name.text.clone(), SectionInfo { block: b, span: st.span });
                }
                StmtKind::Arrange(_) | StmtKind::ArrangeExpr(_) => {
                    if arrange.is_some() {
                        self.err(Diagnostic::error(st.span, "a song has one `arrange`").help("list every section in one line: arrange intro build drop"));
                    } else {
                        arrange = Some((crate::algebra::arrange_items(&st.kind), st.span));
                    }
                }
                StmtKind::LetSection(l) => {
                    if note_name(self, &l.name, "let", &mut names) {
                        section_lets.push(l);
                    }
                }
                _ => {}
            }
        }
        // a scene a `launch` names plays when a pad is hit, so it is not one that "never plays" (launch.rs)
        let launched = crate::launch::targets(&parsed.program);
        // named arrangements: the flow between them is walked and unrolled into one arrange (flow.rs)
        let mut flow_loops = false;
        let mut loop_item: Option<usize> = None;
        let mut check_items: Option<Vec<crate::algebra::SExpr>> = None;
        let mut flow_report: Option<FlowReport> = None;
        if arrs.is_empty() {
            if let Some(w) = &start_word {
                self.err(Diagnostic::error(w.span, "`start` names an arrangement, and this program has none").help("arrange open: intro build then main"));
            }
        } else if let Some((_, _)) = &arrange {
            let sp = arrs[0].1;
            self.err(Diagnostic::error(sp, "a song has one plain `arrange` or named arrangements, not both").help("name it: arrange main: intro build drop"));
        } else {
            use crate::ast::ArrEnd;
            let names_ix: BTreeMap<&str, usize> = arrs.iter().enumerate().map(|(i, (d, _))| (d.name.text.as_str(), i)).collect();
            let mut nodes: Vec<crate::flow::FlowNode> = Vec::new();
            for (d, _) in &arrs {
                let then = if let ArrEnd::Then(w) = &d.ends {
                    let t = names_ix.get(w.text.as_str()).copied();
                    if t.is_none() {
                        let all: Vec<&str> = names_ix.keys().copied().collect();
                        self.err(Diagnostic::error(w.span, format!("there is no arrangement `{}`", w.text)).help(format!("the arrangements are: {}", all.join(", "))));
                    }
                    t
                } else {
                    None
                };
                nodes.push(crate::flow::FlowNode { name: d.name.text.clone(), ends: d.ends.clone(), then });
            }
            let start = match &start_word {
                Some(w) => match names_ix.get(w.text.as_str()) {
                    Some(i) => *i,
                    None => {
                        self.err(Diagnostic::error(w.span, format!("there is no arrangement `{}` to start with", w.text)));
                        0
                    }
                },
                None => 0,
            };
            let f = crate::flow::walk(&nodes, start);
            if f.too_long {
                self.err(Diagnostic::error(arrs[0].1, format!("the arrangements unroll to more than {} plays", crate::flow::MAX_VISITS)).help("use a smaller `loop N`"));
            }
            for &u in f.unreachable.iter().filter(|u| !launched.contains(&arrs[**u].0.name.text)) {
                self.diags.push(Diagnostic::warning(arrs[u].1, format!("arrangement `{}` is not reachable from the start: it never plays", arrs[u].0.name.text)));
            }
            let mut items: Vec<crate::algebra::SExpr> = Vec::new();
            let mut firsts: Vec<usize> = Vec::new();
            for &v in &f.visits {
                firsts.push(items.len());
                items.extend(arrs[v].0.items.iter().cloned());
            }
            loop_item = f.loop_from.map(|v| firsts[v]);
            flow_loops = f.loop_from.is_some();
            check_items = Some(arrs.iter().flat_map(|(d, _)| d.items.iter().cloned()).collect());
            let ends_text = |e: &ArrEnd| match e {
                ArrEnd::Fall => String::new(),
                ArrEnd::Then(w) => format!("then {}", w.text),
                ArrEnd::Loop => "loop".to_string(),
                ArrEnd::Times(n, _) => format!("loop {n}"),
                ArrEnd::End => "end".to_string(),
            };
            flow_report = Some(FlowReport {
                arrangements: arrs.iter().map(|(d, _)| (d.name.text.clone(), ends_text(&d.ends))).collect(),
                order: f.visits.iter().map(|&v| arrs[v].0.name.text.clone()).collect(),
                loop_from: f.loop_from,
                unreachable: f.unreachable.iter().map(|&u| arrs[u].0.name.text.clone()).filter(|n| !launched.contains(n)).collect(),
                cycle: f.loop_from.is_some() && !f.visits.iter().any(|&v| matches!(arrs[v].0.ends, ArrEnd::Loop)),
            });
            if !f.too_long && !items.is_empty() {
                arrange = Some((items, arrs[0].1));
            }
        }
        // the launches: each names a section or an arrangement, and each pad one scene
        let compositions: BTreeSet<String> = section_lets.iter().map(|l| l.name.text.clone()).collect();
        let launches = self.launches(&parsed.program, &sections, &arrs, &compositions);
        let mut timeline: Vec<(String, u32, u32)> = Vec::new();
        let mut top_unbounded = false;
        // a top-level evolve is the program's own timeline: generation 0 and as many more as it says
        let top_evolves: Vec<(&EvolveBlock, Span)> = parsed.program.stmts.iter().filter_map(|s| if let StmtKind::Evolve(b) = &s.kind { Some((b, s.span)) } else { None }).collect();
        if !top_evolves.is_empty() {
            if arrange.is_some() || !sections.is_empty() {
                self.err(Diagnostic::error(top_evolves[0].1, "in a song, an `evolve` belongs inside the section it varies").help("section drop 24 bars { evolve every 8 bars { ... } }"));
            } else if top_evolves.len() > 1 {
                self.err(Diagnostic::error(top_evolves[1].1, "a program has one top-level `evolve`").help("put each in its own `section`"));
            } else {
                let (b, sp) = top_evolves[0];
                let _ = sp;
                // with no `stop` the timeline is the evolution's own orbit, which is only known once the parts are
                // compiled: one generation's worth for now
                let total = match b.last {
                    Some((last, _)) => (last + 1).saturating_mul(b.every),
                    None => {
                        top_unbounded = true;
                        b.every
                    }
                };
                timeline.push(("evolve".to_string(), 0, total));
                self.places.entry("evolve".to_string()).or_default().push((0, total));
                self.arranged = true;
            }
        }
        let mut plan = crate::algebra::Plan::default();
        // F10: `let NAME = stack(...)` compositions, each using sections and the lets above it
        let (lets, bad_lets) = self.section_lets(&section_lets, &sections);
        if top_evolves.is_empty() && (arrange.is_some() || !sections.is_empty()) {
            // the arrangement is an expression over sections (a plain `arrange a b c` is `then(a, b, c)`); it is laid
            // out as placements on a timeline, cut into stretches where the stacked sections start and end
            let (items, asp): (Vec<crate::algebra::SExpr>, Span) = match &arrange {
                Some((items, sp)) => {
                    for it in check_items.as_ref().unwrap_or(items) {
                        self.check_section_names(it, &sections, &lets, &BTreeSet::new(), &bad_lets);
                    }
                    (items.clone(), *sp)
                }
                None => (section_order.iter().map(|n| crate::algebra::SExpr::Name(sections[n].block.name.clone())).collect(), Span::default()),
            };
            plan = self.plan_arrangement(&items, asp, &sections, &lets, arrange.is_some());
            for k in 0..plan.stretches.len() {
                let st = &plan.stretches[k];
                timeline.push((plan.stretch_name(k), st.start, st.bars));
            }
            if arrange.is_some() {
                let used: BTreeSet<&str> = plan.placements.iter().map(|p| p.name.as_str()).collect();
                for n in &section_order {
                    if !used.contains(n.as_str()) && !launched.contains(n) {
                        self.diags.push(Diagnostic::warning(sections[n].span, format!("section `{n}` is not in the `arrange`: it never plays")));
                    }
                }
            }
            self.arranged = !timeline.is_empty();
        }
        let loop_from_bar = loop_item.and_then(|i| plan.item_starts.get(i).copied()).unwrap_or(0);
        // ---- the end
        let song_bars: u32 = timeline.last().map_or(0, |(_, s, b)| s + b);
        let (mut end, mut looped): (Option<u32>, bool) = (None, false);
        let mut end_at: Option<Span> = None;
        for st in &parsed.program.stmts {
            match &st.kind {
                StmtKind::Loop => {
                    if looped {
                        self.err(Diagnostic::error(st.span, "`loop` is said once"));
                    }
                    if !arrs.is_empty() {
                        self.err(Diagnostic::error(st.span, "named arrangements say how they end: `loop` goes after the arrangement's sections").help("arrange set: drop wubs loop"));
                    }
                    looped = true;
                    // a written `arrange` whose sections failed (`section a`: `a` reads as a note) has an error of its
                    // own: saying the program has no `arrange` would be a second, wrong diagnostic
                    if !self.arranged && arrange.is_none() && sections.is_empty() {
                        self.diags.push(Diagnostic::warning(st.span, "a program with no `arrange` plays on by itself: `loop` does nothing here").help("`end at bar N` stops it"));
                    }
                }
                StmtKind::End(w) => {
                    if end.is_some() {
                        self.err(Diagnostic::error(st.span, "a program ends once").help("keep one `end`"));
                        continue;
                    }
                    end_at = Some(st.span);
                    end = match w {
                        EndWhen::Song if self.arranged => Some(song_bars),
                        EndWhen::Song => {
                            self.err(Diagnostic::error(st.span, "`end` alone ends the song after its last section, and this program has no sections").help("end at bar 64"));
                            None
                        }
                        EndWhen::At(n) => Some(*n),
                        EndWhen::After(name) => self.place(name).map(|(s, b)| s + b),
                    };
                }
                _ => {}
            }
        }
        looped |= flow_loops;
        if let (true, Some(sp)) = (looped && end.is_some(), end_at) {
            self.err(Diagnostic::error(sp, "a song either ends or loops").help("take out the `end` or the `loop`"));
        }
        // a song with a timeline ends after its last section unless it is told to loop (or to end elsewhere)
        let mut end_default = false;
        if self.arranged && end.is_none() && !looped {
            end = Some(song_bars);
            end_default = true;
        }
        // ---- the lines
        let mut out = Lowered { arranged: self.arranged, end, looped, loop_from: if looped { loop_from_bar } else { 0 }, flow: flow_report, top_unbounded, end_default, placements: crate::linked::placements(&plan, &sections), launches, ..Lowered::default() };
        for (k, (n, start, bars)) in timeline.iter().enumerate() {
            out.segments.push(if k < plan.stretches.len() {
                plan.segment(k, &sections)
            } else {
                SegmentLines { name: n.clone(), start: *start, bars: *bars, lines: Vec::new(), header: sections.get(n).map(|s| s.span), key: None, chain: false, header_more: Vec::new(), phase: 0 }
            });
        }
        let top = Scope { base: None, depth: 0, path: String::new(), segment: None, via: Vec::new(), timed: false };
        let mut globals = Vec::new();
        for st in &parsed.program.stmts {
            match &st.kind {
                StmtKind::Section(_) | StmtKind::Arrange(_) | StmtKind::ArrangeExpr(_) | StmtKind::ArrangeNamed(_) | StmtKind::Start(_) | StmtKind::Launch(_) | StmtKind::LetSection(_) | StmtKind::Let(_) | StmtKind::Def(_) | StmtKind::Import(_) | StmtKind::Setting(_) | StmtKind::End(_) | StmtKind::Loop | StmtKind::Sample(_) | StmtKind::UseRestricted | StmtKind::Wire(_) | StmtKind::Assert(_) | StmtKind::Set(_) | StmtKind::Seed(_) => {}
                StmtKind::Evolve(b) => {
                    if self.arranged && !timeline.is_empty() && timeline[0].0 == "evolve" {
                        self.evolves.push((0, b.clone()));
                    }
                }
                _ => self.stmt(st, &top, &mut globals, &mut out.segments),
            }
        }
        // F10: each placed section's lines, in the stretches it plays over (algebra.rs)
        self.layer_lines(&plan, &sections, &mut out);
        out.globals = globals;
        out.evolves = std::mem::take(&mut self.evolves);
        self.port_warnings();
        out
    }

    // ---- recursion

    /// Calls between defs that come back round to where they began.
    pub(crate) fn call_cycles(&mut self) {
        let mut edges: BTreeMap<String, Vec<(String, Span)>> = BTreeMap::new();
        for (name, info) in &self.defs {
            let b = &info.def.body;
            for (i, t) in b.iter().enumerate() {
                if let (Tok::Word(w), Some(Token { tok: Tok::LParen, .. })) = (&t.tok, b.get(i + 1)) {
                    if self.defs.contains_key(w) {
                        edges.entry(name.clone()).or_default().push((w.clone(), t.span));
                    }
                }
            }
        }
        // a depth-first walk from every def; a path that meets itself is a cycle
        let names: Vec<String> = self.defs.keys().cloned().collect();
        let mut reported: BTreeSet<String> = BTreeSet::new();
        for n in &names {
            let mut path: Vec<(String, Span)> = vec![(n.clone(), self.defs[n].def.name.span)];
            self.dfs(&edges, &mut path, &mut reported);
        }
    }

    pub(crate) fn dfs(&mut self, edges: &BTreeMap<String, Vec<(String, Span)>>, path: &mut Vec<(String, Span)>, reported: &mut BTreeSet<String>) {
        let here = path.last().unwrap().0.clone();
        for (to, at) in edges.get(&here).cloned().unwrap_or_default() {
            if let Some(i) = path.iter().position(|(n, _)| *n == to) {
                let mut cyc: Vec<String> = path[i..].iter().map(|(n, _)| n.clone()).collect();
                cyc.push(to.clone());
                let mut key = cyc[..cyc.len() - 1].to_vec();
                key.sort();
                if reported.insert(key.join(",")) {
                    let shown = cyc.join(" -> ");
                    self.err(
                        Diagnostic::error(at, format!("`{to}` calls itself: {shown}"))
                            .help("a def cannot call itself, directly or through another def (it would never end); use `repeat N { }` with the count written out"),
                    );
                    for n in &cyc {
                        self.cyclic.insert(n.clone());
                    }
                }
                continue;
            }
            if path.len() > 64 {
                continue;
            }
            path.push((to, at));
            self.dfs(edges, path, reported);
            path.pop();
        }
    }

    // ---- patterns by name

    /// `seq` with every name of a `let` pattern in it put in its place.
    pub(crate) fn put_lets(&self, seq: &Seq) -> Seq {
        Seq { span: seq.span, terms: seq.terms.iter().map(|t| self.put_term(t)).collect() }
    }

    pub(crate) fn put_term(&self, t: &Term) -> Term {
        let atom = match &t.atom {
            Atom::Note(w) if self.pats.contains_key(&w.text) => Atom::Group(vec![self.pats[&w.text].clone()]),
            Atom::Group(seqs) => Atom::Group(seqs.iter().map(|s| self.put_lets(s)).collect()),
            Atom::Alt(terms) => Atom::Alt(terms.iter().map(|x| self.put_term(x)).collect()),
            Atom::Call(c) => Atom::Call(self.put_call(c)),
            Atom::Chance(c) => Atom::Chance(Box::new(crate::chance::ChanceTerm { term: self.put_term(&c.term), prob: c.prob })),
            Atom::Sized(s) => Atom::Sized(Box::new(crate::sugar::Sized { term: self.put_term(&s.term), ..(**s).clone() })),
            Atom::Choose(c) => Atom::Choose(Box::new(crate::chance::Choose { options: c.options.iter().map(|x| self.put_term(x)).collect(), ..(**c).clone() })),
            other => other.clone(),
        };
        Term { span: t.span, atom, fast: t.fast, accent: t.accent, slide: t.slide, marks: t.marks }
    }

    pub(crate) fn put_call(&self, c: &Call) -> Call {
        let args = c
            .args
            .iter()
            .map(|a| self.put_arg(a))
            .collect();
        Call { name: c.name.clone(), args, span: c.span }
    }

    pub(crate) fn put_arg(&self, a: &Arg) -> Arg {
        match a {
            Arg::Word(w) if self.pats.contains_key(&w.text) => Arg::Pat(self.pats[&w.text].clone()),
            Arg::Pat(s) => Arg::Pat(self.put_lets(s)),
            Arg::Call(c) => Arg::Call(Box::new(self.put_call(c))),
            Arg::Named(n, v) => Arg::Named(n.clone(), Box::new(self.put_arg(v))),
            other => other.clone(),
        }
    }

    // ---- how big a pattern is once its lets are put in place, worked out from the text

    pub(crate) fn dims_seq(&self, s: &Seq) -> (usize, usize) {
        s.terms.iter().fold((0, 0), |(n, d), t| {
            let (a, b) = self.dims_term(t);
            (n.saturating_add(a), d.max(b))
        })
    }

    fn dims_terms<'t>(&self, ts: impl Iterator<Item = &'t Term>) -> (usize, usize) {
        let (n, d) = ts.fold((0usize, 0usize), |(n, d), t| {
            let (a, b) = self.dims_term(t);
            (n.saturating_add(a), d.max(b))
        });
        (n.saturating_add(1), d.saturating_add(1))
    }

    fn dims_term(&self, t: &Term) -> (usize, usize) {
        match &t.atom {
            Atom::Note(w) if self.dims.contains_key(&w.text) => {
                let (n, d) = self.dims[&w.text];
                (n.saturating_add(1), d.saturating_add(1))
            }
            Atom::Group(seqs) | Atom::Deg(seqs) => {
                let (n, d) = seqs.iter().fold((0usize, 0usize), |(n, d), s| {
                    let (a, b) = self.dims_seq(s);
                    (n.saturating_add(a), d.max(b))
                });
                (n.saturating_add(1), d.saturating_add(1))
            }
            Atom::Alt(terms) => self.dims_terms(terms.iter()),
            Atom::Call(c) => self.dims_call(c),
            Atom::Chance(c) => self.dims_terms(std::iter::once(&c.term)),
            // (what `c2~3` writes out is bounded where it is written out: sugar::MAX_TERMS)
            Atom::Sized(s) => self.dims_terms(std::iter::once(&s.term)),
            Atom::Choose(c) => self.dims_terms(c.options.iter()),
            _ => (1, 1),
        }
    }

    fn dims_call(&self, c: &Call) -> (usize, usize) {
        let (n, d) = c.args.iter().fold((0usize, 0usize), |(n, d), a| {
            let (x, y) = self.dims_arg(a);
            (n.saturating_add(x), d.max(y))
        });
        (n.saturating_add(1), d.saturating_add(1))
    }

    fn dims_arg(&self, a: &Arg) -> (usize, usize) {
        match a {
            Arg::Word(w) if self.dims.contains_key(&w.text) => self.dims[&w.text],
            Arg::Pat(s) => self.dims_seq(s),
            Arg::Call(c) => self.dims_call(c),
            Arg::Named(_, v) => self.dims_arg(v),
            _ => (1, 0),
        }
    }

    fn pattern_too_big(&self, at: Span, what: &str, nodes: usize, depth: usize) -> Diagnostic {
        let (size, limit) = if nodes > MAX_PATTERN_NODES { (format!("{nodes} terms"), format!("{MAX_PATTERN_NODES} terms")) } else { (format!("{depth} levels deep"), format!("{MAX_PATTERN_DEPTH} levels")) };
        Diagnostic::error(at, format!("{what} is {size} once the `let`s in it are put in place (at most {limit})"))
            .help("a `let` that names the one before it twice doubles it each time: use fewer, or a `repeat` (`x*4`) for the doubling")
    }

    pub(crate) fn put_parts(&self, parts: &[Part]) -> Vec<Part> {
        parts.iter().map(|p| Part { pattern: self.put_lets(&p.pattern), ..p.clone() }).collect()
    }

    // ---- statements

    pub(crate) fn budget(&mut self, at: Span) -> bool {
        if self.over_budget {
            return false;
        }
        self.produced += 1;
        if self.produced > MAX_LINES {
            if !self.over_budget {
                self.over_budget = true;
                self.err(Diagnostic::error(at, format!("this program expands to more than {MAX_LINES} lines")).help("fewer or smaller `repeat`s and calls"));
            }
            return false;
        }
        true
    }

    /// Charge one copy of a repeat, or one call, whose text is `tokens` long. False (after saying so once) when the
    /// program has asked for more than [`MAX_WORK`], or when an error already stopped the expansion.
    pub(crate) fn charge(&mut self, at: Span, tokens: usize) -> bool {
        if self.over_budget {
            return false;
        }
        self.work = self.work.saturating_add(tokens + COPY_COST);
        if self.work > MAX_WORK {
            self.over_budget = true;
            self.err(Diagnostic::error(at, format!("this program's repeats and calls write out more than {MAX_WORK} tokens of text")).help("fewer or smaller `repeat`s and calls, or nest them less: each copy of a repeat and each call counts, even one that plays nothing"));
            return false;
        }
        true
    }

    /// A call or a repeat nested too deep: said once, and the expansion stops (carrying on would only meet the same
    /// error in every copy of every loop around it, which is the cost of the nesting).
    fn too_deep(&mut self, at: Span) {
        if !self.over_budget {
            self.err(Diagnostic::error(at, format!("calls and repeats nest more than {MAX_DEPTH} deep")));
        }
        self.over_budget = true;
    }

    pub(crate) fn stmt(&mut self, st: &Stmt, sc: &Scope, out: &mut Vec<FlatLine>, segs: &mut Vec<SegmentLines>) {
        match &st.kind {
            StmtKind::Line(l) => self.line(st.span, l, sc, out, segs),
            StmtKind::Repeat(r) => self.repeat(st.span, r, sc, out, segs),
            StmtKind::Setting(Setting::Key(_)) => self.err(Diagnostic::error(st.span, "a `key` belongs at the top of the program, or at the head of a `section` (before its other lines), not in a def or a repeat")),
            // a tempo in a section, a def or a repeat is a tempo from where that body starts (F3, tempo.rs)
            StmtKind::Setting(Setting::Tempo(_)) | StmtKind::Setting(Setting::TempoRamp(_)) => self.tempo_stmt(st, sc, out, segs),
            StmtKind::Setting(_) => self.err(Diagnostic::error(st.span, "`swing` belongs at the top of the program, outside blocks (`tempo` and `key` may also stand in a section)")),
            StmtKind::Let(l) => self.err(Diagnostic::error(l.name.span, "a `let` belongs at the top of the program, outside blocks")),
            StmtKind::Def(d) => self.err(Diagnostic::error(d.name.span, "a `def` belongs at the top of the program, outside blocks")),
            StmtKind::Section(s) => self.err(Diagnostic::error(s.name.span, "a `section` belongs at the top of the program, not inside another block")),
            StmtKind::Arrange(_) | StmtKind::ArrangeExpr(_) | StmtKind::ArrangeNamed(_) | StmtKind::Start(_) | StmtKind::ArrangeBlock(_) => self.err(Diagnostic::error(st.span, "`arrange` belongs at the top of the program")),
            StmtKind::Launch(_) => self.err(Diagnostic::error(st.span, "a `launch` belongs at the top of the program, not inside another block")),
            StmtKind::LetSection(l) => self.err(Diagnostic::error(l.name.span, "a `let` belongs at the top of the program, outside blocks")),
            StmtKind::Import(_) => self.err(Diagnostic::error(st.span, "`import` belongs at the top of the program")),
            StmtKind::Set(_) => self.err(Diagnostic::error(st.span, "a `set` is a whole program: it belongs at the top, with nothing else but `end` and `loop`")),
            StmtKind::Sample(_) | StmtKind::UseRestricted => self.err(Diagnostic::error(st.span, "`sample`, `loop \"...\" as` and `use restricted` belong at the top of the program")),
            StmtKind::End(_) | StmtKind::Loop => self.err(Diagnostic::error(st.span, "`end` and `loop` belong at the top of the program")),
            StmtKind::Wire(_) => self.err(Diagnostic::error(st.span, "a `wire` belongs at the top of the program, outside blocks")),
            StmtKind::Seed(_) => self.err(Diagnostic::error(st.span, "`section ... from seed(...)` and `arrange from seed(...)` belong at the top of the program")),
            StmtKind::Assert(_) => {
                let d = Diagnostic::error(st.span, "an `assert` belongs at the top of the program, outside blocks").help("say which section it is about with `in`: `assert level A < B in drop`");
                // said once, not once for every copy of a repeat
                if !self.diags.iter().any(|x| x.span == d.span && x.message == d.message) {
                    self.err(d);
                }
            }
            StmtKind::Evolve(b) => match sc.segment {
                Some(k) if sc.depth == 0 => {
                    if self.evolves.iter().any(|(i, _)| *i == k) {
                        self.err(Diagnostic::error(st.span, "a section has one `evolve`").help("one rule block: list every part in it"));
                    } else {
                        self.evolves.push((k, b.clone()));
                    }
                }
                _ => self.err(Diagnostic::error(st.span, "an `evolve` belongs directly in a `section` (or at the top of a program with no sections)")),
            },
        }
    }

    pub(crate) fn line(&mut self, span: Span, l: &Line, sc: &Scope, out: &mut Vec<FlatLine>, segs: &mut Vec<SegmentLines>) {
        // a `when` guard is decided first: a line it drops is not checked, timed or compiled (so it may name
        // things that do not exist in this repetition); one it keeps is the line without it
        let stripped;
        let l = match &l.guard {
            // a guard that names an input (`when held(key)`) is decided while the program plays: the line keeps it (perform.rs)
            Some(g) if crate::perform::mentions_input(&g.expr) => {
                if let Some(d) = crate::perform::misplaced_guard(l) {
                    if !self.diags.iter().any(|x| x.span == d.span && x.message == d.message) {
                        self.err(d);
                    }
                    return;
                }
                l
            }
            Some(g) => {
                match crate::expr::eval_bool(&g.expr, &crate::expr::NoHooks) {
                    Ok(true) => {}
                    Ok(false) => return,
                    Err(d) => {
                        // said once, not once for every copy of a repeat
                        if !self.diags.iter().any(|x| x.span == d.span && x.message == d.message) {
                            self.err(d);
                        }
                        return;
                    }
                }
                stripped = Line { guard: None, ..l.clone() };
                &stripped
            }
            None => l,
        };
        if let Body::Invoke(inv) = &l.body {
            self.invoke(span, l, inv, sc, out, segs);
            return;
        }
        if !self.budget(span) {
            return;
        }
        let Some(timing) = self.timing(l, sc, span) else { return };
        let mut line = l.clone();
        if let Body::Parts(ps) = &l.body {
            // a pattern too big once its lets are put in place is refused before it is built
            for p in ps {
                let (nodes, depth) = self.dims_seq(&p.pattern);
                if nodes > MAX_PATTERN_NODES || depth > MAX_PATTERN_DEPTH {
                    let d = self.pattern_too_big(p.pattern.span, &format!("this `{}` pattern", p.role.text), nodes, depth);
                    if !self.diags.iter().any(|x| x.span == d.span && x.message == d.message) {
                        self.err(d);
                    }
                    return;
                }
            }
            line.body = Body::Parts(self.put_parts(ps));
        }
        let untimed = !sc.timed && says_no_time(l) && l.hold.is_none();
        out.push(FlatLine { span, line, timing, hold: l.hold.map(|(d, _)| d), path: sc.path.clone(), via: sc.via.clone(), phase: None, untimed });
    }

    pub(crate) fn place(&mut self, name: &Word) -> Option<(u32, u32)> {
        match self.places.get(&name.text).map(Vec::as_slice) {
            Some([one]) => Some(*one),
            Some(many) if many.len() > 1 => {
                self.err(Diagnostic::error(name.span, format!("section `{}` plays {} times on the timeline: say which with `at bar N`", name.text, many.len())));
                None
            }
            _ if self.declined.contains(&name.text) => None,
            _ => {
                // a scene plays on its own: a line that counts from a section that is not in it has nothing to count from, and does not fire there
                if let Some(scene) = crate::launch::current_scene() {
                    self.diags.push(Diagnostic::warning(name.span, format!("`{}` is not in the scene `{scene}`: a line that counts from it does not fire there", name.text)));
                    return None;
                }
                let all: Vec<String> = self.places.keys().cloned().collect();
                let mut d = Diagnostic::error(name.span, format!("there is no section `{}` on the timeline", name.text)).found(format!("`{}`", name.text));
                if let Some(n) = crate::env::suggest(&name.text, &all, 1).first() {
                    d = d.help(format!("did you mean `{n}`?"));
                }
                if self.places.is_empty() {
                    d = d.help("`after` and `before` need `section` blocks and an `arrange`");
                }
                self.err(d);
                None
            }
        }
    }

    /// `a + b` bars, or an error at `at` when the sum is more than a bar number can hold (a `repeat 3 every 4294967295
    /// bars`, an `at bar 4294967295` inside a `repeat`): never a wrap or a panic.
    pub(crate) fn bars_sum(&mut self, a: u32, b: u32, at: Span) -> Option<u32> {
        let sum = a.checked_add(b);
        if sum.is_none() {
            self.err(Diagnostic::error(at, format!("bar {a} + {b} is past the last bar number ({})", u32::MAX)).help("a song is at most 16384 bars: a bar that far out is never reached"));
        }
        sum
    }

    /// When a line fires, as the bar it means (relative to the song when it has a timeline).
    pub(crate) fn timing(&mut self, l: &Line, sc: &Scope, span: Span) -> Option<Timing> {
        let base = sc.base;
        let arranged = self.arranged;
        let plain = |t: Option<u32>| -> Timing {
            match t {
                Some(b) if b > 0 || arranged => Timing::At(b),
                _ => Timing::Now,
            }
        };
        Some(match &l.when {
            None | Some((When::Now, _)) | Some((When::NextBar, _)) => plain(base),
            Some((When::Every(n), sp)) => match base {
                Some(b) => Timing::EveryFrom(*n, self.bars_sum(b, *n, *sp)?),
                None => Timing::Every(*n),
            },
            Some((When::Phrase, sp)) => match base {
                Some(b) => Timing::EveryFrom(8, self.bars_sum(b, 8, *sp)?),
                None => Timing::Every(8),
            },
            Some((When::EveryFrom(n, f), sp)) => Timing::EveryFrom(*n, self.bars_sum(base.unwrap_or(0), *f, *sp)?),
            Some((When::At(n), sp)) => {
                let bar = self.bars_sum(base.unwrap_or(0), *n, *sp)?;
                if *n == 0 && base.is_none() && !arranged {
                    self.err(Diagnostic::error(*sp, "bars are counted from 1: the program's first bar is bar 0 and nothing is `at` it").help("at bar 1 is the first bar after the program starts"));
                    return None;
                }
                if bar == 0 && !arranged {
                    Timing::Now
                } else {
                    Timing::At(bar)
                }
            }
            Some((When::NextPhrase(n), _)) => Timing::NextMult(*n),
            Some((When::AtPos(p), sp)) => {
                let abs = Rat::int(i64::from(base.unwrap_or(0))).add(p.as_bars()).ok()?;
                // a time inside bar 0 (`at bar 1/2`, `at bar 0 beat 2`) is legal: beats are counted from 0 like bars, and only the
                // whole bar 0 (the bar line the program starts on, which is nothing to be `at`) is refused, as `at bar 0` is (L-07)
                if base.is_none() && !arranged && !abs.is_neg() && abs.is_zero() {
                    self.err(Diagnostic::error(*sp, "bars are counted from 1: the program's first bar is bar 0 and nothing is `at` it").help("at bar 1 is the first bar after the program starts"));
                    return None;
                }
                Timing::AtPos(abs)
            }
            Some((When::EveryPos(step, from), _)) => {
                let first = from.map_or(step.as_bars(), |p| p.as_bars());
                Timing::EveryPos { step: step.as_bars(), from: Rat::int(i64::from(base.unwrap_or(0))).add(first).ok()? }
            }
            Some((When::BeforeDur(name, d), sp)) => {
                let (start, _) = self.place(name)?;
                let t = Rat::int(i64::from(start)).sub(d.as_bars()).ok()?;
                if t.is_neg() {
                    self.err(Diagnostic::error(*sp, format!("`{}` starts on bar {start}: {d} before it is before the song", name.text)));
                    return None;
                }
                Timing::AtPos(t)
            }
            Some((When::After(name), _)) => {
                let (start, bars) = self.place(name)?;
                Timing::At(start + bars)
            }
            Some((When::Before(name, n), sp)) => {
                let (start, _) = self.place(name)?;
                if *n > start {
                    self.err(Diagnostic::error(*sp, format!("`{}` starts on bar {start}: {n} bars before it is before the song", name.text)));
                    return None;
                }
                Timing::At(start - n)
            }
        })
        .map(|t| {
            let _ = span;
            t
        })
    }

    // ---- calls and repeats

    pub(crate) fn invoke(&mut self, span: Span, l: &Line, inv: &Invocation, sc: &Scope, out: &mut Vec<FlatLine>, segs: &mut Vec<SegmentLines>) {
        let name = &inv.name;
        if self.over_budget {
            return;
        }
        let Some(info) = self.defs.get(&name.text) else {
            if self.declined.contains(&name.text) {
                return;
            }
            let all: Vec<String> = self.defs.keys().cloned().collect();
            let mut d = Diagnostic::error(name.span, format!("`{}` is not a def", name.text)).found(format!("`{}`", name.text));
            if matches!(name.text.as_str(), "stack" | "then") {
                d = d.help("`stack(...)` and `then(...)` of parts go after a deck: `deck A: stack(kick x ~ x ~, snare ~ x ~ x)` (a def of that name is called with no deck before it)");
            } else if let Some(n) = crate::env::suggest(&name.text, &all, 1).first() {
                d = d.help(format!("did you mean `{n}`?"));
            } else if all.is_empty() {
                d = d.help("a call is the name of a `def name(params) { ... }`");
            }
            self.err(d);
            return;
        };
        let def = info.def.clone();
        let imported = info.imported;
        if self.cyclic.contains(&name.text) {
            self.err(Diagnostic::error(name.span, format!("`{}` takes part in a call cycle and is not run", name.text)).help("see the error on the call that closes the cycle"));
            return;
        }
        if sc.depth >= MAX_DEPTH {
            self.too_deep(name.span);
            return;
        }
        if inv.args.len() != def.params.len() {
            self.err(
                Diagnostic::error(inv.span, format!("`{}` takes {} argument{}, and this has {}", name.text, def.params.len(), if def.params.len() == 1 { "" } else { "s" }, inv.args.len()))
                    .help(format!("def {}({})", name.text, def.params.iter().map(|p| p.text.clone()).collect::<Vec<_>>().join(", "))),
            );
            return;
        }
        // what a time word on the call means: where the body starts
        let base = match &l.when {
            None | Some((When::Now, _)) | Some((When::NextBar, _)) => sc.base,
            Some((When::At(n), sp)) => match self.bars_sum(sc.base.unwrap_or(0), *n, *sp) {
                Some(b) => Some(b),
                None => return,
            },
            Some((When::After(w), _)) => match self.place(w) {
                Some((s, b)) => Some(s + b),
                // (in a scene the call is left out, as a line is: its lines have nothing to count from)
                None if crate::launch::current_scene().is_some() => return,
                None => None,
            },
            Some((When::Before(w, n), sp)) => match self.place(w) {
                Some((s, _)) if *n <= s => Some(s - n),
                Some(_) => {
                    self.err(Diagnostic::error(*sp, "that is before the song begins"));
                    return;
                }
                None => return,
            },
            Some((_, sp)) => {
                self.err(Diagnostic::error(*sp, format!("a call of `{}` takes `at bar N:`, `after X:` or `before X N bars:`, or no time word; its own lines carry the rest", name.text)));
                return;
            }
        };
        if l.hold.is_some() {
            self.err(Diagnostic::error(l.hold.map_or(span, |(_, s)| s), "a call takes no `for`: put it on the lines inside"));
        }
        let mut subs: BTreeMap<String, Vec<Token>> = def.params.iter().zip(inv.args.iter()).map(|(p, a)| (p.text.clone(), a.clone())).collect();
        let call_text = print::line(l);
        // a def with ports: its inputs are the values wired to them, and its `emit` lines are not lines of the program
        let Some(body) = self.port_call(&def, span, &mut subs) else { return };
        if !self.charge(name.span, body.len()) {
            return;
        }
        let Some(toks) = self.fill(&body, &subs, &format!("def {}", name.text)) else { return };
        let (stmts, ds) = parse_tokens(toks, self.src);
        let trace = if imported { format!("{} (imported)", call_text) } else { call_text.clone() };
        for mut d in ds {
            d.help = Some(match d.help.take() {
                Some(h) => format!("{h} (in {trace}, line {})", crate::diag::line_col(self.src, span.start).0),
                None => format!("in {trace}, line {}", crate::diag::line_col(self.src, span.start).0),
            });
            self.err(d);
        }
        let mut via = vec![span];
        via.extend(sc.via.iter().copied());
        let sub = Scope { base, depth: sc.depth + 1, path: format!("{}{}: ", sc.path, call_text.trim_end_matches(':')), segment: sc.segment, via, timed: sc.timed || !says_no_time(l) };
        for st in &stmts {
            // the lines of a call that failed to read are not run
            self.stmt(st, &sub, out, segs);
        }
    }

    pub(crate) fn repeat(&mut self, span: Span, r: &RepeatBlock, sc: &Scope, out: &mut Vec<FlatLine>, segs: &mut Vec<SegmentLines>) {
        if self.over_budget {
            return;
        }
        if sc.depth >= MAX_DEPTH {
            self.too_deep(span);
            return;
        }
        let mut said: Vec<(String, Span)> = Vec::new();
        for i in 0..r.n {
            if !self.charge(span, r.body.len()) {
                return;
            }
            let mut subs: BTreeMap<String, Vec<Token>> = BTreeMap::new();
            subs.insert("i".into(), vec![Token { tok: Tok::Num { value: f64::from(i), int: true }, span }]);
            subs.insert("n".into(), vec![Token { tok: Tok::Num { value: f64::from(r.n), int: true }, span }]);
            let Some(toks) = self.fill(&r.body, &subs, "repeat") else { return };
            let (stmts, ds) = parse_tokens(toks, self.src);
            // (an error every copy has is said once, with the first; one that only a later copy has, because its arithmetic
            // came to a number that does not fit, is said too, and not dropped with the line: at most MAX_COPY_ERRORS in all)
            for mut d in ds {
                let seen_it = said.iter().any(|(m, s)| *m == d.message && *s == d.span);
                if seen_it || said.len() >= MAX_COPY_ERRORS {
                    continue;
                }
                said.push((d.message.clone(), d.span));
                d.help = Some(d.help.take().map_or("in a repeat".to_string(), |h| format!("{h} (in a repeat)")));
                if i > 0 {
                    d.help = d.help.take().map(|h| format!("{h}, copy {} of {}", i + 1, r.n));
                }
                self.err(d);
            }
            let base = match (r.every, sc.base) {
                (Some(e), b) if i > 0 || b.is_some() => match e.checked_mul(i) {
                    Some(shift) => match self.bars_sum(b.unwrap_or(0), shift, span) {
                        Some(sum) => Some(sum),
                        None => return,
                    },
                    None => {
                        self.err(Diagnostic::error(span, format!("copy {} of this repeat would start {e} x {i} bars in: past the last bar number ({})", i + 1, u32::MAX)).help("a song is at most 16384 bars"));
                        return;
                    }
                },
                (_, b) => b,
            };
            let mut via = vec![span];
            via.extend(sc.via.iter().copied());
            let sub = Scope { base, depth: sc.depth + 1, path: format!("{}[repeat {}/{}] ", sc.path, i + 1, r.n), segment: sc.segment, via, timed: true };
            for st in &stmts {
                self.stmt(st, &sub, out, segs);
            }
            if self.over_budget {
                return;
            }
        }
    }

    /// `body` with each `$name` put in place; an unknown `$name` is an error.
    pub(crate) fn fill(&mut self, body: &[Token], subs: &BTreeMap<String, Vec<Token>>, owner: &str) -> Option<Vec<Token>> {
        let mut out = Vec::with_capacity(body.len());
        let mut ok = true;
        for (at, t) in body.iter().enumerate() {
            match &t.tok {
                Tok::Dollar(n) => match subs.get(n) {
                    // the argument's tokens stand where the `$name` was: a line's span stays inside the def's own
                    // text, and the call site is in `via`
                    Some(v) => {
                        // an argument that is arithmetic keeps its grouping: `$x * 2` with `x = 1 + 2` is `(1 + 2) * 2`;
                        // how it is fenced depends on the slot it lands in
                        let wrap = match slot_of(body, at) {
                            Slot::Glued => crate::expr::needs_parens_glued(v),
                            // (a step of a pattern keeps what `1*4` and `1*2 3` always meant: the degree 1 four times)
                            Slot::Step => crate::expr::needs_parens(v) && !crate::expr::pattern_like(v),
                            Slot::Operand | Slot::Free => crate::expr::needs_parens(v),
                        };
                        if wrap {
                            out.push(Token { tok: Tok::LParen, span: t.span });
                        }
                        // (tokens that touched in the call keep touching: `put_at`)
                        out.extend(crate::lex::put_at(v, t.span));
                        if wrap {
                            out.push(Token { tok: Tok::RParen, span: t.span });
                        }
                    }
                    // `$i` and `$n` of a `repeat` inside a def's body are the repeat's: left for it to put in place
                    None if (n == "i" || n == "n") && owner.starts_with("def ") && in_repeat_block(body, at) => out.push(t.clone()),
                    None => {
                        // (`$i` and `$n` are not suggested: they are a repeat's, whatever parameter has a close name)
                        let all: Vec<String> = subs.keys().filter(|k| k.as_str() != "i" && k.as_str() != "n").cloned().collect();
                        let mut d = Diagnostic::error(t.span, format!("`${n}` is not a parameter of {owner}")).found(format!("`${n}`"));
                        if n == "i" || n == "n" {
                            d = d.help(format!("`$i` and `$n` are the index and the count of a `repeat`, and exist inside its braces{}", if owner.starts_with("def ") { " (a `repeat` in the def's body)" } else { "" }));
                        } else if let Some(s) = crate::env::suggest(n, &all, 1).first() {
                            d = d.help(format!("did you mean `${s}`?"));
                        } else if all.is_empty() {
                            d = d.help(format!("{owner} has no parameters"));
                        }
                        self.err(d);
                        ok = false;
                    }
                },
                _ => out.push(t.clone()),
            }
        }
        ok.then_some(out)
    }
}

/// Where a `$name` stands in a body, as far as how a value put there must be fenced goes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Slot {
    /// A slot written glued to what is before it in a part: `x*$k`, `x?$p`, `hihat/$n`, `weights $w 1 1`.
    Glued,
    /// Beside an operator of an expression: `$a * 2`, `1 - $b`.
    Operand,
    /// A step of a part's pattern: `bass $p ~ 5`.
    Step,
    /// Anywhere else: `at bar $b:`, `tempo $t`, `level=$v`.
    Free,
}

fn is_arith_op(t: &Tok) -> bool {
    matches!(t, Tok::Plus | Tok::Minus | Tok::Star | Tok::Slash | Tok::Percent)
}

/// The slot the `$name` at `body[at]` stands in, from the tokens round it on its own line.
fn slot_of(body: &[Token], at: usize) -> Slot {
    let line = body[..at].iter().rposition(|t| t.tok == Tok::Newline).map_or(0, |p| p + 1);
    let before = &body[line..at];
    let in_part = before.iter().any(|t| matches!(&t.tok, Tok::Word(w) if parse::ROLES.contains(&w.as_str())));
    let prev = before.last().map(|t| &t.tok);
    let next = body.get(at + 1).map(|t| &t.tok);
    // `weights 1 $w 1`: back over the weights already written (numbers, names, parentheses) to the word
    let after_weights = before.iter().rev().take_while(|t| matches!(t.tok, Tok::Dollar(_) | Tok::Num { .. } | Tok::LParen | Tok::RParen) || is_arith_op(&t.tok)).count();
    let weights = before.len() >= after_weights && before.len() > after_weights && matches!(&before[before.len() - after_weights - 1].tok, Tok::Word(w) if w == "weights");
    if in_part && (matches!(prev, Some(Tok::Star | Tok::Slash | Tok::Question)) || weights) {
        return Slot::Glued;
    }
    if prev.is_some_and(is_arith_op) || next.is_some_and(is_arith_op) {
        return Slot::Operand;
    }
    if in_part {
        return Slot::Step;
    }
    Slot::Free
}

/// Whether `body[at]` is inside the braces of a `repeat` block of the body.
fn in_repeat_block(body: &[Token], at: usize) -> bool {
    let mut open: Vec<bool> = Vec::new();
    let mut pending = false;
    let mut line_start = true;
    for t in &body[..at] {
        match &t.tok {
            Tok::Newline => {
                line_start = true;
                continue;
            }
            Tok::Word(w) if w == "repeat" && line_start => pending = true,
            Tok::LBrace => {
                open.push(pending);
                pending = false;
            }
            Tok::RBrace => {
                open.pop();
            }
            _ => {}
        }
        line_start = matches!(t.tok, Tok::LBrace | Tok::RBrace);
    }
    open.contains(&true)
}

/// `parse::parse`, then the expansion: what the compiler reads.
pub fn parsed_and_lowered(parsed: &Parsed, src: &str, diags: &mut Vec<Diagnostic>) -> Lowered {
    let _ = parse::SUGAR;
    lower(parsed, src, diags)
}
