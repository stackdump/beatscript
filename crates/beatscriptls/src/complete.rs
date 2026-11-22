//! Completion at the cursor.
//!
//! The statement around the cursor is read up to it (`djlive::partial::statement_at`: the partial AST, and what the parser
//! would take at the cursor). Candidates come from the place:
//!
//! - the tokens before the cursor (after `deck A:` a role or a control word; after a role, pattern terms and `vel=` ...; after
//!   `inst=` a voice; after `arrange` a section; after `do noun` a verb and its params; in a `gen@2` call its genres, its
//!   names and their values);
//! - the parser's `expected` at the cursor, mapped from its prose (`` `bars` ``, "a section name", "a key like am") to
//!   typed candidates;
//! - the environment (voices, kits, palettes, the registry's nouns, verbs, params and their options);
//! - the program's own names (sections, lets, defs, parameters and ports, samples).
//!
//! Each candidate is ranked by how well it fits the place (`sortText`), then by whether it starts with what is typed.

use std::collections::BTreeSet;

use djengine::json::{JsonObject, JsonValue};
use djlive::env::{NounSpec, Registry, Voices};
use djlive::lex::Tok;
use djlive::partial::{statement_at, Partial};

use crate::analysis::Analysis;
use crate::json::{arr, obj, s, span_range};
use crate::lang;
use crate::symbols::Kind;
use crate::text::Encoding;

/// LSP `CompletionItemKind`s used here.
pub mod kind {
    pub const FUNCTION: i64 = 3;
    pub const FIELD: i64 = 5;
    pub const VARIABLE: i64 = 6;
    pub const MODULE: i64 = 9;
    pub const PROPERTY: i64 = 10;
    pub const VALUE: i64 = 12;
    pub const KEYWORD: i64 = 14;
    pub const SNIPPET: i64 = 15;
    pub const FILE: i64 = 17;
    pub const ENUM_MEMBER: i64 = 20;
    pub const CONSTANT: i64 = 21;
    pub const STRUCT: i64 = 22;
    pub const EVENT: i64 = 23;
    pub const OPERATOR: i64 = 24;
    pub const TYPE_PARAMETER: i64 = 25;
}

/// One candidate.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub label: String,
    pub kind: i64,
    pub detail: String,
    pub doc: String,
    /// A snippet (`${1:...}`) inserted instead of the label.
    pub snippet: Option<String>,
    /// Lower is a better fit for the place.
    pub rank: u8,
}

impl Item {
    fn new(label: impl Into<String>, kind: i64, rank: u8) -> Item {
        Item { label: label.into(), kind, detail: String::new(), doc: String::new(), snippet: None, rank }
    }
    fn detail(mut self, d: impl Into<String>) -> Item {
        self.detail = d.into();
        self
    }
    fn doc(mut self, d: impl Into<String>) -> Item {
        self.doc = d.into();
        self
    }
    fn snippet(mut self, s: impl Into<String>) -> Item {
        self.snippet = Some(s.into());
        self
    }
}

/// What completion knows of the environment: the registry (for `do` lines; its param docs when the host has them), the
/// catalog of sounds, the singing voices.
pub struct EnvNames<'a> {
    pub registry: &'a dyn Registry,
    pub voices: Vec<String>,
    pub families: Vec<String>,
    pub kits: Vec<String>,
    pub palettes: Vec<String>,
    pub sing_voices: Vec<String>,
}

impl EnvNames<'static> {
    /// The site's environment (btwapi's snapshot), the same one documents are checked against.
    pub fn web() -> EnvNames<'static> {
        let w = btwapi::web_env();
        EnvNames::with_registry(&w.registry)
    }
}

impl<'a> EnvNames<'a> {
    /// The site's sounds, with another registry (the host's own, whose params carry their docs).
    pub fn with_registry(registry: &'a dyn Registry) -> EnvNames<'a> {
        let w = btwapi::web_env();
        EnvNames {
            registry,
            voices: w.catalog.voices.clone(),
            families: w.catalog.families.clone(),
            kits: w.catalog.kits.clone(),
            palettes: w.catalog.palettes.clone(),
            sing_voices: djlive::env::StaticVoices::default().names(),
        }
    }
}

fn word_of(t: &djlive::lex::Token) -> Option<&str> {
    match &t.tok {
        Tok::Word(w) => Some(w),
        _ => None,
    }
}

/// The noun a `do` line (or its sugar) names, and its verb.
fn do_call<'t>(words: &[&'t str], reg: &dyn Registry) -> Option<(NounSpec, Option<&'t str>, usize)> {
    let (noun_at, sugar) = match words.first() {
        Some(&"do") => (1, false),
        Some(w) if djlive::parse::sugar_noun(w).is_some() => (0, true),
        _ => return None,
    };
    let noun_word = words.get(noun_at)?;
    let noun = reg.noun(if sugar { djlive::parse::sugar_noun(noun_word).unwrap_or(noun_word) } else { noun_word })?;
    let verb = words.get(noun_at + 1).copied().filter(|v| noun.action(v).is_some());
    Some((noun, verb, noun_at))
}

/// The candidates at byte `at`, ranked, filtered by the word being typed. Also returns the partial statement.
pub fn candidates(a: &Analysis, at: usize, env: &EnvNames) -> (Vec<Item>, Partial) {
    let p = statement_at(&a.text, at);
    if a.in_comment(at) {
        return (Vec::new(), p);
    }
    let mut items = context_items(a, &p, at, env);
    let dollar = p.word.starts_with('$') || (p.word.is_empty() && a.text[..at].ends_with('$'));
    if let Some(d) = p.wants.as_ref().filter(|_| !dollar) {
        // what the parser expects: first where the place says nothing, after what the place offers where it does
        let lift = if items.is_empty() { 0 } else { 3 };
        items.extend(expected_items(a, &d.expected, at).into_iter().map(|mut i| {
            i.rank = i.rank.saturating_add(lift);
            i
        }));
    }
    (rank_and_filter(items, &p.word), p)
}

fn rank_and_filter(items: Vec<Item>, word: &str) -> Vec<Item> {
    let lw = word.to_lowercase();
    let mut seen = BTreeSet::new();
    let mut out: Vec<Item> = Vec::new();
    let mut items = items;
    // the best rank of a label wins
    items.sort_by(|x, y| x.rank.cmp(&y.rank));
    for it in items {
        let ll = it.label.to_lowercase();
        let fits = lw.is_empty() || ll.starts_with(&lw) || (lw.len() >= 2 && subsequence(&lw, &ll));
        if !fits || !seen.insert(it.label.clone()) {
            continue;
        }
        out.push(it);
    }
    // a label that starts with the word goes before one that only contains it, within a rank
    out.sort_by(|x, y| x.rank.cmp(&y.rank).then_with(|| (!x.label.to_lowercase().starts_with(&lw)).cmp(&!y.label.to_lowercase().starts_with(&lw))).then_with(|| x.label.cmp(&y.label)));
    out
}

fn subsequence(needle: &str, hay: &str) -> bool {
    let mut it = hay.chars();
    needle.chars().all(|c| it.any(|h| h == c))
}

/// The program's names of some kinds, as candidates.
fn symbol_items(a: &Analysis, kinds: &[Kind], rank: u8) -> Vec<Item> {
    let mut v = Vec::new();
    for s in &a.symbols.symbols {
        if !kinds.contains(&s.kind) {
            continue;
        }
        let (k, detail) = match s.kind {
            Kind::Section => (kind::MODULE, format!("section, {}", s.detail)),
            Kind::LetSection => (kind::STRUCT, format!("let {} = {}", s.name, s.detail)),
            Kind::Let => (kind::VARIABLE, format!("let {} = {}", s.name, s.detail)),
            Kind::Def => (kind::FUNCTION, format!("def {}{}", s.name, s.detail)),
            Kind::Sample => (kind::FILE, s.detail.clone()),
            _ => (kind::VARIABLE, s.detail.clone()),
        };
        let mut it = Item::new(s.name.clone(), k, rank).detail(detail);
        if s.kind == Kind::Def {
            let params: Vec<&str> = s.children.iter().map(|&c| &a.symbols.symbols[c]).filter(|c| c.kind == Kind::Param).map(|c| c.name.as_str()).collect();
            let args: Vec<String> = params.iter().enumerate().map(|(i, p)| format!("${{{}:{p}}}", i + 1)).collect();
            it = it.snippet(format!("{}({})", s.name, args.join(", ")));
        }
        v.push(it);
    }
    v
}

fn call_items(rank: u8, filter: impl Fn(&str) -> bool) -> Vec<Item> {
    lang::CALLS.iter().filter(|c| filter(c.name)).map(|c| Item::new(c.name, kind::FUNCTION, rank).detail(c.label()).doc(c.doc).snippet(c.snippet)).collect()
}

fn statement_start_items(a: &Analysis, top: bool) -> Vec<Item> {
    let mut v = Vec::new();
    v.push(Item::new("deck A:", kind::SNIPPET, 0).detail("a line of deck A's parts").snippet("deck A: ${1|kick,snare,hihat,clap,bass,melody,harmony,arp|} $0"));
    v.push(Item::new("deck B:", kind::SNIPPET, 0).detail("a line of deck B's parts").snippet("deck B: ${1|kick,snare,hihat,clap,bass,melody,harmony,arp|} $0"));
    v.push(Item::new("master:", kind::SNIPPET, 1).detail("a control line on the master").snippet("master: $0"));
    for (w, doc, snip) in lang::STATEMENTS {
        // a section, an arrange, an import ... belong at the top
        let top_only = matches!(*w, "section" | "arrange" | "import" | "def" | "sample" | "set" | "use" | "end" | "loop" | "wire" | "launch");
        if top_only && !top {
            continue;
        }
        let k = if snip.contains('\n') { kind::SNIPPET } else { kind::KEYWORD };
        v.push(Item::new(*w, k, 2).doc(*doc).snippet(*snip));
    }
    v.extend(symbol_items(a, &[Kind::Def], 1));
    for (w, _) in djlive::parse::SUGAR {
        v.push(Item::new(w, kind::KEYWORD, 3).detail(format!("do {w} ...")));
    }
    v
}

/// How an arrangement ends: `end`, `loop`, `loop N then ... end` (after a number: `then`).
fn ending_items(after_number: bool) -> Vec<Item> {
    lang::ENDINGS
        .iter()
        .filter(|(w, _, _)| !after_number || *w == "then")
        .map(|(w, d, snip)| Item::new(*w, kind::KEYWORD, 0).doc(*d).snippet(*snip))
        .collect()
}

/// The arrange mutation operators, as docs/ARRANGE.md says them.
fn mutation_op_items() -> Vec<Item> {
    lang::mutation_ops()
        .into_iter()
        .map(|(n, sig, doc)| {
            let args: Vec<String> = sig.split_whitespace().skip(1).enumerate().map(|(i, a)| format!("${{{}:{a}}}", i + 1)).collect();
            let it = Item::new(n, kind::FUNCTION, 0).detail(format!("mutation: {sig}")).doc(doc);
            if args.is_empty() {
                it
            } else {
                it.snippet(format!("{n} {}", args.join(" ")))
            }
        })
        .collect()
}

/// A line of an `arrange { }` block, from the tokens before the cursor.
fn arrange_block_items(a: &Analysis, toks: &[djlive::lex::Token]) -> Vec<Item> {
    let mark = matches!(toks.first().map(|t| &t.tok), Some(Tok::Tilde | Tok::At));
    if !mark {
        let mut v = symbol_items(a, &[Kind::Section], 0);
        if toks.is_empty() {
            v.push(Item::new("~", kind::OPERATOR, 1).detail("a mutation, from here on").doc("`~ acid_a rot 2 | thin 0.1`: every section after it reads `acid_a` rotated and thinned (`~3 ...`: on every 3rd pass only)").snippet("~ ${1:pattern} ${2:rot 2}"));
            v.push(Item::new("@", kind::OPERATOR, 1).detail("a mutation for the next section only").doc("`@ hoov rev`: the next section reads `hoov` backwards, then it reverts (`@4 ...`: on every 4th pass only)").snippet("@ ${1:pattern} ${2:rev}"));
        }
        return v;
    }
    let last = toks.last().map(|t| &t.tok);
    let prev = toks.len().checked_sub(2).map(|i| &toks[i].tok);
    let targets = || symbol_items(a, &[Kind::Let], 0);
    match last {
        // `~ ` / `, ` / `~3 `: what to change (a let pattern, or a glob `acid_*`)
        Some(Tok::Tilde | Tok::At | Tok::Comma) => targets(),
        Some(Tok::Num { .. }) if matches!(prev, Some(Tok::Tilde | Tok::At)) => targets(),
        // `| `: the next operator of the chain
        Some(Tok::Pipe) => mutation_op_items(),
        Some(Tok::Word(w)) => {
            // `~ acid_a `: an operator; `morph `: the pattern it takes
            let counted = matches!(prev, Some(Tok::Num { .. })) && toks.len().checked_sub(3).is_some_and(|i| matches!(toks[i].tok, Tok::Tilde | Tok::At));
            if matches!(prev, Some(Tok::Tilde | Tok::At | Tok::Comma)) || matches!(prev, Some(Tok::Star)) || counted {
                mutation_op_items()
            } else if djlive::mutate::operator_args(w).is_some_and(|k| k.first() == Some(&"pattern")) {
                targets()
            } else if djlive::mutate::operator_args(w).is_some_and(|k| k.first() == Some(&"operator")) {
                // `jux `: the operator the right copy gets (not another `jux`: it forks once)
                mutation_op_items().into_iter().filter(|i| i.label != *w).collect()
            } else {
                Vec::new()
            }
        }
        Some(Tok::Star) => mutation_op_items(),
        _ => Vec::new(),
    }
}

fn evolve_items(at_target: bool) -> Vec<Item> {
    if at_target {
        let mut v: Vec<Item> = djlive::parse::ROLES.iter().map(|r| Item::new(format!("{r}:"), kind::FIELD, 0).detail("an evolve rule for this role on every deck").doc(lang::role_doc(r))).collect();
        v.push(Item::new("deck A:", kind::FIELD, 1).detail("an evolve rule for every part of deck A"));
        v.push(Item::new("deck B:", kind::FIELD, 1).detail("an evolve rule for every part of deck B"));
        v.push(Item::new("stop when gen ==", kind::SNIPPET, 1).detail("the last generation").snippet("stop when gen == ${1:8}"));
        v
    } else {
        lang::EVOLVE_OPS.iter().map(|(n, d)| Item::new(*n, kind::FUNCTION, 0).detail("evolve operator").doc(*d)).collect()
    }
}

fn param_value_items(a: &Analysis, param: &str, words: &[&str], env: &EnvNames) -> Vec<Item> {
    let mut v = Vec::new();
    match param {
        "inst" => {
            v.extend(symbol_items(a, &[Kind::Sample], 0));
            v.extend(env.voices.iter().map(|n| Item::new(n.clone(), kind::VALUE, 1).detail("voice")));
            v.extend(env.families.iter().map(|n| Item::new(n.clone(), kind::VALUE, 3).detail("family word: the family's generic voice")));
        }
        "kit" => v.extend(env.kits.iter().map(|n| Item::new(n.clone(), kind::VALUE, 0).detail("drum kit"))),
        "sound" => {
            for o in ["own", "synth"] {
                v.push(Item::new(o, kind::ENUM_MEMBER, 0).detail("role sound"));
            }
            v.extend(env.palettes.iter().map(|n| Item::new(format!("synth:{n}"), kind::VALUE, 1).detail("palette")));
        }
        "voice" => v.extend(env.sing_voices.iter().map(|n| Item::new(n.clone(), kind::VALUE, 0).detail("singing voice"))),
        _ => {
            if let Some((noun, verb, _)) = do_call(words, env.registry) {
                let spec = verb.and_then(|vb| noun.action(vb)).and_then(|act| act.params.iter().find(|p| p.name == param).cloned()).or_else(|| noun.param(param).cloned());
                if let Some(spec) = spec {
                    v.extend(spec.options.iter().map(|o| Item::new(o.clone(), kind::ENUM_MEMBER, 0).detail(format!("{param} option"))));
                    if spec.types.iter().any(|t| t == "boolean") {
                        v.extend(["true", "false"].map(|b| Item::new(b, kind::VALUE, 0)));
                    }
                }
            }
        }
    }
    // a number may be a scalar let
    v.extend(symbol_items(a, &[Kind::Let], 4));
    v
}

fn do_items(words: &[&str], present: &BTreeSet<String>, env: &EnvNames) -> Vec<Item> {
    let mut v = Vec::new();
    if words.first() == Some(&"do") && words.len() == 1 {
        for n in env.registry.noun_names() {
            let spec = env.registry.noun(&n);
            let verbs = spec.map(|s| s.actions.iter().map(|a| a.name.clone()).collect::<Vec<_>>().join(", ")).unwrap_or_default();
            v.push(Item::new(n, kind::MODULE, 0).detail(if verbs.is_empty() { "noun".to_string() } else { format!("noun: {verbs}") }));
        }
        return v;
    }
    let Some((noun, verb, noun_at)) = do_call(words, env.registry) else { return v };
    if verb.is_none() && words.len() == noun_at + 1 {
        for act in &noun.actions {
            let def = if act.name == noun.default_action { " (the default)" } else { "" };
            v.push(Item::new(act.name.clone(), kind::FUNCTION, 0).detail(format!("{} {}: {}{def}", noun.name, act.name, act.effect)));
        }
    }
    let params = verb.and_then(|vb| noun.action(vb)).map(|act| act.params.clone()).filter(|p| !p.is_empty()).unwrap_or_else(|| noun.params.clone());
    for p in params {
        if present.contains(&p.name) {
            continue;
        }
        let ty = if p.options.is_empty() { p.types.join(" | ") } else { p.options.join(" | ") };
        let req = if p.required { ", required" } else { "" };
        let rank = if p.required { 0 } else { 1 };
        v.push(Item::new(format!("{}=", p.name), kind::PROPERTY, rank).detail(format!("{ty}{req}")).doc(p.doc.clone()).snippet(format!("{}=$1", p.name)));
    }
    v
}

fn part_items(a: &Analysis, terms: usize) -> Vec<Item> {
    let mut v = symbol_items(a, &[Kind::Let], 0);
    v.push(Item::new("x", kind::CONSTANT, 1).detail("a hit"));
    v.push(Item::new("~", kind::CONSTANT, 1).detail("a rest"));
    v.extend(call_items(2, |n| !matches!(n, "stack" | "then" | "knob" | "pad")));
    v.push(Item::new("choose", kind::SNIPPET, 3).detail("one option a bar, by weight").snippet("choose [${1:a b c}] weights ${2:3 1 1}"));
    // the parameters, once the part has steps
    let prank = if terms > 0 { 1 } else { 4 };
    for (n, d, snip) in lang::PART_PARAMS {
        v.push(Item::new(format!("{n}="), kind::PROPERTY, prank).doc(*d).snippet(*snip));
    }
    v.push(Item::new("voice-lead", kind::PROPERTY, prank + 1).detail("chords voice-led from one to the next"));
    v
}

fn line_items(deck: bool) -> Vec<Item> {
    let mut v = Vec::new();
    if deck {
        v.extend(djlive::parse::ROLES.iter().map(|r| Item::new(*r, kind::FIELD, 0).detail("role").doc(lang::role_doc(r))));
    }
    for w in djlive::parse::ACTION_WORDS.iter().chain(["drive", "duck", "mix", "sing"].iter()) {
        let doc = lang::action_doc(w);
        v.push(Item::new(*w, kind::KEYWORD, 2).detail("control").doc(doc));
    }
    v
}

/// `gen@2`, the seeded generator's call, as an item: `what` says which piece of its program the place takes, `tail` the names
/// the snippet goes on with.
fn gen_item(rank: u8, what: &str, example: &str, tail: &str) -> Item {
    let head = format!("gen@{}", djlive::gen::BARE);
    Item::new(head.clone(), kind::SNIPPET, rank)
        .detail(format!("the seeded generator: {what}"))
        .doc(format!("`{example}`: an engine call, pinned by the CID of the program it prints; it takes the program's key and tempo unless it says its own"))
        .snippet(format!("{head} ${{1:techno}} seed=${{2:42}}{tail}"))
}

/// After `arrange from`: a seed, a generator call or a template; inside `template(`: the templates, then the phrasebook's ids.
fn from_items(toks: &[djlive::lex::Token]) -> Vec<Item> {
    let template = toks.iter().position(|t| matches!(&t.tok, Tok::Word(w) if w == "template"));
    match template {
        None if toks.len() > 2 => Vec::new(),
        None => vec![
            Item::new("template(", kind::SNIPPET, 0)
                .detail("a phrase compiled to a whole track")
                .doc("`arrange from template(deep-house, @tempest_such_stuff)`: the phrase is the vocal line of the track; any words in quotes work too")
                .snippet("template(${1:deep-house}, ${2:@tempest_such_stuff})"),
            Item::new("seed(", kind::SNIPPET, 1).detail("a track of the beats generator").doc("`arrange from seed(techno, 42)`").snippet("seed(${1:techno}, ${2:42})"),
            gen_item(1, "its whole program", "arrange from gen@2 techno seed=42", ""),
        ],
        Some(_) if toks.iter().any(|t| t.tok == Tok::Comma) => {
            let mut v: Vec<Item> = djengine::vocal::phoneme::pd_phrases().iter().map(|p| Item::new(format!("@{}", p.id), kind::VALUE, 0).detail(format!("{}: {}", p.family, p.text)).doc(format!("{}, {}, {}", p.work, p.author, p.year))).collect();
            v.extend(djengine::vocal::phoneme::phrases().into_iter().map(|p| Item::new(format!("@{}", p.id), kind::VALUE, 1).detail(p.words)));
            v
        }
        Some(_) => djlive::template::TEMPLATES.iter().map(|t| Item::new(t.name, kind::VALUE, 0).detail("template").doc(t.summary)).collect(),
    }
}

/// The program a generator call prints, compiled (no environment: the music is the same on every host), for completion to
/// read its sections from. `call`: the call's text as far as it is written. A few ms: only after `section=`.
fn generated(a: &Analysis, call: &str) -> Option<djlive::Compiled> {
    let r = djlive::gen::resolve_word(call.trim(), &djlive::gen::host_of(&a.tokens)).ok()?;
    djlive::compile::check_with(&r.text, &djlive::env::Env::none()).compiled
}

/// Inside a `gen@2` call (refs.rs): after its head the genres of the installed generator; then its names (`seed=` first, it is
/// required), each once; after a name its values: the lengths, the keys, the roles, and for `section=` the sections of the
/// program the call prints (generated here). None when the cursor is not in a call (a `gen` of `evolve`'s, or the line's own
/// params after the call).
fn gen_items(a: &Analysis, toks: &[djlive::lex::Token]) -> Option<Vec<Item>> {
    let g = toks.iter().rposition(|t| matches!(&t.tok, Tok::Word(w) if crate::refs::is_gen_head(w)))?;
    let head = word_of(&toks[g])?;
    let prev = g.checked_sub(1).and_then(|i| word_of(&toks[i]));
    let on_part = prev.is_some_and(|p| djlive::parse::ROLES.contains(&p));
    // a bare `gen` is a call only where one goes (evolve's `gen` is in a `when`)
    if head == "gen" && !(on_part || prev == Some("from") || g.checked_sub(1).is_some_and(|i| toks[i].tok == Tok::Eq)) {
        return None;
    }
    let version = head.strip_prefix("gen@").and_then(|n| n.parse::<u32>().ok()).unwrap_or(djlive::gen::BARE);
    let after = &toks[g + 1..];
    if after.iter().any(|t| !matches!(t.tok, Tok::Word(_) | Tok::Eq | Tok::Num { .. } | Tok::Colon)) {
        return None;
    }
    if after.is_empty() {
        let genres = djlive::gen::generator().map(|gen| gen.genres(version)).unwrap_or_default();
        return Some(genres.into_iter().map(|n| Item::new(n, kind::VALUE, 0).detail(format!("a genre of seeded@{version}"))).collect());
    }
    // the names said so far; a name that is not the call's ends it (`inst=` of the part line)
    let names: Vec<(usize, &str)> = after.windows(2).enumerate().filter(|(_, w)| w[1].tok == Tok::Eq).filter_map(|(k, w)| word_of(&w[0]).map(|n| (k, n))).collect();
    if names.iter().any(|(_, n)| !djlive::gen::PARAMS.contains(n)) {
        return None;
    }
    if after.last().map(|t| &t.tok) == Some(&Tok::Eq) {
        let &(k, name) = names.last()?;
        let mut v = Vec::new();
        match name {
            "length" => {
                for l in djlive::gen::LENGTHS {
                    v.push(Item::new(l, kind::ENUM_MEMBER, 0).detail(if l == "standard" { "the default" } else { "length" }));
                }
            }
            "key" => v.extend(lang::KEYS.iter().map(|k| Item::new(*k, kind::VALUE, 0).detail("key (the program's own when the call says none)"))),
            "part" => v.extend(djlive::parse::ROLES.iter().map(|r| Item::new(*r, kind::FIELD, 0).detail("role").doc(lang::role_doc(r)))),
            "section" => {
                // the program the call prints, as far as it is written: its sections, in the order it plays them
                let call = &a.text[toks[g].span.start..after[k].span.start];
                if let Some(c) = generated(a, call) {
                    let mut seen: Vec<&str> = Vec::new();
                    for seg in &c.segments {
                        if seg.header.is_some() && !seg.chain && !seg.name.contains('+') && !seen.contains(&seg.name.as_str()) {
                            seen.push(&seg.name);
                            let roles: Vec<&str> = seg.decks.values().flat_map(|d| d.parts.iter().map(|p| p.role.as_str())).filter(|r| djlive::parse::ROLES.contains(r)).collect();
                            v.push(Item::new(seg.name.clone(), kind::MODULE, 0).detail(format!("{} bars: {}", seg.bars, roles.join(" "))));
                        }
                    }
                }
            }
            _ => {}
        }
        return Some(v);
    }
    let mut v: Vec<Item> = djlive::gen::PARAMS
        .iter()
        .filter(|n| !names.iter().any(|(_, m)| m == *n))
        .map(|n| {
            let (rank, doc) = match *n {
                "seed" => (0, "a whole number or a word (required)"),
                "bpm" => (1, "a tempo, 40 to 300 (the program's own when not said)"),
                "key" => (1, "a key (the program's own when not said)"),
                "length" => (1, "short, standard (the default) or long"),
                "section" => (1, "which of its sections"),
                _ => (1, "which part (a role)"),
            };
            Item::new(format!("{n}="), kind::PROPERTY, rank).detail(doc).snippet(format!("{n}=$1"))
        })
        .collect();
    // on a part line the line's own params may follow the call
    if on_part {
        for (n, d, snip) in lang::PART_PARAMS {
            v.push(Item::new(format!("{n}="), kind::PROPERTY, 4).doc(*d).snippet(*snip));
        }
    }
    Some(v)
}

/// The candidates the tokens before the cursor call for.
fn context_items(a: &Analysis, p: &Partial, at: usize, env: &EnvNames) -> Vec<Item> {
    let toks = &p.tokens;
    let words: Vec<&str> = toks.iter().filter_map(word_of).collect();
    let last = toks.last().map(|t| &t.tok);
    let before_last = if toks.len() >= 2 { Some(&toks[toks.len() - 2].tok) } else { None };
    let container = a.symbols.container_at(at).map(|i| &a.symbols.symbols[i]);
    // `$` typed: a parameter or a port of the def
    let dollar = p.word.starts_with('$') || (p.word.is_empty() && a.text[..at].ends_with('$'));
    if dollar {
        let Some(def) = a.symbols.symbols.iter().enumerate().filter(|(_, s)| s.kind == Kind::Def && s.body.is_some_and(|b| b.start < at && at < b.end)).last() else { return Vec::new() };
        return def.1.children.iter().map(|&c| &a.symbols.symbols[c]).filter(|c| matches!(c.kind, Kind::Param | Kind::Port)).map(|c| Item::new(format!("${}", c.name), kind::TYPE_PARAMETER, 0).detail(format!("{} of {}", c.detail, def.1.name))).collect();
    }
    // `owner.`: a def's ports, a deck's roles (a `.` glued to a word with nothing after it lexes as a ghost mark, `x.`;
    // where the lexer refuses it alone, the text says)
    let dot = last == Some(&Tok::Dot) || (last == Some(&Tok::Ghost) && matches!(before_last, Some(Tok::Word(_))));
    let dotted = dot || (a.text[..p.word_span.start].ends_with('.') && matches!(last, Some(Tok::Word(_))));
    if dotted {
        let owner_tok = if dot { before_last } else { last };
        if let Some(Tok::Word(owner)) = owner_tok {
            if owner == "A" || owner == "B" {
                return djlive::parse::ROLES.iter().map(|r| Item::new(*r, kind::FIELD, 0).detail("role")).collect();
            }
            if let Some((_, def)) = a.symbols.named(owner).find(|(_, s)| s.kind == Kind::Def) {
                return def.children.iter().map(|&c| &a.symbols.symbols[c]).filter(|c| c.kind == Kind::Port).map(|c| Item::new(c.name.clone(), kind::FIELD, 0).detail(c.detail.clone())).collect();
            }
        }
        return Vec::new();
    }
    // in a generator call: its genres, names and values
    if let Some(v) = gen_items(a, toks) {
        return v;
    }
    // `name=`: the value
    if last == Some(&Tok::Eq) && !matches!(before_last, Some(Tok::Eq | Tok::Lt | Tok::Gt | Tok::Bang)) {
        if let Some(Tok::Word(param)) = before_last {
            if words.first() != Some(&"let") {
                return param_value_items(a, param, &words, env);
            }
        }
        if words.first() == Some(&"let") {
            let mut v = call_items(1, |_| true);
            v.extend(symbol_items(a, &[Kind::Let, Kind::Section], 2));
            v.push(gen_item(3, "one part of a section of its program, a let a bar", "let bed = gen@2 techno seed=42 section=drop part=bass", " section=${3:drop} part=${4:bass}"));
            return v;
        }
    }
    let head = words.first().copied().unwrap_or("");
    let in_evolve = container.is_some_and(|c| c.kind == Kind::Evolve);
    // after an arrange block's `}`, on its line: how it ends (`loop 4 then outro end`)
    if let Some(arr) = a.symbols.symbols.iter().find(|s| matches!(s.kind, Kind::Arrange | Kind::Arrangement) && s.body.is_some_and(|b| b.end <= at && !a.text[b.end..at].contains('\n'))) {
        let tail: Vec<&str> = a.text[arr.body.unwrap().end..at].split_whitespace().collect();
        let typing = !a.text[..at].ends_with(char::is_whitespace);
        let done: &[&str] = if typing { &tail[..tail.len().saturating_sub(1)] } else { &tail[..] };
        return match done.last() {
            Some(&"then") => {
                let mut v = if arr.kind == Kind::Arrangement { symbol_items(a, &[Kind::Arrangement], 0) } else { symbol_items(a, &[Kind::Section], 0) };
                v.push(Item::new("end", kind::KEYWORD, 1).doc(lang::ENDINGS[0].1));
                v
            }
            Some(&"end") => Vec::new(),
            Some(w) if w.parse::<f64>().is_ok() => ending_items(true),
            Some(_) if done.contains(&"then") => {
                let mut v = symbol_items(a, &[Kind::Section], 0);
                v.push(Item::new("end", kind::KEYWORD, 0).doc(lang::ENDINGS[0].1));
                v
            }
            _ => ending_items(false),
        };
    }
    // a line of an `arrange { }` block: sections, or a mutation (`~ acid_a rot 2 | thin 0.1, hoov up 2`)
    if container.is_some_and(|c| c.kind == Kind::Arrange || (c.kind == Kind::Arrangement && c.body.is_some())) {
        return arrange_block_items(a, toks);
    }
    if toks.is_empty() {
        if in_evolve {
            return evolve_items(true);
        }
        if container.is_some_and(|c| c.kind == Kind::Block && c.name == "mix") {
            return ["A", "B", "master"].iter().map(|d| Item::new(*d, kind::FIELD, 0).detail("a mixer strip: level, eq low/mid/high, pan")).collect();
        }
        return statement_start_items(a, container.is_none());
    }
    // an evolve rule: `melody: rotate 1 ...`
    if in_evolve && toks.iter().any(|t| t.tok == Tok::Colon) && head != "stop" {
        return evolve_items(false);
    }
    if head == "start" && toks.len() == 1 {
        return symbol_items(a, &[Kind::Arrangement], 0);
    }
    if head == "arrange" && words.get(1) == Some(&"from") {
        return from_items(toks);
    }
    // `launch drop on pad(2)`: a scene is a section or a named arrangement; then the pad
    if head == "launch" && toks.len() == 1 {
        return symbol_items(a, &[Kind::Section, Kind::Arrangement], 0);
    }
    if head == "launch" && toks.len() == 2 {
        return vec![Item::new("on pad()", kind::SNIPPET, 0).detail("the pad that brings the scene in").doc("`launch drop on pad(2)`: a hit of pad 2 queues the scene for the next phrase line; `next bar` or `next phrase 16` say another").snippet("on pad(${1:1})")];
    }
    if head == "arrange" {
        let named = toks.len() >= 3 && toks[2].tok == Tok::Colon;
        let after_block = toks.iter().any(|t| t.tok == Tok::RBrace);
        if last == Some(&Tok::Word("then".into())) {
            // `then` names the coda's sections after a block, the arrangement to go on to after a named one
            let mut v = if named { symbol_items(a, &[Kind::Arrangement], 0) } else { symbol_items(a, &[Kind::Section], 0) };
            if after_block {
                v.push(Item::new("end", kind::KEYWORD, 1).doc(lang::ENDINGS[0].1));
            }
            return v;
        }
        if after_block {
            // how the block ends: grammar.ebnf's `block_end`
            return ending_items(matches!(last, Some(Tok::Num { .. })));
        }
        if named && toks.len() > 3 {
            let mut v = symbol_items(a, &[Kind::Section, Kind::LetSection], 0);
            v.extend(ending_items(false).into_iter().map(|mut i| {
                i.rank = 1;
                i
            }));
            return v;
        }
        if toks.len() == 1 {
            let mut v = vec![Item::new("{ }", kind::SNIPPET, 1).detail("an arrangement with mutations").doc("`arrange { intro build ~ acid rot 2 drop } loop 4 then outro end`: the sections in order, `~` a mutation from there on, `@` for the next section only").snippet("{\n  ${1:intro}\n  ~ ${2:pattern} ${3:rot 2}\n  $0\n}")];
            v.push(Item::new("name:", kind::SNIPPET, 2).detail("one of several named arrangements").doc("`arrange set: drop wubs loop`, `start set`").snippet("${1:name}: ${2:intro} $0"));
            v.extend(symbol_items(a, &[Kind::Section, Kind::LetSection], 0));
            v.extend(call_items(2, |n| matches!(n, "stack" | "then")));
            v.push(Item::new("silence", kind::KEYWORD, 3).detail("bars of nothing"));
            return v;
        }
        let mut v = symbol_items(a, &[Kind::Section, Kind::LetSection], 0);
        v.extend(call_items(2, |n| matches!(n, "stack" | "then")));
        v.push(Item::new("silence", kind::KEYWORD, 3).detail("bars of nothing"));
        return v;
    }
    // inside `stack(` / `then(` of a `let`: sections
    if head == "let" && toks.iter().any(|t| matches!(&t.tok, Tok::Word(w) if w == "stack" || w == "then")) {
        return symbol_items(a, &[Kind::Section, Kind::LetSection], 0);
    }
    if matches!(words.last(), Some(&"after" | &"before")) && matches!(last, Some(Tok::Word(_))) {
        return symbol_items(a, &[Kind::Section, Kind::LetSection], 0);
    }
    if head == "end" && words.len() == 1 {
        return vec![Item::new("at bar", kind::KEYWORD, 0).snippet("at bar ${1:64}"), Item::new("after", kind::KEYWORD, 0).snippet("after ${1:outro}")];
    }
    if head == "section" && toks.len() == 3 && words.get(2) == Some(&"from") {
        return vec![gen_item(0, "a section of its program", "section hook from gen@2 techno seed=42 section=drop", " section=${3:drop}")];
    }
    if head == "section" && words.len() == 2 && toks.len() == 3 {
        return vec![Item::new("bars", kind::KEYWORD, 0).snippet("bars {\n  $0\n}")];
    }
    if head == "section" && toks.len() == 2 {
        let mut v = vec![Item::new("16", kind::VALUE, 1), Item::new("8", kind::VALUE, 1)];
        v.extend(symbol_items(a, &[Kind::Let], 0));
        return v;
    }
    if head == "key" && toks.len() == 1 {
        return lang::KEYS.iter().map(|k| Item::new(*k, kind::VALUE, 0).detail("key")).collect();
    }
    if head == "evolve" && toks.len() == 1 {
        return vec![Item::new("every", kind::KEYWORD, 0).snippet("every ${1:8} bars {\n  $0\n}")];
    }
    if head == "wire" && matches!(last, Some(Tok::Arrow)) || (head == "wire" && toks.len() == 1) {
        let out = toks.len() == 1;
        return a.symbols.symbols.iter().filter(|s| s.kind == Kind::Def).map(|s| Item::new(s.name.clone(), kind::FUNCTION, 0).detail(format!("def {}{}", s.name, s.detail)).snippet(format!("{}.{}", s.name, if out { "out" } else { "$1" }))).collect();
    }
    let present: BTreeSet<String> = toks.windows(2).filter(|w| w[1].tok == Tok::Eq).filter_map(|w| word_of(&w[0]).map(str::to_string)).collect();
    if head == "do" || (djlive::parse::sugar_noun(head).is_some() && head != "key" && !toks.iter().any(|t| t.tok == Tok::Colon)) {
        return do_items(&words, &present, env);
    }
    // a line of parts or controls: after `deck A:` or `master:` (perhaps after a timing `every 4 bars:`)
    let colon_at = toks.iter().rposition(|t| t.tok == Tok::Colon);
    if let Some(c) = colon_at {
        let target = if c >= 2 && word_of(&toks[c - 2]) == Some("deck") { Some(true) } else if c >= 1 && word_of(&toks[c - 1]) == Some("master") { Some(false) } else { None };
        match target {
            Some(deck) => {
                let seg_start = toks.iter().rposition(|t| t.tok == Tok::Pipe).map_or(c + 1, |k| k + 1).max(c + 1);
                let seg = &toks[seg_start..];
                let first = seg.first().and_then(word_of);
                match first {
                    None => return line_items(deck),
                    Some(r) if djlive::parse::ROLES.contains(&r) => {
                        // a part: after its role, terms (and once it has some, params)
                        let terms = seg.len().saturating_sub(1);
                        return part_items(a, terms);
                    }
                    Some("filter") if seg.len() == 1 => return ["lp", "hp", "off", "open"].iter().map(|m| Item::new(*m, kind::ENUM_MEMBER, 0).detail("filter mode")).collect(),
                    Some(_) => {
                        let mut v = symbol_items(a, &[Kind::Let], 1);
                        v.push(Item::new("lfo", kind::KEYWORD, 2).snippet("lfo ${1|sine,tri,square,saw|} ${2:2} bars depth=${3:0.2} center=${4:0.5}"));
                        v.push(Item::new("over", kind::KEYWORD, 2).snippet("over ${1:8} bars"));
                        return v;
                    }
                }
            }
            None => {
                // a timing just ended (`every 4 bars:`): a line comes
                if c + 1 == toks.len() && matches!(head, "every" | "on" | "at" | "after" | "before" | "for") {
                    let mut v = vec![
                        Item::new("deck A:", kind::SNIPPET, 0).snippet("deck A: $0"),
                        Item::new("deck B:", kind::SNIPPET, 0).snippet("deck B: $0"),
                        Item::new("master:", kind::SNIPPET, 0).snippet("master: $0"),
                        Item::new("do", kind::KEYWORD, 1).snippet("do ${1:noun}"),
                    ];
                    v.extend(line_items(false));
                    v.extend(symbol_items(a, &[Kind::Def], 1));
                    return v;
                }
            }
        }
    }
    // a call of a def at the start of a line, or a word the parser expects: left to the expected list
    Vec::new()
}

/// The parser's prose `expected` at the cursor as typed candidates. A word in backticks is itself; "a section name" is the
/// sections; "a key like am" the keys; "a generator like e(5,8)" the generators; "a setting (tempo, key, swing)" each word.
pub fn expected_items(a: &Analysis, expected: &[String], _at: usize) -> Vec<Item> {
    let mut v = Vec::new();
    for e in expected {
        let t = e.trim();
        if let Some(inner) = t.strip_prefix('`').and_then(|x| x.strip_suffix('`')) {
            // a word or a phrase (`deck A:`), not a mark (`[`, `<`): a mark is typed, not picked
            if !inner.contains('`') && inner.chars().next().is_some_and(char::is_alphabetic) {
                let it = Item::new(inner, kind::KEYWORD, 0).detail("expected here");
                v.push(if inner == "bars" { it.snippet("bars") } else { it });
            }
            continue;
        }
        if t.contains("section name") {
            v.extend(symbol_items(a, &[Kind::Section, Kind::LetSection], 0));
        } else if t.starts_with("a key") {
            v.extend(lang::KEYS.iter().map(|k| Item::new(*k, kind::VALUE, 1).detail("key")));
        } else if t.starts_with("a generator") {
            v.extend(call_items(1, |n| crate::lang::call(n).is_some() && djlive::pattern::GENERATORS.contains(&n)));
        } else if t == "a step" {
            v.push(Item::new("x", kind::CONSTANT, 0).detail("a hit"));
            v.push(Item::new("~", kind::CONSTANT, 0).detail("a rest"));
        } else if let Some(name) = t.strip_suffix("(...)") {
            if let Some(c) = lang::call(name) {
                v.push(Item::new(c.name, kind::FUNCTION, 0).detail(c.label()).doc(c.doc).snippet(c.snippet));
            }
        } else if let (Some(o), Some(c)) = (t.find(" ("), t.rfind(')')) {
            // "a setting (tempo, key, swing)"
            for w in t[o + 2..c].split([',']).flat_map(|x| x.split(" or ")) {
                let w = w.trim();
                if !w.is_empty() && !w.contains(' ') {
                    v.push(Item::new(w, kind::KEYWORD, 1).detail("expected here"));
                }
            }
        } else if !t.contains(' ') && !t.is_empty() {
            // a bare word: `on`, `lp`
            v.push(Item::new(t.trim_matches('"'), kind::KEYWORD, 0).detail("expected here"));
        }
    }
    v
}

/// `textDocument/completion`: the items, each replacing the word being typed.
pub fn completion(a: &Analysis, at: usize, env: &EnvNames, enc: Encoding) -> JsonValue {
    let (items, p) = candidates(a, at, env);
    let range = span_range(&a.index, &a.text, p.word_span, enc);
    let list: Vec<JsonValue> = items
        .into_iter()
        .take(200)
        .map(|it| {
            let mut o = JsonObject::new();
            o.set("label", s(it.label.clone()));
            o.set("kind", JsonValue::int(it.kind));
            if !it.detail.is_empty() {
                o.set("detail", s(it.detail.clone()));
            }
            if !it.doc.is_empty() {
                o.set("documentation", obj([("kind", s("markdown")), ("value", s(it.doc.clone()))]));
            }
            o.set("sortText", s(format!("{:02}{}", it.rank, it.label)));
            o.set("filterText", s(it.label.clone()));
            let (text, fmt) = match &it.snippet {
                Some(sn) => (sn.clone(), 2),
                None => (it.label.clone(), 1),
            };
            o.set("insertTextFormat", JsonValue::int(fmt));
            o.set("textEdit", obj([("range", range.clone()), ("newText", s(text))]));
            JsonValue::Object(o)
        })
        .collect();
    obj([("isIncomplete", JsonValue::Bool(false)), ("items", arr(list))])
}
