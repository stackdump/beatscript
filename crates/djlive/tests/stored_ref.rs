//! `<cid>.drop`: a stored program's music as a reference (stored.rs, docs/BEATSCRIPT.md "Borrow from a stored program"):
//! - a borrowed section / part plays as its literal, so a borrowed drop has the source's own net CIDs;
//! - the program's CID is of the text as written (the reference holds the CID), whether or not the source can be found;
//! - the link carries the source, and opens where nothing is saved;
//! - the errors and warnings, each on the reference with what would fix it.

use djlive::compile::Compiled;
use djlive::env::{Catalog, Env, StaticSnippets};
use djlive::share;

/// hand-dubstep-140 v2: a whole track (CC0, pinned: tests/cid-pins.json).
const V2: &str = include_str!("../../../docs/beatscript/hand-dubstep-140-v2.beatscript");
const V2_CID: &str = "z4EBG9jEj3ephVYiwBA3bmUnp5GmMQ8hjtdgFSbrfgKHp3cJfef";
/// The net CIDs of its `drop` (cid-pins.json: `drop@16:A`, `drop@16:B`).
const V2_DROP_A: &str = "z4EBG9j8BDvgUEjm15dXsf4xLcYpZ15bpPh1xk49t59eL5kwm92";
const V2_DROP_B: &str = "z4EBG9j6xC7XhPsRu89PCeg1kDJ36o19QZ4Jk6cQNa6CKK5hokY";

/// A program with no sections: a kick, a bass and a melody on two decks, one part with chance.
const FIRST: &str = "tempo 124\nkey am\ndeck A: kick x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ vel=100 | bass a1 ~ ~ a1 ~ ~ e2 ~ vel=90 oct=2 inst=sub-bass\ndeck B: melody a4 ~ c5 ~ e5 ~ ~ ~ c5 ~ a4 ~ g4 ~ ~ ~ inst=warm-pad | hihat x*8?0.5 vel=60..80\n";

/// The snippets: programs by CID, each checked against the ones before it (a program may borrow from an earlier one).
fn store(progs: &[&str]) -> StaticSnippets {
    let mut s = StaticSnippets::default();
    for (i, text) in progs.iter().enumerate() {
        let c = check(text, &s);
        let comp = c.compiled.unwrap_or_else(|| panic!("program {i} does not compile: {:?}", c.diagnostics.iter().map(|d| d.message.clone()).collect::<Vec<_>>()));
        s.items.push((format!("p{i}"), comp.cid, comp.text));
    }
    s
}

fn check(text: &str, s: &StaticSnippets) -> djlive::Check {
    djlive::compile::check_with(text, &Env { snippets: Some(s), ..Env::default() })
}

fn ok(text: &str, s: &StaticSnippets) -> Compiled {
    let c = check(text, s);
    c.compiled.unwrap_or_else(|| panic!("{text}\n{:?}", c.diagnostics.iter().map(|d| format!("{}: {}", d.span.start, d.message)).collect::<Vec<_>>()))
}

fn errors(text: &str, s: &StaticSnippets) -> Vec<(String, Option<String>)> {
    check(text, s).diagnostics.iter().filter(|d| d.is_error()).map(|d| (d.message.clone(), d.help.clone())).collect()
}

fn first_error(text: &str, s: &StaticSnippets) -> (String, String) {
    let e = errors(text, s);
    assert!(!e.is_empty(), "no error for: {text}");
    (e[0].0.clone(), e[0].1.clone().unwrap_or_default())
}

fn warnings(text: &str, s: &StaticSnippets) -> Vec<(String, Option<String>)> {
    check(text, s).diagnostics.iter().filter(|d| !d.is_error()).map(|d| (d.message.clone(), d.help.clone())).collect()
}

fn v2() -> StaticSnippets {
    let s = store(&[V2]);
    assert_eq!(s.items[0].1, V2_CID, "the source is the pinned program");
    s
}

fn deck_net(c: &Compiled, seg: usize, d: djlive::ast::Deck) -> String {
    c.segments[seg].decks[&d].net_cid()
}

#[test]
fn a_borrowed_drop_has_the_source_s_own_nets() {
    let s = v2();
    // tempo and key are in a net's CID, so the borrower says the source's: then the drop is the same music, net for net
    let c = ok(&format!("tempo 140\nkey fm\nsection hook from {V2_CID}.drop\narrange hook\n"), &s);
    assert_eq!(c.segments.len(), 1);
    assert_eq!((c.segments[0].name.as_str(), c.segments[0].bars), ("hook", 16));
    assert_eq!(deck_net(&c, 0, djlive::ast::Deck::A), V2_DROP_A);
    assert_eq!(deck_net(&c, 0, djlive::ast::Deck::B), V2_DROP_B);
    // the canonical text holds the reference as written, and the program's CID is of that text
    assert!(c.text.contains(&format!("section hook from {V2_CID}.drop\n")), "{}", c.text);
    assert_eq!(c.cid, djlive::program_cid(&c.text));
    // and it is the literal's music: expand writes the section out, and that plays the same
    let expanded = djlive::stored::expand(&format!("tempo 140\nkey fm\nsection hook from {V2_CID}.drop\narrange hook\n"), &Env { snippets: Some(&s), ..Env::default() });
    let (e, uses) = expanded.expect("a reference").expect("expands");
    assert!(e.text.contains("# from ") && e.text.contains("section hook 16 bars {"), "{}", e.text);
    assert_eq!(uses, vec![(V2_CID.to_string(), s.items[0].2.clone())]);
    let literal = ok(&e.text, &StaticSnippets::default());
    assert_eq!(deck_net(&literal, 0, djlive::ast::Deck::A), V2_DROP_A);
    assert_ne!(literal.cid, c.cid, "the literal is another program: it has no reference in it");
}

#[test]
fn a_borrowed_section_says_which_control_lines_stayed_behind() {
    let s = v2();
    let w = warnings(&format!("tempo 140\nkey fm\nsection hook from {V2_CID}.drop\narrange hook\n"), &s);
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].0.contains("section `drop` of ") && w[0].0.contains("has control lines (master: filter, master: reverb) that a borrowed section does not bring"), "{}", w[0].0);
    assert_eq!(w[0].1.as_deref(), Some("write the ones you want in this section, for example: at bar 5: master: reverb 0.15"));
    // a section with none says nothing: the intro's one control line is a filter, so look at a source that has no controls
    let quiet = store(&["tempo 100\nsection verse 2 bars {\n  deck A: kick x ~ x ~\n}\narrange verse\n"]);
    let c = ok(&format!("tempo 100\nsection chorus from {}.verse\narrange chorus\n", quiet.items[0].1), &quiet);
    assert!(c.warnings.is_empty(), "{:?}", c.warnings.iter().map(|d| d.message.clone()).collect::<Vec<_>>());
}

#[test]
fn a_borrowed_section_brings_its_key_when_it_sounds_in_another() {
    let s = v2();
    let own_key = |k: &str| format!("tempo 140\nkey {k}\nsection hook from {V2_CID}.drop\narrange hook\n");
    // in its own key it writes none; in another it says so, and the music is the source's
    let same = djlive::stored::expand(&own_key("fm"), &Env { snippets: Some(&s), ..Env::default() }).unwrap().unwrap().0.text;
    assert!(!same.contains("  key "), "{same}");
    let other = djlive::stored::expand(&own_key("am"), &Env { snippets: Some(&s), ..Env::default() }).unwrap().unwrap().0.text;
    assert!(other.contains("section hook 16 bars {\n  key fm\n"), "{other}");
    let c = ok(&own_key("am"), &s);
    assert_eq!(deck_net(&c, 0, djlive::ast::Deck::A), V2_DROP_A, "the key rode along: the same net");
    assert_eq!(c.section_keys.get("hook").map(|k| k.text()), Some("fm".to_string()));
}

#[test]
fn a_borrowed_part_is_its_literal_and_the_lines_own_params_win() {
    let s = v2();
    // the drop's bass is `<wob_a wob_b> inst=wobble-lead`
    let borrowed = ok(&format!("tempo 140\nkey fm\ndeck A: kick x ~ x ~ | bass {V2_CID}.drop inst=hoover\n"), &s);
    let literal = ok(
        "tempo 140\nkey fm\ndeck A: kick x ~ x ~ | bass <[f1 ~ ~ f1 ~ ~ ab1 ~ f1 ~ ~ ~ eb1 ~ c1 ~] [f1 ~ f1 ~ ~ ~ db1 ~ ~ ~ c1 ~ eb1 ~ eb1 ~]> inst=hoover\n",
        &StaticSnippets::default(),
    );
    assert_eq!(borrowed.decks[&djlive::ast::Deck::A].net_cid(), literal.decks[&djlive::ast::Deck::A].net_cid());
    // with no params of its own the part's own come along (its voice)
    let b2 = ok(&format!("tempo 140\nkey fm\ndeck A: bass {V2_CID}.drop\n"), &s);
    let l2 = ok(
        "tempo 140\nkey fm\ndeck A: bass <[f1 ~ ~ f1 ~ ~ ab1 ~ f1 ~ ~ ~ eb1 ~ c1 ~] [f1 ~ f1 ~ ~ ~ db1 ~ ~ ~ c1 ~ eb1 ~ eb1 ~]> inst=wobble-lead\n",
        &StaticSnippets::default(),
    );
    assert_eq!(b2.decks[&djlive::ast::Deck::A].net_cid(), l2.decks[&djlive::ast::Deck::A].net_cid());
    // the canonical text keeps the reference, params and all
    assert!(borrowed.text.contains(&format!("bass {V2_CID}.drop inst=hoover")), "{}", borrowed.text);
    // another part of the drop, on this line's role: the pattern of the drop's melody on a bass
    let m = ok(&format!("tempo 140\nkey fm\ndeck B: bass {V2_CID}.drop.melody\n"), &s);
    let lm = ok("tempo 140\nkey fm\ndeck B: bass <[~ ~ c4 ~ ab3 ~ ~ ~ f3 ~ ~ g3 ~ ab3 ~ ~] [~ ~ c4 ~ eb4 ~ ~ ~ db4 ~ c4 ~ ab3 ~ ~ ~]> inst=pwm-lead\n", &StaticSnippets::default());
    assert_eq!(m.decks[&djlive::ast::Deck::B].net_cid(), lm.decks[&djlive::ast::Deck::B].net_cid());
}

#[test]
fn a_let_of_a_stored_part_is_its_pattern_and_names_one_part() {
    let s = v2();
    let via_let = ok(&format!("tempo 140\nkey fm\nlet riff = {V2_CID}.drop.bass\ndeck A: bass riff inst=wobble-lead\n"), &s);
    let direct = ok(&format!("tempo 140\nkey fm\ndeck A: bass {V2_CID}.drop\n"), &s);
    assert_eq!(via_let.decks[&djlive::ast::Deck::A].net_cid(), direct.decks[&djlive::ast::Deck::A].net_cid());
    let (m, h) = first_error(&format!("let riff = {V2_CID}.drop\ndeck A: bass riff\n"), &s);
    assert_eq!(m, "a `let` of a stored program names one part: say which");
    assert_eq!(h, format!("let riff = {V2_CID}.drop.bass"));
}

#[test]
fn a_program_with_no_sections_is_borrowed_by_its_bare_cid_and_one_with_sections_is_not() {
    let s = store(&[FIRST, V2]);
    let (first, v2) = (s.items[0].1.clone(), s.items[1].1.clone());
    let c = ok(&format!("tempo 124\nkey am\ndeck A: kick {first} | bass {first}\n"), &s);
    let lit = ok("tempo 124\nkey am\ndeck A: kick x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ x ~ ~ ~ vel=100 | bass a1 ~ ~ a1 ~ ~ e2 ~ oct=2 vel=90 inst=sub-bass\n", &StaticSnippets::default());
    assert_eq!(c.decks[&djlive::ast::Deck::A].net_cid(), lit.decks[&djlive::ast::Deck::A].net_cid());
    // the part on another deck than it was: the same hits (a bass is a bass wherever it is put)
    let b = ok(&format!("tempo 124\nkey am\ndeck B: bass {first}\n"), &s);
    assert_eq!(b.decks[&djlive::ast::Deck::B].parts[0].cycles, lit.decks[&djlive::ast::Deck::A].parts[1].cycles);
    // a bare CID of a program that has sections names none
    let (m, h) = first_error(&format!("deck A: kick {v2}\n"), &s);
    assert!(m.starts_with("a bare ") && m.contains("is a program with no sections; this one has intro, build, drop, breakdown, drop2, outro"), "{m}");
    assert_eq!(h, format!("name one: {v2}.intro"));
    // a section of one that has none
    let (m, h) = first_error(&format!("section x from {first}.drop\narrange x\n"), &s);
    assert!(m.ends_with("has no section `drop`"), "{m}");
    assert!(h.contains("it has no sections: borrow a part of it, `bass "), "{h}");
    // and a section line with no section in it
    let (m, _) = first_error(&format!("section x from {first}\narrange x\n"), &s);
    assert!(m.contains("needs one of its sections"), "{m}");
}

#[test]
fn a_part_with_chance_plays_the_same_hits_on_any_deck() {
    let s = store(&[FIRST]);
    let first = s.items[0].1.clone();
    // in the source the hats are on deck B: borrowed onto deck A they draw their dice from the source's seed, not this deck's
    let src = ok(FIRST, &StaticSnippets::default());
    let src_hats = src.decks[&djlive::ast::Deck::B].parts.iter().find(|p| p.role == "hihat").unwrap();
    assert!(src_hats.chance && src_hats.seed.is_some());
    let c = ok(&format!("tempo 124\nkey am\ndeck A: hihat {first}\n"), &s);
    let hats = &c.decks[&djlive::ast::Deck::A].parts[0];
    // (a velocity range, `vel=60..80`, is drawn from the deck as well: on another deck the same hits have other velocities)
    let hits = |p: &djlive::compile::PartIr| p.cycles.iter().map(|c| c.iter().map(|h| (h.step, h.note)).collect::<Vec<_>>()).collect::<Vec<_>>();
    assert_eq!(hits(hats), hits(src_hats), "the same dice");
    assert!(hits(hats).iter().any(|c| c.len() < 8), "(and they are dice: not every hit plays)");
    assert!(c.text.contains(&format!("hihat {first}")), "{}", c.text);
    // as a let there is no seed to carry: it is said
    let w = warnings(&format!("let hh = {first}.hihat\ndeck A: hihat hh\n"), &s);
    assert!(w.iter().any(|(m, _)| m.contains("has chance in it: as a `let` it draws its own dice")), "{w:?}");
}

#[test]
fn what_cannot_be_borrowed_is_said_on_the_reference_with_what_would_fix_it() {
    let s = v2();
    let r = |t: &str| first_error(&t.replace("CID", V2_CID), &s);
    // a section that is not there, with the nearest and what there is
    let (m, h) = r("section x from CID.drops\narrange x\n");
    assert!(m.ends_with("has no section `drops`"), "{m}");
    assert_eq!(h, "did you mean `drop`? Its sections: intro, build, drop, breakdown, drop2, outro");
    // a part the section does not play
    let (m, h) = r("deck A: bass CID.intro\n");
    assert!(m.starts_with("`intro` of ") && m.ends_with("has no `bass`; it plays: kick, hihat, harmony"), "{m}");
    assert_eq!(h, "a part of it: `.kick`");
    // a role on two decks is not one part
    let two = store(&["tempo 100\nsection s 2 bars {\n  deck A: kick x ~ x ~\n  deck B: kick x x ~ ~\n}\narrange s\n"]);
    let (m, h) = first_error(&format!("deck A: kick {}.s\n", two.items[0].1), &two);
    assert!(m.ends_with("is on deck A and deck B in `s` of ") || m.contains("is on deck A and deck B"), "{m}");
    assert!(h.starts_with("borrow the section: section hook from "), "{h}");
    // a part comes after the section, and a part is a role
    let (m, _) = r("deck A: bass CID.bass.drop\n");
    assert!(m.contains("a section comes before a part"), "{m}");
    let (m, h) = r("deck A: bass CID.drop.basss\n");
    assert!(m.starts_with("`.basss`: a part of a stored program is one of its roles"), "{m}");
    assert!(h.contains("did you mean `.bass`?"), "{h}");
    let (m, _) = r("deck A: bass CID.drop.bass.x\n");
    assert!(m.contains("a CID, a section and a part at most"), "{m}");
    // a part has no cycle of its own, and a stored program is not a whole arrangement
    let (m, _) = r("deck A: hihat/12 CID.drop\n");
    assert!(m.contains("takes no cycle of its own"), "{m}");
    let p = djlive::parse::parse(&format!("arrange from {V2_CID}.drop\n"));
    let d = p.diagnostics.iter().find(|d| d.is_error()).expect("arrange from takes a seed");
    assert!(d.message.starts_with("`arrange from` takes a seed of the generator") && d.help.as_deref().is_some_and(|h| h.contains("section hook from <cid>.drop")), "{}", d.message);
    // nowhere a reference goes
    let (m, _) = r("tempo 120\nsection x 2 bars {\n  deck A: kick x | bass x\n}\narrange x CID.drop\n");
    assert!(!m.is_empty());
    // a section written and never played
    let unplayed = store(&["tempo 100\nsection verse 2 bars {\n  deck A: kick x ~ x ~\n}\nsection bridge 2 bars {\n  deck A: snare x ~ x ~\n}\narrange verse\n"]);
    let (m, h) = first_error(&format!("section x from {}.bridge\narrange x\n", unplayed.items[0].1), &unplayed);
    assert!(m.contains("section `bridge` of ") && m.ends_with("is written but never played"), "{m}");
    assert_eq!(h, "its sections that play: verse");
}

#[test]
fn a_program_the_host_does_not_have_is_not_here_on_the_reference() {
    let none = StaticSnippets::default();
    let text = format!("tempo 140\nkey fm\nsection hook from {V2_CID}.drop\narrange hook\n");
    let c = check(&text, &none);
    assert!(c.compiled.is_none());
    let d = c.diagnostics.iter().find(|d| d.is_error()).unwrap();
    assert!(djlive::env::is_not_here(&d.message), "{}", d.message);
    assert_eq!(d.help.as_deref(), Some(djlive::env::NOT_HERE_HELP));
    // on the reference's own line, from the CID to the end of the section's name
    assert_eq!(&text[d.span.start..d.span.end], format!("{V2_CID}.drop"));
    // with no snippets at all it is the same words
    let c = djlive::compile::check_with(&text, &Env::none());
    assert!(djlive::env::is_not_here(&c.diagnostics.iter().find(|d| d.is_error()).unwrap().message));
    // the text is the program, found or not: the CID does not depend on it
    let parsed = djlive::parse::parse(&text);
    assert!(parsed.ok(), "{:?}", parsed.diagnostics.iter().map(|d| d.message.clone()).collect::<Vec<_>>());
    let canonical = djlive::print::canonical(&parsed.program);
    assert_eq!(ok(&text, &v2()).cid, djlive::program_cid(&canonical));
    // a host that gives a text that is not the program it names is refused
    let mut liar = StaticSnippets::default();
    liar.items.push(("x".into(), V2_CID.to_string(), "tempo 100\ndeck A: kick x\n".into()));
    let (m, _) = first_error(&text, &liar);
    assert!(m.contains("is not the program it names: refused"), "{m}");
}

#[test]
fn the_link_carries_what_a_part_is_borrowed_from_and_opens_where_nothing_is_saved() {
    let s = v2();
    let c = ok(&format!("tempo 140\nkey fm\nsection hook from {V2_CID}.drop\narrange hook\n"), &s);
    assert_eq!(c.plays.iter().map(|(c, _)| c.as_str()).collect::<Vec<_>>(), [V2_CID]);
    let (url, carried) = share::encode_with_tracks(&c, None, false);
    assert!(carried);
    for env in [Env { snippets: Some(&StaticSnippets::default()), ..Env::default() }, Env::none()] {
        let o = share::open(&url, &env).expect("it opens with nothing saved");
        assert_eq!(o.cid, c.cid);
        let opened = o.check.compiled.expect("and checks");
        assert_eq!(deck_net(&opened, 0, djlive::ast::Deck::A), V2_DROP_A);
    }
    // without what it carried, it is not here
    let link = share::parse_link(&url).unwrap();
    assert_eq!(link.tracks.len(), 1);
}

#[test]
fn what_a_borrowed_program_borrows_from_travels_too_and_a_chain_has_a_depth() {
    // a borrows from nothing, b from a, c from b
    let a = "tempo 100\nsection s 2 bars {\n  deck A: kick x ~ x ~ | bass c2 ~ e2 ~\n}\narrange s\n";
    let mut s = store(&[a]);
    let b = format!("tempo 100\nsection t from {}.s\narrange t\n", s.items[0].1);
    let cb = ok(&b, &s);
    s.items.push(("b".into(), cb.cid.clone(), cb.text.clone()));
    let c = format!("tempo 100\nsection u from {}.t\narrange u\n", cb.cid);
    let cc = ok(&c, &s);
    // leaves first: a, then b, which borrows from it
    assert_eq!(cc.plays.iter().map(|(x, _)| x.as_str()).collect::<Vec<_>>(), [s.items[0].1.as_str(), cb.cid.as_str()]);
    let (url, carried) = share::encode_with_tracks(&cc, None, false);
    assert!(carried);
    assert!(share::open(&url, &Env::none()).is_ok(), "the link has all the chain");
    // the music is the first one's, through both
    assert_eq!(deck_net(&cc, 0, djlive::ast::Deck::A), deck_net(&ok(a, &StaticSnippets::default()), 0, djlive::ast::Deck::A));
    // five deep is too deep
    let mut prev = cc;
    for i in 0..4 {
        s.items.push((format!("n{i}"), prev.cid.clone(), prev.text.clone()));
        let next = format!("tempo 100\nsection v{i} from {}.{}\narrange v{i}\n", prev.cid, if i == 0 { "u".to_string() } else { format!("v{}", i - 1) });
        let r = check(&next, &s);
        match r.compiled {
            Some(c) => prev = c,
            None => {
                let m = r.diagnostics.iter().find(|d| d.is_error()).unwrap().message.clone();
                assert!(m.contains("borrows nest more than 4 deep"), "{m}");
                return;
            }
        }
    }
    panic!("a chain of {} borrows was not refused", 3 + 4);
}

/// A host whose catalogue has no `wobble-lead`: the music of the slice is the source's, and what the host cannot play is an error
/// on the reference (not somewhere in text the person never wrote).
struct Cat;

impl Catalog for Cat {
    fn voices(&self) -> Vec<String> {
        vec!["hoover".into(), "warm-pad".into(), "sub-bass".into()]
    }
    fn kits(&self) -> Vec<String> {
        vec!["drums".into()]
    }
    fn palettes(&self) -> Vec<String> {
        Vec::new()
    }
}

#[test]
fn a_voice_the_hosts_catalogue_lacks_is_an_error_on_the_reference() {
    let s = v2();
    let text = format!("tempo 140\nkey fm\ndeck A: bass {V2_CID}.drop\n");
    let with = Env { snippets: Some(&s), catalog: Some(&Cat), ..Env::default() };
    let c = djlive::compile::check_with(&text, &with);
    let d = c.diagnostics.iter().find(|d| d.is_error()).expect("wobble-lead is not an instrument here");
    assert!(d.message.contains("`wobble-lead` is not an instrument"), "{}", d.message);
    assert_eq!(&text[d.span.start..d.span.end], format!("{V2_CID}.drop"), "on the reference");
    // the line's own `inst=` wins, and fixes it
    let fixed = djlive::compile::check_with(&format!("tempo 140\nkey fm\ndeck A: bass {V2_CID}.drop inst=hoover\n"), &with);
    assert!(fixed.compiled.is_some(), "{:?}", fixed.diagnostics.iter().map(|d| d.message.clone()).collect::<Vec<_>>());
    // the source compiles in a plain environment: with a catalogue that lacks its voice it is still borrowed from, so the slice
    // (and the net CID of what is played) is the same on every host
    let plain = ok(&text.replace("deck A: bass", "deck A: kick x | bass"), &s);
    let lit = ok("tempo 140\nkey fm\ndeck A: kick x | bass <[f1 ~ ~ f1 ~ ~ ab1 ~ f1 ~ ~ ~ eb1 ~ c1 ~] [f1 ~ f1 ~ ~ ~ db1 ~ ~ ~ c1 ~ eb1 ~ eb1 ~]> inst=wobble-lead\n", &StaticSnippets::default());
    assert_eq!(plain.decks[&djlive::ast::Deck::A].net_cid(), lit.decks[&djlive::ast::Deck::A].net_cid());
}

#[test]
fn a_snippet_that_borrows_is_imported_with_its_references_written_out_and_its_source_travels() {
    let mut s = v2();
    // a saved snippet whose let borrows the drop's bass
    let snip = ok(&format!("let riff = {V2_CID}.drop.bass\n"), &s);
    s.items.push(("riffs".into(), snip.cid.clone(), snip.text.clone()));
    let c = ok(&format!("tempo 140\nkey fm\nimport {}\ndeck A: bass riff inst=wobble-lead\n", snip.cid), &s);
    let direct = ok(&format!("tempo 140\nkey fm\ndeck A: bass {V2_CID}.drop\n"), &s);
    assert_eq!(c.decks[&djlive::ast::Deck::A].net_cid(), direct.decks[&djlive::ast::Deck::A].net_cid());
    // the source of the borrow, then the snippet that borrows from it, go with the program
    assert_eq!(c.plays.iter().map(|(x, _)| x.as_str()).collect::<Vec<_>>(), [V2_CID, snip.cid.as_str()]);
    let (url, carried) = share::encode_with_tracks(&c, None, false);
    assert!(carried);
    let o = share::open(&url, &Env::none()).expect("opens with nothing saved");
    assert_eq!(o.cid, c.cid);
    assert!(o.check.compiled.is_some());
}

#[test]
fn a_cid_is_a_word_of_the_language_and_the_cids_a_program_borrows_from_are_listed() {
    // a name that reads as a CID cannot be a let, a def or a section: it would be a reference
    let name = "z4EBG9jDzsE2UHZfL8A9ouKLU5vUJsgHC8nVaTE7sWjci7PCukZ";
    let c = djlive::check(&format!("let {name} = c2 e2\n"));
    assert!(c.diagnostics.iter().any(|d| d.message.contains("reads as a stored program's CID")), "{:?}", c.diagnostics.iter().map(|d| d.message.clone()).collect::<Vec<_>>());
    // an import and a set's play name a CID too, and are not references
    assert!(djlive::stored::cids_of(&format!("import {name}\nplay {name}\n")).is_empty());
    let text = format!("tempo 140\nsection a from {V2_CID}.drop\nsection b from {V2_CID}.build\nlet r = {name}.bass\ndeck A: bass {V2_CID}.drop\n");
    assert_eq!(djlive::stored::cids_of(&text), [V2_CID.to_string(), name.to_string()]);
}

#[test]
fn the_example_program_checks_with_its_source_and_keeps_its_cid_through_reformatting() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/beatscript");
    let text = std::fs::read_to_string(dir.join("borrow-a-drop.beatscript")).expect("the example");
    assert!(text.contains(V2_CID), "it borrows from the pinned program");
    let s = v2();
    let c = ok(&text, &s);
    // the canonical text is a fixed point with the same CID, and reformatting keeps it
    let again = ok(&c.text, &s);
    assert_eq!((again.text.as_str(), again.cid.as_str()), (c.text.as_str(), c.cid.as_str()));
    let mut rng = common::Rng(0x5eed_6597_0000_0000);
    for round in 0..40 {
        let t = common::reformat(&text, &mut rng);
        let got = check(&t, &s);
        let cid = got.compiled.unwrap_or_else(|| panic!("round {round} no longer compiles:\n{t}\n{}", djlive::diag::render_all(&got.diagnostics, &t))).cid;
        assert_eq!(cid, c.cid, "round {round} forked:\n{t}");
    }
    // and it is a sentence of the grammar, though only a host can read it
    assert!(djlive::parse::parse(&text).ok());
    assert!(djlive::grammar::recognize::recognize(&djlive::grammar::grammar(), &text).is_ok());
}

mod common;
