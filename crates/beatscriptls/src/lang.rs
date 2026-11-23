//! What the service knows of the language beyond the tables djlive exports: the signatures and one-line docs of the
//! generators and transforms (docs/BEATSCRIPT.md "Generators", "Pattern transforms"), the part parameters, the evolve operators, the
//! statement keywords and their snippets. Written once here so completion, hover and signature help say the same thing.

/// A callable: `e(k, n [, rot])`.
#[derive(Clone, Copy, Debug)]
pub struct Sig {
    pub name: &'static str,
    /// The parameters; an optional one is in `[ ]`.
    pub params: &'static [&'static str],
    pub doc: &'static str,
    /// The snippet that is inserted (`e(${1:5}, ${2:8})`).
    pub snippet: &'static str,
}

impl Sig {
    pub fn label(&self) -> String {
        format!("{}({})", self.name, self.params.join(", "))
    }
}

pub const CALLS: &[Sig] = &[
    Sig { name: "e", params: &["k", "n", "[rot]"], doc: "Euclidean rhythm: k hits spread over n steps (Bjorklund), rotated by rot. One slot: `kick e(5,8)` is a bar of eight steps. 0 <= k <= n, 1 <= n <= 64.", snippet: "e(${1:5}, ${2:8})" },
    Sig { name: "euclid", params: &["k", "n", "[rot]"], doc: "The long name of `e(k, n [, rot])` (printed as `e(...)`).", snippet: "euclid(${1:5}, ${2:8})" },
    Sig { name: "markov", params: &["[key]", "density", "[seed]"], doc: "A Markov melody of one bar (16 steps) in the key's scale; the program's key when left out. density 0..1; the seed is taken from the text when left out, so the same text is the same notes.", snippet: "markov(${1:am}, ${2:0.7})" },
    Sig { name: "arp", params: &["chord", "[mode]", "[n]"], doc: "The chord's notes one at a time over n steps (default 8), mode up | down | updown. A chord is a symbol (am7) or a Roman numeral (V7), read in the key in force.", snippet: "arp(${1:am7}, ${2:up}, ${3:8})" },
    Sig { name: "chord", params: &["chord", "[octave]"], doc: "The chord's notes together. A chord is a symbol (am7) or a Roman numeral (V7), read in the key in force.", snippet: "chord(${1:am7})" },
    Sig { name: "maybe", params: &["p", "prob", "[seed]"], doc: "Each hit of p kept with chance prob (0 to 1), seeded by a hash of the bar, the step and the note.", snippet: "maybe([${1:x ~ x ~}], ${2:0.75})" },
    Sig { name: "transpose", params: &["p", "n"], doc: "p moved n semitones up (a hit in a tonal part is its root first).", snippet: "transpose(${1:p}, ${2:5})" },
    Sig { name: "octave", params: &["p", "n"], doc: "p moved n octaves up.", snippet: "octave(${1:p}, ${2:1})" },
    Sig { name: "reverse", params: &["p"], doc: "The steps of p backwards.", snippet: "reverse(${1:p})" },
    Sig { name: "rotate", params: &["p", "n"], doc: "The steps of p moved n steps along.", snippet: "rotate(${1:p}, ${2:1})" },
    Sig { name: "every", params: &["n", "p", "[q]"], doc: "p on every nth bar (the 4th, 8th ...), q (or a rest) on the others.", snippet: "every(${1:4}, ${2:p})" },
    Sig { name: "degrade", params: &["p", "a", "[seed]"], doc: "Drops the share a of the hits; seeded, so the same text is the same hits.", snippet: "degrade(${1:p}, ${2:0.2})" },
    Sig { name: "slow", params: &["p", "n"], doc: "p stretched over n bars.", snippet: "slow(${1:p}, ${2:2})" },
    Sig { name: "fast", params: &["p", "n"], doc: "p played n times a bar.", snippet: "fast(${1:p}, ${2:2})" },
    Sig { name: "humanize", params: &["p", "seed", "[amount]"], doc: "Velocities wander by up to amount (default 10), seeded.", snippet: "humanize(${1:p}, ${2:7})" },
    Sig { name: "rests", params: &["p", "[q]"], doc: "The answer to p: a hit on every step where p rests, or q kept only on those steps. The steps are p's own (`x ~ x ~` is four). p has no chance in it.", snippet: "rests(${1:call}${2:, reply})" },
    Sig { name: "deg", params: &["degrees..."], doc: "Degrees of the key in force: `deg(1 b3 5' 1,)`; `'` an octave up, `,` an octave down, `b`/`#` a semitone. One slot, like `[ ]`.", snippet: "deg(${1:1 3 5})" },
    Sig { name: "stack", params: &["a", "b", "..."], doc: "Sections played together (an arrange or a `let` of sections).", snippet: "stack(${1:a}, ${2:b})" },
    Sig { name: "then", params: &["a", "b", "..."], doc: "Sections one after the other, as one.", snippet: "then(${1:a}, ${2:b})" },
    Sig { name: "knob", params: &["n"], doc: "The value of controller knob n, 0 to 1, read as it plays (performed input).", snippet: "knob(${1:1})" },
    Sig { name: "pad", params: &["n"], doc: "Controller pad n: 1 while held (performed input).", snippet: "pad(${1:1})" },
];

pub fn call(name: &str) -> Option<&'static Sig> {
    CALLS.iter().find(|s| s.name == name)
}

/// The parameters a part takes (`kick x ~ vel=90..110`), with their docs.
pub const PART_PARAMS: &[(&str, &str, &str)] = &[
    ("vel", "velocity, 1..127, or a range lo..hi the hits spread over", "vel=${1:80..110}"),
    ("oct", "the octave notes and degrees are played in", "oct=${1:2}"),
    ("len", "how many steps a note is held", "len=${1:2}"),
    ("inst", "the voice: a name with a sound of its own, or one with a family word (it plays that family's voice); `beatscript instruments` lists them", "inst=$1"),
    ("kit", "the drum kit of a drum part", "kit=$1"),
    ("seed", "the seed of the part's chances and generators", "seed=${1:1}"),
    ("sound", "the role's sound: own, synth, synth:NAME, inst:NAME", "sound=$1"),
    ("route", "where the part goes", "route=$1"),
    ("glide", "a tonal part's slide time in ms, 1..2000: a slide `^` bends the voice into the next note (without glide= a `^` only holds)", "glide=${1:60}"),
    ("decay", "a tonal part's note tail in ms, 10..10000", "decay=${1:300}"),
    ("pitch-drop", "a tonal part's drop in semitones, 1..24: each note starts that high and falls onto its pitch in 50 ms", "pitch-drop=${1:12}"),
];

/// The evolve operators (`melody: rotate 1 transpose <0 5>`), with what their number is.
pub const EVOLVE_OPS: &[(&str, &str)] = &[
    ("rotate", "steps (sixteenths) to move every hit along its bar, -15 to 15"),
    ("transpose", "semitones, -48 to 48 (a list `<0 5 7>` takes one per generation)"),
    ("octave", "octaves, -4 to 4"),
    ("invert", "the notes mirrored round the first"),
    ("reverse", "the bar backwards"),
    ("thin", "the share of hits dropped, 0 to 1"),
];

/// Statement keywords: (word, doc, snippet).
pub const STATEMENTS: &[(&str, &str, &str)] = &[
    ("tempo", "the tempo in BPM, or a ramp: `tempo 100 -> 130 over 8 bars exp`", "tempo ${1:124}"),
    ("key", "the key: `am`, `f#m`, `d:dorian`", "key ${1:am}"),
    ("swing", "the swing amount, 0 to 1", "swing ${1:0.1}"),
    ("let", "a name for a number, a word or a pattern", "let ${1:name} = ${2:value}"),
    ("def", "a macro with parameters (and ports `in:` / `out:`), called as `name(args)`", "def ${1:name}(${2:param}) {\n  $0\n}"),
    ("section", "a part of the song, N bars long, placed by `arrange`", "section ${1:name} ${2:16} bars {\n  $0\n}"),
    ("arrange", "the sections in order: `arrange intro build drop`; a block with mutations, `arrange { intro ~ acid rot 2 drop } loop 4 then outro end`; or one of several named ones, `arrange set: drop wubs loop`", "arrange ${1:intro}"),
    ("start", "the named arrangement that plays first: `start set` (default: the first written)", "start ${1:name}"),
    ("launch", "a pad brings a scene (a section, or a named arrangement) in on a bar line, the next phrase's by default: `launch drop on pad(2) next phrase 16`", "launch ${1:drop} on pad(${2:1})"),
    ("evolve", "the program (or the section) rewritten every N bars: generation n is the rules applied n times", "evolve every ${1:8} bars {\n  ${2:melody}: ${3:rotate 1}\n  stop when gen == ${4:8}\n}"),
    ("repeat", "the block N times, `$i` the copy's number (0..N-1), `every N bars` to shift each copy", "repeat ${1:4} {\n  $0\n}"),
    ("end", "where the program stops: `end`, `end at bar 64`, `end after outro`", "end"),
    ("loop", "the song starts again from its first section at its end", "loop"),
    ("import", "a saved snippet's lets and defs, by name or CID", "import ${1:name}"),
    ("sample", "a local sound file the program names: `sample harp from \"...\" license cc0 else pluck`", "sample ${1:name} from \"${2:path}\""),
    ("wire", "an output of one def glued to an input of another: `wire a.out -> b.in`", "wire ${1:a}.out -> ${2:b}.${3:in}"),
    ("assert", "a statement the program makes about itself: `assert length == 3m30s ± 5s`", "assert ${1:length} == ${2:3m30s}"),
    ("set", "programs of programs: `set { play X  mix 8 bars with crossfade  play Y }`", "set {\n  play ${1:a}\n  mix ${2:8} bars with ${3:crossfade}\n  play ${4:b}\n}"),
    ("station", "radio: an endless, deterministic run of tracks, position N always the same track: `station { from gen@2 techno seed=42  next seed=~  forms seeded passacaglia }`", "station {\n  from gen@2 ${1:techno} seed=${2:42}\n  next seed=${3:~}\n  forms ${4:seeded passacaglia rondo fugue}\n}"),
    ("use", "`use restricted`: the program is private and may use restricted assets", "use restricted"),
    ("do", "a call on the control registry: `do noun verb key=value`", "do ${1:noun}"),
    ("control", "the control deck, beside `deck A:`, `deck B:` and `master:`: engine calls only (`move`, `swap pairs`, `queue`, `crossfader`, a timed `tempo`, `do` and its sugar), sequenced like any line: `at bar 12: control: swap pairs with filter_exit`", "control: ${1:move echo_out on B}"),
    ("every", "a timed line: `every 4 bars: swap pairs`", "every ${1:4} bars: $0"),
    ("at", "a timed line: `at bar 16: master: filter off`", "at bar ${1:16}: $0"),
    ("on", "a timed line: `on next phrase: swap pairs`", "on next phrase: $0"),
    ("after", "a timed line: `after drop: master: filter off`", "after ${1:section}: $0"),
    ("before", "a timed line: `before drop 4 bars: ...`", "before ${1:section} ${2:4} bars: $0"),
];

pub fn statement(word: &str) -> Option<&'static (&'static str, &'static str, &'static str)> {
    STATEMENTS.iter().find(|s| s.0 == word)
}

/// Keys offered after `key`.
pub const KEYS: &[&str] = &[
    "c", "cm", "c#", "c#m", "d", "dm", "eb", "ebm", "e", "em", "f", "fm", "f#", "f#m", "g", "gm", "ab", "abm", "a", "am", "bb", "bbm", "b", "bm", "d:dorian", "e:phrygian", "f:lydian", "g:mixolydian", "a:aeolian", "b:locrian",
];

/// What each role is.
pub fn role_doc(role: &str) -> &'static str {
    match role {
        "kick" => "the kick drum part",
        "snare" => "the snare part",
        "hihat" => "the hi-hat part",
        "clap" => "the clap part",
        "bass" => "the bass line",
        "melody" => "the melody (lead) line",
        "harmony" => "chords held (a pad or keys)",
        "arp" => "an arpeggiated line",
        _ => "",
    }
}

/// What a macro of the catalogue is and does (completion and hover): its category, family and length.
pub fn macro_doc(e: &djlive::macros::MacroEntry) -> String {
    use djlive::macros::Length;
    let len = match e.length {
        Length::Bars(n) => format!("runs `over N bars` (1 to {}), {n} when the line says none", djlive::macros::MAX_BARS),
        Length::Sweep(n) => format!("a one-shot that rises over `over N bars`, {n} when the line says none"),
        Length::Held(n) => format!("a one-shot held `over N bars` and let go, {n} when the line says none"),
        Length::Own => "a one-shot that runs its own length: no `over`".to_string(),
    };
    let net = if e.needs_net { "; works a net deck's parts" } else { "" };
    format!("**{}** ({}, {}): {len}{net}. `macro {} on A`", e.label, e.category, e.family, e.id)
}

/// What a control word does on a `master:` or deck line.
pub fn action_doc(w: &str) -> &'static str {
    match w {
        "filter" => "the filter: `filter lp 2k`, `filter hp 300 -> 2k over 8 bars exp`, `filter off`",
        "delay" => "the delay send, 0 to 1",
        "reverb" => "the reverb send, 0 to 1",
        "crush" => "the bit crusher, 0 to 1",
        "drive" => "the drive (saturation), 0 (off) to 1: `drive 0.6`, `drive 0 -> 0.8 over 8 bars`",
        "resonance" => "the filter's resonance, 0 to 1",
        "level" => "the level, 0 to 1 (or a ramp / an lfo)",
        "low" | "mid" | "high" => "an EQ band, in dB",
        "crossfader" => "the crossfader, -1 (A) to 1 (B)",
        "swap" => "`swap pairs`: the decks change places",
        "move" => "a mix move: `move echo_out deck=A over 8 bars`",
        "load" => "load a track or a pair onto a deck",
        "queue" => "queue the next track",
        "play" | "stop" => "start or stop the deck",
        "sync" => "sync the deck's tempo",
        "mute" | "unmute" => "mute or unmute the deck",
        "duck" => "the sidechain: `duck B under A.kick 6db`",
        "mix" => "the mixer, set and put back at the end of the section: `mix { A level 0.7, B eq low -3 }`",
        "sing" => "typed lyrics sung on a part's notes: `sing \"words\" on melody voice=alto`",
        _ => "",
    }
}

// ---- the grammar (crates/djlive/grammar.ebnf): the words completion offers are read from it ------------------------

/// grammar.ebnf, read once.
pub fn grammar() -> &'static djlive::grammar::ebnf::Grammar {
    static G: std::sync::OnceLock<djlive::grammar::ebnf::Grammar> = std::sync::OnceLock::new();
    G.get_or_init(djlive::grammar::grammar)
}

/// The words a production of grammar.ebnf can begin with (its first terminals, through the productions it starts with).
pub fn first_words(rule: &str) -> std::collections::BTreeSet<String> {
    use djlive::grammar::ebnf::Node;
    fn first(g: &djlive::grammar::ebnf::Grammar, n: &Node, seen: &mut Vec<String>, out: &mut std::collections::BTreeSet<String>) -> bool {
        // true: the node can be empty (so what follows it can come first)
        match n {
            Node::Term(t) => {
                out.insert(t.clone());
                false
            }
            Node::Ref(r) => match g.rule(r) {
                Some(rule) if !seen.contains(r) => {
                    seen.push(r.clone());
                    let e = first(g, &rule.node, seen, out);
                    seen.pop();
                    e
                }
                _ => false,
            },
            Node::Opt(x) | Node::Rep(x) => {
                first(g, x, seen, out);
                true
            }
            Node::Choice(v) => v.iter().fold(false, |e, x| first(g, x, seen, out) || e),
            Node::Seq(v) => {
                for x in v {
                    if !first(g, x, seen, out) {
                        return false;
                    }
                }
                true
            }
        }
    }
    let g = grammar();
    let mut out = std::collections::BTreeSet::new();
    if let Some(r) = g.rule(rule) {
        first(g, &r.node, &mut vec![rule.to_string()], &mut out);
    }
    out
}

/// Every terminal of a production and the productions it uses (`block_end` -> end, loop, then).
pub fn words_of(rule: &str) -> std::collections::BTreeSet<String> {
    let g = grammar();
    let mut out = std::collections::BTreeSet::new();
    let mut todo = vec![rule.to_string()];
    let mut seen = std::collections::BTreeSet::new();
    while let Some(r) = todo.pop() {
        if !seen.insert(r.clone()) {
            continue;
        }
        if let Some(rule) = g.rule(&r) {
            rule.node.terms(&mut out);
            let mut refs = Vec::new();
            rule.node.refs(&mut refs);
            todo.extend(refs);
        }
    }
    out
}

/// docs/ARRANGE.md, for the arrange mutation operators' docs.
const ARRANGE_MD: &str = include_str!("../../../docs/ARRANGE.md");

/// The arrange mutation operators (`~ acid_a rot 2 | thin 0.1`), djlive's closed set (`mutate::operator_names`), each with
/// its signature and what it does, as docs/ARRANGE.md's tables say (`rot n` | every step `n` steps later ...).
pub fn mutation_ops() -> Vec<(&'static str, String, String)> {
    let mut docs: Vec<(String, String)> = Vec::new();
    for line in ARRANGE_MD.lines() {
        let cells: Vec<&str> = line.trim().trim_matches('|').split('|').map(str::trim).collect();
        if cells.len() < 2 || !cells[0].starts_with('`') {
            continue;
        }
        // `up n` / `down n`: one row for two
        for sig in cells[0].split(" / ") {
            let sig = sig.trim().trim_matches('`').to_string();
            docs.push((sig, cells[1].replace('`', "")));
        }
    }
    djlive::mutate::operator_names()
        .into_iter()
        .map(|n| match docs.iter().find(|(sig, _)| sig.split_whitespace().next() == Some(n)) {
            Some((sig, d)) => (n, sig.clone(), d.clone()),
            None => (n, n.to_string(), String::new()),
        })
        .collect()
}

/// How an arrangement ends (`arrange { ... } loop 4 then outro end`, `arrange set: drop wubs loop`): the words of grammar.ebnf's
/// `block_end`, with what each does.
pub const ENDINGS: &[(&str, &str, &str)] = &[
    ("end", "the song stops here (the default after the last arrangement)", "end"),
    ("loop", "for ever, each pass a new generation of the mutations (`loop N`: N passes, 1 to 256)", "loop"),
    ("then", "after the passes: the coda (`then outro end`), or the named arrangement to go on to", "then ${1:outro}"),
];
