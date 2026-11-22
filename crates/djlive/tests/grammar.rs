//! grammar.ebnf against the parser, both ways (docs/BTW.md "Grammar"): the grammar is well formed, every example
//! program is a sentence of it, and every sentence it writes is a program the parser reads. A mismatch fails the build.

use djlive::grammar::{self, recognize};

fn examples() -> Vec<(String, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    for dir in [root.join("../../examples/btw"), root.join("tests/fixtures")] {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "btw") {
                out.push((p.file_name().unwrap().to_string_lossy().to_string(), std::fs::read_to_string(&p).unwrap()));
            }
        }
    }
    out.sort();
    assert!(out.len() >= 20, "found only {} example programs", out.len());
    out
}

#[test]
fn the_grammar_is_well_formed() {
    let g = djlive::grammar::ebnf::read(grammar::SOURCE).unwrap_or_else(|e| panic!("{e}"));
    let p = g.problems();
    assert!(p.is_empty(), "grammar.ebnf:\n{}", p.join("\n"));
    let known: Vec<&str> = recognize::CLASSES.iter().map(|c| c.0).collect();
    for c in g.classes() {
        assert!(known.contains(&c.as_str()), "grammar.ebnf names the token class `{c}`, which the recognizer has no matcher for");
    }
}

#[test]
fn every_example_program_is_a_sentence_of_the_grammar() {
    let g = grammar::grammar();
    let mut bad = Vec::new();
    for (name, src) in examples() {
        // only a program the parser reads on its own is a question for the grammar (one that needs a host's registry
        // or snippets says so: `#! skip-test:`, as tests/examples_btw.rs reads it)
        if djlive::linked::header_lines(&src).iter().any(|h| h.key == "skip-test") {
            continue;
        }
        assert!(djlive::parse::parse(&src).ok(), "{name} does not parse");
        if let Err(m) = recognize::recognize(&g, &src) {
            let line = src.lines().nth(m.line - 1).unwrap_or("");
            bad.push(format!("{name}:{m}\n    {line}"));
        }
    }
    assert!(bad.is_empty(), "{} example programs are not sentences of grammar.ebnf:\n{}", bad.len(), bad.join("\n"));
}

/// `jux` is the grammar's `mut_op` read structurally: `jux rev`, `jux rot 3` and a chain around it are sentences of it, and the
/// parser reads each. What the grammar says wider than the language is said out loud: a bare `jux` is a `mut_op` (a NAME) and
/// the parser wants the operator the right copy gets.
#[test]
fn jux_is_a_sentence_of_the_grammar_and_the_parser_reads_it() {
    let g = grammar::grammar();
    let program = |chain: &str| format!("let p = a3 c4 e4 g4\nsection s 1 bars {{\n  deck B: melody p\n}}\narrange {{\n  s\n  ~ p {chain}\n  s\n}}\n");
    for chain in ["jux rev", "jux rot 3", "jux thin 0.2 | up 1", "rot 1 | jux rev | up 2", "jux morph p 0.5"] {
        let src = program(chain);
        assert!(djlive::parse::parse(&src).ok(), "`{chain}` does not parse");
        if let Err(m) = recognize::recognize(&g, &src) {
            panic!("`{chain}` is not a sentence of grammar.ebnf: {m}");
        }
    }
    let bare = program("jux");
    assert!(recognize::recognize(&g, &bare).is_ok(), "a bare `jux` is a mut_op to the grammar");
    assert!(!djlive::parse::parse(&bare).ok(), "and not to the parser");
}

/// `PHRASE` is `@` and an id of a phrasebook: the stock phrases and the public-domain lines. The recognizer and the checker
/// agree on every id of both, and on ones neither has.
#[test]
fn a_phrase_is_a_sentence_of_the_grammar_when_a_phrasebook_has_it() {
    let g = grammar::grammar();
    let ids = djengine::vocal::phoneme::phrase_ids();
    assert!(ids.iter().any(|i| i == "hands_up") && ids.iter().any(|i| i == "tempest_such_stuff"));
    for id in &ids {
        let src = format!("deck B: melody c4 e4\ndeck B: sing @{id} on melody\n");
        assert!(djlive::parse::parse(&src).ok(), "@{id} does not parse");
        recognize::recognize(&g, &src).unwrap_or_else(|m| panic!("@{id} is a phrase, and not a sentence of the grammar: {m}"));
    }
    for id in ["tempest_such_stuf", "hands_upp", "Tempest_such_stuff", "nope"] {
        let src = format!("deck B: melody c4 e4\ndeck B: sing @{id} on melody\n");
        assert!(!djlive::parse::parse(&src).ok(), "@{id} is not a phrase, and parses");
        assert!(recognize::recognize(&g, &src).is_err(), "@{id} is not a phrase, and is a sentence of the grammar");
    }
}

/// What the parser refuses that is not syntax, so no context-free grammar says it, each a diagnostic's first words. Keep
/// it short: every entry is a place the grammar is wider than the language, said out loud, and of one of five kinds:
/// - a consistency between the operands of a `then`: they make one part, of one role, cycle and size;
/// - a count that must agree with another: `choose` has a weight for each option;
/// - a seed's bars: from 1, and `a..b` with `a <= b`;
/// - the value arithmetic works out to (the grammar sees `(1 + 0.5)`, not 1.5): a chance is 0 to 1, a beat under 4,
///   a count or a degree whole;
/// - a name a `let` above gave a value: it is that value, so it cannot also name a sound.
const SEMANTIC: &[&str] = &[
    "`then` makes one",
    "`then` follows one part with another",
    "`then` follows a part with another part of its role",
    "this `choose` has",
    "a repeat count is a whole number",
    "a chance is 0 to 1",
    "a beat is 0 to 4",
    "a scale degree is a whole number",
    "a `let` name cannot name a sound",
    // the same kind: a `let` gave the name a number, so `start NAME`, `launch NAME` and a section name in `arrange NAME: { }` read one
    "`start` is not something to do",
    "expected a section or an arrangement to launch",
    "in `arrange { }`, expected a section name or a mutation, found a number",
    // a seed's bars count from 1, forwards (seed.rs)
    "a seed's bars",
    // the same kind as the `start` and `arrange { }` entries: a `let` gave the name a number, so a section named in `arrange` reads a number
    "expected a section, found a number",
    // `master` is a subject of `assert level`, and the one subject of a rule about notes cannot be (constraint.rs): the mix has no parts
    "`master` has no parts: it is the mix of both decks",
];

#[test]
fn every_sentence_of_the_grammar_is_a_program_the_parser_reads() {
    use djlive::grammar::generate::{Rng, Writer};
    let g = grammar::grammar();
    let mut w = Writer::new(&g);
    let mut bad: Vec<String> = Vec::new();
    let (mut tried, mut refused) = (0, 0);
    for depth in [8, 12, 16] {
        w.max_depth = depth;
        for seed in 0..1000u64 {
            let mut rng = Rng::new(seed * 31 + depth as u64);
            let src = w.sentence("program", &mut rng);
            tried += 1;
            let p = djlive::parse::parse(&src);
            if let Some(d) = p.diagnostics.iter().find(|d| d.is_error() && !SEMANTIC.iter().any(|m| d.message.starts_with(m))) {
                refused += 1;
                if bad.len() < 12 {
                    let line = src[..d.span.start.min(src.len())].matches('\n').count();
                    bad.push(format!("depth {depth} seed {seed}: {}\n    {}", d.message, src.lines().nth(line).unwrap_or("")));
                }
            }
        }
    }
    assert!(refused == 0, "{refused} of {tried} sentences of grammar.ebnf are refused by the parser:\n{}", bad.join("\n"));
    // and the walk wrote every production (but those only a host can read): nothing in the grammar goes untested
    let seen = w.visited.borrow().clone();
    let missed: Vec<String> = w.reachable("program").into_iter().filter(|n| !seen.contains(n)).collect();
    assert!(missed.is_empty(), "the sentences never used these productions: {missed:?}");
}

/// The text of a lyric is in the grammar too (`lyric_text`, read over characters: [`recognize::recognize_text`]), and holds to
/// what the checker does with it: which `/` is a phrase break, which opens an escape, and which lyrics are refused (6634).
/// What no grammar of text says is a lyric with no words in it (`/ /`): the checker's "there is nothing to sing".
fn lyric_ok(g: &djlive::grammar::ebnf::Grammar, lyric: &str) -> bool {
    recognize::recognize_text(g, "lyric_text", lyric).is_ok()
}

#[test]
fn a_lyric_is_a_sentence_of_lyric_text_exactly_when_the_checker_reads_it() {
    let g = grammar::grammar();
    // by hand: the record, the neighbours of its break, and the refused forms; and against the whole checker on a program
    let accepted = [
        "/h ii/ ! / /h ii/ !",
        "/ /h ii/",
        "/h ii/ / /h aa/",
        "/h ii/ /",
        "/ /h ii/ /",
        "/h ii/!/",
        "/h ii/!/ /h aa/",
        "/h ii//h aa/",
        "/h ii / /h aa/",
        "deep er in to night / hold me close / we move slow",
        "hello / world / again",
        "hello /\\nworld /\\n",
        "we /dh uh/ rave / and /dh uh/ rave",
        "/w ee - aa: r - dh uh - r ay! v/ _ /h ie - er/",
        "/ w ee /",
        "/h ii",
        "go /h ii",
        "go on /",
        "la",
        "jingle=/j ii ng g uh l/ all the way",
        "o'er=/oh r/ the hills, hey=/h ay:/",
        "road =/r oh d/",
    ];
    let refused = ["jingle=/zz/", "jingle=//", "hello // world", "/h ii/ / //", "//", "a /-/ b", "/h ii/ ! / /zz aa/", "/zz/", "a /s: aa/ b", "/h ii/ /zz/ /h aa/", "so /h aa::/"];
    let program = |lyric: &str| format!("tempo 120\ndeck B: melody c4 e4 g4 e4\ndeck B: sing \"{lyric}\" on melody voice=robot\n");
    for (lyrics, want) in [(&accepted[..], true), (&refused[..], false)] {
        for l in lyrics {
            // the lyric as the program writes it (`\n` is a line break the lexer reads), the text it comes to
            let text = l.replace("\\n", "\n");
            assert_eq!(lyric_ok(&g, &text), want, "lyric_text and {l:?}");
            let c = djlive::compile::check(&program(l));
            assert_eq!(c.ok(), want, "check and {l:?}: {}", djlive::diag::render_all(&c.diagnostics, &program(l)));
        }
    }
    // a lyric of breaks only is a sentence of the grammar of text and has no words to sing
    assert!(lyric_ok(&g, "/ /"));
    assert!(!djlive::compile::check(&program("/ /")).ok());
}

/// Every string of up to six of these pieces, against the checker's own reading of escapes (`check_escapes`, the first
/// thing a `sing` line does with its lyric): `lyric_text` takes the string if and only if the checker finds nothing to
/// refuse in it. The pieces are what a break and an escape are made of, with one symbol that is not in the set (`zz`) and
/// one mark (`:`) that a consonant cannot carry.
#[test]
fn lyric_text_and_the_checker_agree_on_every_short_lyric() {
    let g = grammar::grammar();
    let atoms = ["/", " ", "h", "ii", "-", "zz", ":"];
    let mut idx = [0usize; 6];
    let (mut ok, mut refused) = (0usize, 0usize);
    for len in 0..=6usize {
        idx.iter_mut().for_each(|i| *i = 0);
        loop {
            let t: String = idx[..len].iter().map(|i| atoms[*i]).collect();
            let checker = djengine::vocal::phoneme::check_escapes(&t).is_empty();
            assert_eq!(lyric_ok(&g, &t), checker, "lyric_text and the checker disagree on {t:?}");
            if checker {
                ok += 1;
            } else {
                refused += 1;
            }
            let mut k = len;
            while k > 0 && idx[k - 1] == atoms.len() - 1 {
                idx[k - 1] = 0;
                k -= 1;
            }
            if k == 0 {
                break;
            }
            idx[k - 1] += 1;
        }
    }
    eprintln!("{ok} read, {refused} refused");
    assert!(ok > 10_000 && refused > 10_000, "{ok} read, {refused} refused");
}

/// The lyrics of the example programs are sentences of `lyric_text` (the text of each `sing "..."`).
#[test]
fn every_example_lyric_is_a_sentence_of_lyric_text() {
    let g = grammar::grammar();
    let mut seen = 0;
    for (name, src) in examples() {
        let mut rest = src.as_str();
        while let Some(i) = rest.find("sing \"") {
            rest = &rest[i + 6..];
            let mut text = String::new();
            let mut chars = rest.chars();
            while let Some(c) = chars.next() {
                match c {
                    '"' | '\n' => break,
                    '\\' => match chars.next() {
                        Some('n') => text.push('\n'),
                        Some(o) => text.push(o),
                        None => break,
                    },
                    c => text.push(c),
                }
            }
            assert!(lyric_ok(&g, &text), "{name}: the lyric {text:?} is not a sentence of lyric_text");
            seen += 1;
        }
    }
    assert!(seen >= 5, "found only {seen} lyrics in the examples");
}

/// `~3` / `@4` (`mutation = ( "~" | "@" ) [ INT ] mut_target ...`): the grammar reads a count after the mark and nowhere else,
/// the sentences it writes use one, and the parser reads every such sentence (its count in 1 to 256: the writer's INTs are
/// 2, 4 and 8).
#[test]
fn a_counted_mutation_is_in_the_grammar_and_only_where_the_grammar_puts_it() {
    use djlive::grammar::generate::{Rng, Writer};
    let g = grammar::grammar();
    let prog = |line: &str| format!("let a = x ~ x ~\nsection s 1 bars {{\n  deck A: kick a\n}}\narrange {{\n  s\n  {line}\n}} loop 4 then end\n");
    for line in ["~3 a rev", "~ 3 a rev", "@4 a double", "~ a rev", "~2 a rev | thin 0.5, a up 1"] {
        let src = prog(line);
        assert!(recognize::recognize(&g, &src).is_ok(), "`{line}` is a sentence");
        assert!(djlive::parse::parse(&src).ok(), "`{line}` parses");
    }
    // a count is for the line: after the mark, before the first target
    for line in ["~ a rev, 3 a up 1", "~ a 3 rev", "~3", "~-3 a rev"] {
        let src = prog(line);
        assert!(recognize::recognize(&g, &src).is_err(), "`{line}` is not a sentence");
        assert!(!djlive::parse::parse(&src).ok(), "`{line}` does not parse");
    }
    let mut w = Writer::new(&g);
    w.max_depth = 12;
    let counted = (0..1500u64)
        .map(|seed| w.sentence("program", &mut Rng::new(seed * 31 + 12)))
        .filter(|src| src.lines().any(|l| matches!(l.trim_start().strip_prefix(['~', '@']), Some(r) if r.trim_start().starts_with(|c: char| c.is_ascii_digit()))))
        .inspect(|src| assert!(djlive::parse::parse(src).diagnostics.iter().all(|d| !d.is_error() || SEMANTIC.iter().any(|m| d.message.starts_with(m))), "{src}"))
        .count();
    assert!(counted > 0, "the sentences of the grammar never write a counted mutation");
}

/// A reference to a stored program is a sentence of the grammar, and the parser reads it (it needs a host to be *checked*: the
/// sentence writer of the drift test leaves `stored` out, `[host]`; tests/stored_ref.rs checks them against a store).
#[test]
fn a_reference_to_a_stored_program_is_a_sentence_the_parser_reads() {
    let g = grammar::grammar();
    let cid = "z4EBG9j9cjqaYU5uVNFH6YKAMoiyPfNkof87KEXjPNPpAhJzDhi";
    let sentences = [
        format!("section hook from {cid}.drop\narrange hook\n"),
        format!("section bassline from {cid}.drop.bass\narrange bassline\n"),
        format!("deck A: kick x ~ x ~ | bass {cid}.drop inst=hoover\n"),
        format!("deck B: melody {cid}.drop.melody vel=90\n"),
        format!("deck A: bass {cid}\n"),
        format!("let riff = {cid}.drop.bass\ndeck A: bass riff\n"),
        format!("let riff = {cid}.bass\n"),
    ];
    for src in &sentences {
        assert!(djlive::parse::parse(src).ok(), "the parser refuses {src:?}: {:?}", djlive::parse::parse(src).diagnostics.iter().map(|d| d.message.clone()).collect::<Vec<_>>());
        recognize::recognize(&g, src).unwrap_or_else(|m| panic!("not a sentence of the grammar: {src:?}: {m}"));
    }
    // a whole program is a fork, not an arrangement: neither the grammar nor the parser takes `arrange from <cid>`
    let whole = format!("arrange from {cid}.drop\n");
    assert!(recognize::recognize(&g, &whole).is_err());
    assert!(!djlive::parse::parse(&whole).ok());
    // and a CID is a name no `let`, `def` or section may have
    assert!(!djlive::check(&format!("let {cid} = c2 e2\n")).diagnostics.iter().all(|d| !d.is_error()));
}

/// The parameters of a part are named in the grammar, so the Grammar page lists every one: `pname` is the parser's
/// `parse::PARAMS` (`glide=`, `decay=`, `pitch-drop=` and the rest), no more and no fewer.
#[test]
fn the_grammar_names_every_parameter_of_a_part() {
    let g = grammar::grammar();
    let r = g.rule("pname").expect("grammar.ebnf has `pname`, the parameters of a part");
    let mut named = std::collections::BTreeSet::new();
    r.node.terms(&mut named);
    let want: std::collections::BTreeSet<String> = djlive::parse::PARAMS.iter().map(|p| p.to_string()).collect();
    assert_eq!(named, want, "grammar.ebnf's `pname` is not parse::PARAMS");
}

/// `drive` is a word of the control grammar and no reserved word: the grammar writes it after a target and the parser reads it
/// there, ramped and swung as the other effects are, while a section (or a let, a def) may still be named `drive` (the pinned
/// example `matrix-130` has `section drive`), so it is in `action` and in neither `parse::ACTION_WORDS` nor the reserved names.
#[test]
fn drive_is_in_the_grammars_action_and_is_no_reserved_word() {
    let g = grammar::grammar();
    let mut named = std::collections::BTreeSet::new();
    g.rule("action").expect("grammar.ebnf has `action`").node.terms(&mut named);
    for w in ["delay", "reverb", "crush", "drive", "resonance"] {
        assert!(named.contains(w), "grammar.ebnf's `action` has no `{w}`");
    }
    assert!(!djlive::parse::ACTION_WORDS.contains(&"drive") && djlive::parse::reserved("drive").is_none(), "`drive` must stay a free name");
    for src in [
        "deck A: kick x ~ x ~\nmaster: drive 0.6\n",
        "deck A: kick x ~ x ~\ndeck A: drive 0\n",
        "deck A: kick x ~ x ~\nmaster: drive 0.1 -> 0.9 over 4 bars exp\n",
        "deck A: kick x ~ x ~\ndeck B: drive lfo sine 2 bars center=0.5 depth=0.3\n",
        "deck A: kick x ~ x ~\nat bar 4 for 2 bars: master: drive 0.8\n",
        // the name: a section, a let and a def called `drive`, said where it is read as a name
        "section drive 2 bars {\n  deck A: kick x ~ x ~\n  master: drive 0.5\n}\narrange drive\n",
        "let drive = 0.5\nlet kicks = x ~ x ~\ndeck A: kick kicks\nmaster: drive drive\n",
    ] {
        assert!(djlive::parse::parse(src).ok(), "the parser refuses {src:?}: {:?}", djlive::parse::parse(src).diagnostics);
        assert!(recognize::recognize(&g, src).is_ok(), "{src:?} is not a sentence of the grammar: {:?}", recognize::recognize(&g, src));
    }
}

/// A Roman numeral is an `arg` where a chord symbol is: `chord(V7)`, `arp(bVII, up, 8)`, `arp(V7/7, down)` are sentences of
/// grammar.ebnf, and the parser reads and checks each (the numeral is read in the key in force).
#[test]
fn a_roman_numeral_is_a_sentence_where_a_chord_symbol_is() {
    let g = grammar::grammar();
    for call in ["chord(V7)", "chord(ii\u{b0}, 3)", "arp(i, up, 8)", "arp(bVII, updown)", "arp(#iv, down, 4)", "arp(V7/7, down, 4)", "chord(I/3)"] {
        let src = format!("tempo 120\nkey am\ndeck B: harmony {call}\n");
        assert!(recognize::recognize(&g, &src).is_ok(), "{call} is not a sentence of grammar.ebnf");
        let c = djlive::compile::check(&src);
        assert!(c.compiled.is_some(), "{call}:\n{}", djlive::diag::render_all(&c.diagnostics, &src));
    }
}

/// The page as a file, to look at (`BTW_GRAMMAR_PAGE=/path/to/page.html cargo test -p djlive --test grammar write_page`).
#[test]
fn write_page() {
    if let Ok(p) = std::env::var("BTW_GRAMMAR_PAGE") {
        std::fs::write(p, grammar::page::grammar_page("")).unwrap();
    }
}

/// docs/BTW.md's grammar block is grammar.ebnf, generated: a production added to the grammar and not to the docs fails
/// here (`BTW_BLESS=1 cargo test -p djlive --test grammar` writes it).
#[test]
fn the_docs_grammar_is_generated_from_grammar_ebnf() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/BTW.md");
    let md = std::fs::read_to_string(&path).unwrap();
    let a = md.find(grammar::DOCS_BEGIN).expect("BTW.md has the grammar.ebnf begin marker") + grammar::DOCS_BEGIN.len();
    let b = a + md[a..].find(grammar::DOCS_END).expect("BTW.md has the grammar.ebnf end marker");
    let want = grammar::docs_block();
    if md[a..b] != want {
        if std::env::var("BTW_BLESS").is_ok() {
            std::fs::write(&path, format!("{}{}{}", &md[..a], want, &md[b..])).unwrap();
            return;
        }
        panic!("docs/BTW.md's grammar block is not grammar.ebnf: run BTW_BLESS=1 cargo test -p djlive --test grammar");
    }
}

/// Golden SVGs: a few productions' diagrams, byte for byte (tests/fixtures/railroad/). A choice (`mark`), optionals and a
/// repetition (`term`), a sequence with a class (`dur`), a longer sequence (`set_mix`). A change to the layout shows
/// here; `BTW_BLESS=1` writes the new ones, to be looked at before they are committed.
#[test]
fn railroad_diagrams_match_their_goldens() {
    let g = grammar::grammar();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/railroad");
    let bless = std::env::var("BTW_BLESS").is_ok();
    std::fs::create_dir_all(&dir).unwrap();
    for name in ["mark", "term", "dur", "set_mix"] {
        let r = g.rule(name).unwrap_or_else(|| panic!("no production `{name}`"));
        let got = grammar::railroad::svg(name, &r.node, &g);
        let path = dir.join(format!("{name}.svg"));
        if bless {
            std::fs::write(&path, &got).unwrap();
            continue;
        }
        let want = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("no golden {} (BTW_BLESS=1 writes it)", path.display()));
        assert_eq!(got, want, "the diagram of `{name}` changed (BTW_BLESS=1 rewrites tests/fixtures/railroad/{name}.svg)");
    }
}
