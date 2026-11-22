//! A program that draws on other programs (an `import`, a set's tracks) is checked the same on every host: the closure the
//! link carries, never a fetch (docs/BTW.md, "Share links"). Whether a program checks depends on the text and the programs
//! it is given, never on the machine: a host that has none of them says so in words, and a host that has them hears the
//! same music.

use djlive::env::{Env, StaticSnippets};
use djlive::share;

const RIFFS: &str = "let riff = c2 e2 g2 b2\ndef hit(d) {\n  deck $d: kick x ~ x ~\n}\n";
const LIB: &str = "let rumble = a1 a1 e2 a1\n";

fn snippets(progs: &[(&str, &str)]) -> StaticSnippets {
    let mut s = StaticSnippets::default();
    for (name, text) in progs {
        let c = djlive::check(text).compiled.unwrap_or_else(|| panic!("{name} does not compile"));
        s.items.push((name.to_string(), c.cid, c.text));
    }
    s
}

fn compile(text: &str, s: &StaticSnippets) -> djlive::Check {
    djlive::compile::check_with(text, &Env { snippets: Some(s), ..Env::default() })
}

#[test]
fn a_link_that_imports_carries_the_snippet_and_opens_where_nothing_is_saved() {
    let s = snippets(&[("riffs", RIFFS)]);
    let cid = s.items[0].1.clone();
    let c = compile("import \"riffs\"\ndeck A: bass riff\nhit(B)\n", &s).compiled.expect("it checks with the snippet saved");
    // the program draws on the snippet, and the link says so
    assert_eq!(c.plays, vec![(cid.clone(), s.items[0].2.clone())]);
    let (url, carried) = share::encode_with_tracks(&c, None, false);
    assert!(carried);
    assert_eq!(share::parse_link(&url).unwrap().tracks.len(), 1);
    // a machine with nothing saved, and one with no store at all, open it and get the same program
    for env in [Env { snippets: Some(&StaticSnippets::default()), ..Env::default() }, Env::none()] {
        let o = share::open(&url, &env).expect("the link carries what it imports");
        assert_eq!(o.cid, c.cid);
        let opened = o.check.compiled.expect("and it compiles there");
        assert_eq!(opened.net_cids(), c.net_cids(), "the same nets, so the same music");
    }
}

#[test]
fn what_an_import_imports_travels_too_leaves_first() {
    let s = snippets(&[("lib", LIB)]);
    let mid = format!("import {}\nlet riff = c2 e2\n", s.items[0].1);
    let s2 = {
        let mut x = s.clone();
        let c = compile(&mid, &s).compiled.expect("mid checks");
        x.items.push(("mid".into(), c.cid, c.text));
        x
    };
    let top = compile("import mid\ndeck A: bass rumble\n", &s2).compiled.expect("top checks");
    let order: Vec<&str> = top.plays.iter().map(|(c, _)| c.as_str()).collect();
    assert_eq!(order, [s2.items[0].1.as_str(), s2.items[1].1.as_str()], "lib, then mid, which needs it");
    let (url, carried) = share::encode_with_tracks(&top, None, false);
    assert!(carried);
    let o = share::open(&url, &Env::none()).expect("opens with nothing saved");
    assert_eq!(o.cid, top.cid);
    assert!(o.check.compiled.is_some());
}

#[test]
fn a_link_always_carries_what_it_draws_on_however_long_it_is() {
    // a snippet that does not compress: the link is far past 2000 characters, and still whole (a link that named it by CID
    // alone would open on this machine and not on another)
    let mut big = String::from(LIB);
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    for _ in 0..400 {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        big.push_str(&format!("let n{:x} = {}\n", x >> 40, (x >> 20) % 97));
    }
    let s = snippets(&[("big", &big)]);
    let c = compile("import big\ndeck A: bass rumble\n", &s).compiled.expect("it checks");
    let (url, carried) = share::encode_with_tracks(&c, None, false);
    assert!(carried && url.chars().count() > share::INLINE_LIMIT, "{} chars", url.chars().count());
    assert!(share::open(&url, &Env::none()).is_ok());
}

#[test]
fn a_carried_program_that_is_not_the_one_its_cid_names_is_refused() {
    let s = snippets(&[("riffs", RIFFS)]);
    let mut c = compile("import riffs\ndeck A: bass riff\n", &s).compiled.unwrap();
    c.plays[0].1 = c.plays[0].1.replace("c2", "d2");
    let (url, _) = share::encode_with_tracks(&c, None, false);
    let e = share::open(&url, &Env::none()).err().expect("refused");
    assert!(e.contains("not the program it names"), "{e}");
}

#[test]
fn a_program_that_is_not_here_says_so_the_same_way_everywhere() {
    let s = snippets(&[("riffs", RIFFS)]);
    let cid = s.items[0].1.clone();
    let text = format!("import {cid}\ndeck A: bass riff\n");
    // checked with nothing carried and no store: not here, on the import's own line, with what would fix it
    let none = StaticSnippets::default();
    let inline = djlive::set::Inline { tracks: Default::default(), store: None };
    for env in [Env { snippets: Some(&none), ..Env::default() }, Env { snippets: Some(&inline), ..Env::default() }] {
        let c = djlive::compile::check_with(&text, &env);
        let d = c.diagnostics.iter().find(|d| d.is_error()).expect("an error");
        if djlive::env::is_not_here(&d.message) {
            assert!(d.message.starts_with(&format!("`{}` is not here", djengine::rights::short_cid(&cid))), "{}", d.message);
            assert_eq!(d.help.as_deref(), Some(djlive::env::NOT_HERE_HELP));
            assert_eq!(d.position(&text).0, 1);
        }
    }
    // the Inline closure (the link's, or a page's) always says it so
    let c = djlive::compile::check_with(&text, &Env { snippets: Some(&inline), ..Env::default() });
    let d = c.diagnostics.iter().find(|d| d.is_error()).expect("an error");
    assert!(djlive::env::is_not_here(&d.message), "{}", d.message);
    // and with the program in the closure, it checks and means what it means with the store
    let mut tracks = std::collections::BTreeMap::new();
    tracks.insert(cid, s.items[0].2.clone());
    let have = djlive::set::Inline { tracks, store: None };
    let c2 = djlive::compile::check_with(&text, &Env { snippets: Some(&have), ..Env::default() });
    assert_eq!(c2.compiled.expect("it checks with the closure").cid, compile(&text, &s).compiled.unwrap().cid);
}
