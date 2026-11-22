//! `btwd` from outside: text on stdin or `--file`, JSON on stdout, exit 0 whenever the tool ran (a program with
//! errors is an answer), non-zero for usage and I/O only, and every answer is btwapi's own.

use std::io::Write;
use std::process::{Command, Output, Stdio};

use djengine::json::JsonValue;

const BIN: &str = env!("CARGO_BIN_EXE_btwd");
const V2: &str = include_str!("../../btwapi/tests/corpus/hand-dubstep-140-v2.btw");
const MARKERS: &str = include_str!("../../btwapi/tests/corpus/markers-demo.btw");

fn run(args: &[&str], stdin: Option<&[u8]>) -> Output {
    let mut c = Command::new(BIN);
    c.args(args).stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = c.spawn().expect("spawn btwd");
    if let Some(bytes) = stdin {
        child.stdin.take().unwrap().write_all(bytes).unwrap();
    }
    child.wait_with_output().unwrap()
}

fn out(o: &Output) -> String {
    String::from_utf8(o.stdout.clone()).unwrap()
}

fn tmp(name: &str, text: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("btwd-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(name);
    std::fs::write(&p, text).unwrap();
    p
}

#[test]
fn every_answer_is_btwapis_and_one_json_line() {
    let cases: Vec<(&[&str], String)> = vec![
        (&["check"], btwapi::check_json(V2)),
        (&["fmt"], btwapi::fmt_json(V2)),
        (&["highlight"], btwapi::highlight_json(V2)),
        (&["nets", "--brief"], btwapi::nets_json(V2, false)),
        (&["nets"], btwapi::nets_json(V2, true)),
        (&["share"], btwapi::share_json(V2, None, false, "")),
        (&["share", "--sig", "abc_-", "--notes", "--origin", "http://localhost:18097"], btwapi::share_json(V2, Some("abc_-"), true, "http://localhost:18097")),
    ];
    for (args, want) in cases {
        for how in 0..3 {
            let path = tmp("v2.btw", V2);
            let mut a: Vec<String> = args.iter().map(|s| s.to_string()).collect();
            let stdin: Option<&[u8]> = match how {
                0 => Some(V2.as_bytes()),
                1 => {
                    a.push("-".into());
                    Some(V2.as_bytes())
                }
                _ => {
                    a.push("--file".into());
                    a.push(path.to_string_lossy().to_string());
                    None
                }
            };
            let r = run(&a.iter().map(String::as_str).collect::<Vec<_>>(), stdin);
            assert_eq!(r.status.code(), Some(0), "{args:?} {how}: {}", String::from_utf8_lossy(&r.stderr));
            assert_eq!(out(&r), format!("{want}\n"), "{args:?} via {how}");
            assert_eq!(out(&r).lines().count(), 1);
        }
    }
}

#[test]
fn a_program_with_errors_is_an_answer_with_exit_zero() {
    let r = run(&["check"], Some(b"deck A: kick | snare ~ x ~ x\n"));
    assert_eq!(r.status.code(), Some(0));
    let j = JsonValue::parse(&out(&r)).unwrap();
    assert_eq!(j.get("ok").and_then(JsonValue::as_bool), Some(false));
    assert_eq!(j.get("diagnostics").unwrap().at(0).unwrap().get("line").and_then(JsonValue::as_i64), Some(1));
    for sub in ["fmt", "share", "nets", "generation"] {
        let r = run(&[sub, "-n", "1"], Some(b"deck A: kick |\n"));
        assert_eq!(r.status.code(), Some(0), "{sub}: {}", String::from_utf8_lossy(&r.stderr));
        assert_eq!(JsonValue::parse(&out(&r)).unwrap().get("ok").and_then(JsonValue::as_bool), Some(false), "{sub}");
    }
}

#[test]
fn open_takes_a_link_or_a_cid_as_an_argument_or_on_stdin() {
    let share = JsonValue::parse(&btwapi::share_json(V2, None, false, "")).unwrap();
    let url = share.get("url").and_then(JsonValue::as_str).unwrap().to_string();
    let cid = share.get("cid").and_then(JsonValue::as_str).unwrap().to_string();
    for r in [run(&["open", &url], None), run(&["open", "-"], Some(url.as_bytes())), run(&["open"], Some(format!("{url}\n").as_bytes()))] {
        assert_eq!(r.status.code(), Some(0));
        let j = JsonValue::parse(&out(&r)).unwrap();
        assert_eq!(j.get("verified").and_then(JsonValue::as_bool), Some(true));
        assert_eq!(j.get("cid").and_then(JsonValue::as_str), Some(cid.as_str()));
    }
    let r = run(&["open", &cid], None);
    assert_eq!(r.status.code(), Some(0));
    let j = JsonValue::parse(&out(&r)).unwrap();
    assert_eq!((j.get("ok").and_then(JsonValue::as_bool), j.get("has_text").and_then(JsonValue::as_bool)), (Some(false), Some(false)));
    assert_eq!(j.get("claimed_cid").and_then(JsonValue::as_str), Some(cid.as_str()));
}

#[test]
fn generation_takes_n_and_a_scope() {
    let want = btwapi::generation_json(MARKERS, Some("drop"), 2);
    for args in [vec!["generation", "-n", "2", "--scope", "drop"], vec!["generation", "2", "--scope", "drop"]] {
        let r = run(&args, Some(MARKERS.as_bytes()));
        assert_eq!(out(&r), format!("{want}\n"), "{args:?}");
    }
    let r = run(&["generation"], Some(MARKERS.as_bytes()));
    assert_eq!(r.status.code(), Some(2), "-n is needed");
}

#[test]
fn instruments_and_the_cheat_sheet_need_no_input() {
    let r = run(&["instruments"], None);
    assert_eq!(out(&r), format!("{}\n", btwapi::instruments_json()));
    let r = run(&["cheatsheet"], None);
    assert_eq!(out(&r), btwapi::cheatsheet());
    assert!(out(&r).starts_with("# BeatScript cheat sheet"));
}

#[test]
fn version_selftest_and_usage() {
    let r = run(&["--version"], None);
    assert_eq!((r.status.code(), out(&r)), (Some(0), format!("btwd {}\n", env!("CARGO_PKG_VERSION"))));
    let r = run(&["selftest"], None);
    assert_eq!(r.status.code(), Some(0), "{}", String::from_utf8_lossy(&r.stderr));
    assert!(out(&r).starts_with("ok"), "{}", out(&r));
    // usage and I/O errors: exit 2, nothing on stdout, the reason on stderr
    for args in [vec![], vec!["nonesuch"], vec!["check", "--nonesuch"], vec!["check", "--file"], vec!["check", "--file", "/nonexistent/x.btw"], vec!["check", "-n", "x"]] {
        let r = run(&args, Some(b""));
        assert_eq!(r.status.code(), Some(2), "{args:?}");
        assert!(out(&r).is_empty() || args.is_empty(), "{args:?}: {}", out(&r));
        assert!(String::from_utf8_lossy(&r.stderr).starts_with("btwd: ") || args.is_empty(), "{args:?}: {}", String::from_utf8_lossy(&r.stderr));
    }
    let r = run(&["help"], None);
    assert_eq!(r.status.code(), Some(0));
    assert!(out(&r).contains("usage: btwd <command>") && out(&r).contains("cheatsheet"));
    // not UTF-8 on stdin
    let r = run(&["check"], Some(&[0x64, 0xff, 0xfe]));
    assert_eq!(r.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&r.stderr).contains("UTF-8"));
}

#[test]
fn a_large_program_goes_through_stdin() {
    let mut text = String::from("tempo 120\n");
    while text.len() < 300_000 {
        text.push_str("deck A: kick x ~ x ~ | hihat x*8 # filler filler filler\n");
    }
    let r = run(&["check"], Some(text.as_bytes()));
    assert_eq!(r.status.code(), Some(0));
    assert!(JsonValue::parse(&out(&r)).is_ok());
}

#[test]
fn check_same_as_and_the_pair_form() {
    // `check --same-as FILE`: the check report plus a `same_as` field, btwapi's own
    let a = tmp("same-a.btw", V2);
    let b = tmp("same-b.btw", &V2.replace("tempo 140", "tempo 141"));
    let o = run(&["check", "--file", a.to_str().unwrap(), "--same-as", b.to_str().unwrap()], None);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let r = JsonValue::parse(&out(&o)).unwrap();
    let sa = r.get("same_as").expect("a same_as field");
    assert_eq!(sa.js_stringified(), btwapi::same_as_report(V2, &V2.replace("tempo 140", "tempo 141")).js_stringified());
    assert_eq!(sa.get("same").and_then(JsonValue::as_bool), Some(false));
    assert_eq!(r.get("cid").and_then(JsonValue::as_str), JsonValue::parse(&btwapi::check_json(V2)).unwrap().get("cid").and_then(JsonValue::as_str));
    // against itself, by a share link that carries the text
    let link = JsonValue::parse(&btwapi::share_json(V2, None, false, "")).unwrap();
    let url = link.get("url").and_then(JsonValue::as_str).expect("share answers a url").to_string();
    let o = run(&["check", "--file", a.to_str().unwrap(), "--same-as", &url], None);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(JsonValue::parse(&out(&o)).unwrap().get("same_as").unwrap().get("same").and_then(JsonValue::as_bool), Some(true));
    // a bare CID has no text here: usage error, saying so
    let cid = link.get("cid").and_then(JsonValue::as_str).unwrap().to_string();
    let o = run(&["check", "--file", a.to_str().unwrap(), "--same-as", &cid], None);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("bare CID"), "{}", String::from_utf8_lossy(&o.stderr));
    // `same-as`: both programs as one JSON object on stdin
    let pair = JsonValue::Object(djengine::json::JsonObject::from_pairs([("a", JsonValue::str(V2)), ("b", JsonValue::str(MARKERS))])).js_stringified();
    let o = run(&["same-as"], Some(pair.as_bytes()));
    assert!(o.status.success());
    assert_eq!(out(&o).trim_end(), btwapi::same_as_json(V2, MARKERS));
}

#[test]
fn diff_two_files() {
    let a = tmp("diff-a.btw", V2);
    let changed = V2.replace("tempo 140", "tempo 142");
    let b = tmp("diff-b.btw", &changed);
    let o = run(&["diff", a.to_str().unwrap(), b.to_str().unwrap()], None);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(out(&o).trim_end(), btwapi::same_as_json(V2, &changed));
    let r = JsonValue::parse(&out(&o)).unwrap();
    let sum = r.get("diff").and_then(|d| d.get("summary")).and_then(JsonValue::as_array).expect("a diff summary").clone();
    assert_eq!(sum.first().and_then(JsonValue::as_str), Some("tempo 140 -> 142"));
    // two programs or nothing
    let o = run(&["diff", a.to_str().unwrap()], None);
    assert!(!o.status.success() && String::from_utf8_lossy(&o.stderr).contains("two programs"));
}

#[test]
fn fmt_expand_writes_out_seeds() {
    let o = run(&["fmt", "--expand", "--lineage", "-"], Some(b"arrange from seed(house, 42)[1..8]\n"));
    assert!(o.status.success());
    let s = out(&o);
    assert!(s.contains("\"ok\":true") && s.contains("forked-from") && s.contains("from_cid"), "{s}");
    let e = run(&["expand", "-"], Some(b"arrange from seed(house, 42)[1..8]\n"));
    assert!(out(&e).contains("section intro"), "{}", out(&e));
}

#[test]
fn with_gives_check_the_programs_an_import_names_and_never_fetches() {
    let lib = tmp("riffs.btw", "# riffs\nlet riff = c2 e2 g2 b2\ndef hit(d) {\n  deck $d: kick x ~ x ~\n}\n");
    let fmt = JsonValue::parse(&out(&run(&["fmt", "--file", lib.to_str().unwrap()], None))).unwrap();
    let cid = fmt.get("cid").and_then(JsonValue::as_str).unwrap().to_string();
    let song = tmp("song-with.btw", &format!("tempo 120\nimport {cid}\ndeck A: bass riff\nhit(B)\n"));
    let check = |extra: &[&str]| {
        let mut a = vec!["check", "--file", song.to_str().unwrap()];
        a.extend_from_slice(extra);
        let r = run(&a, None);
        assert_eq!(r.status.code(), Some(0), "{}", String::from_utf8_lossy(&r.stderr));
        JsonValue::parse(&out(&r)).unwrap()
    };
    // without it: not here, in words, and no fetch
    let none = check(&[]);
    assert_eq!(none.get("ok").and_then(JsonValue::as_bool), Some(false));
    let msg = none.get("diagnostics").and_then(JsonValue::as_array).unwrap()[0].get("message").and_then(JsonValue::as_str).unwrap().to_string();
    assert!(msg.contains("is not here"), "{msg}");
    // with the file, or the directory it is in, or the link that carries it: it checks, the same every way
    let dir = lib.parent().unwrap().to_str().unwrap().to_string();
    let a = check(&["--with", lib.to_str().unwrap()]);
    let b = check(&["--with", &dir]);
    assert_eq!(a.get("ok").and_then(JsonValue::as_bool), Some(true), "{}", a.js_stringified());
    assert_eq!(a.get("cid"), b.get("cid"));
    assert_eq!(a.get("uses").and_then(JsonValue::as_array).map(Vec::len), Some(1));
    let share = JsonValue::parse(&out(&run(&["share", "--with", lib.to_str().unwrap(), "--file", song.to_str().unwrap()], None))).unwrap();
    let url = share.get("beats_url").and_then(JsonValue::as_str).unwrap().to_string();
    let c = check(&["--with", &url]);
    assert_eq!(c.get("cid"), a.get("cid"));
    // --with a file that does not read, or names nothing, is a usage error
    let bad = tmp("bad-with.btw", "deck A kick x\n");
    assert_eq!(run(&["check", "--with", bad.to_str().unwrap(), "--file", song.to_str().unwrap()], None).status.code(), Some(2));
    assert_eq!(run(&["check", "--with"], None).status.code(), Some(2));
}
