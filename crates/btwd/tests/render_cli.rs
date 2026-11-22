//! `btwd render` as a process: stdin and `--file`, the exit codes, the JSON lines on stderr, and the
//! point of it all: the first bytes of PCM are on stdout long before the render is done.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_btwd");
const CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../btwapi/tests/corpus");

const SONG: &str = "tempo 140\nsection one 2 bars {\n  deck A: kick x ~ x ~ x ~ x ~ x ~ x ~ x ~ x ~\n}\nsection two 2 bars {\n  deck A: kick x ~ x ~ x ~ x ~ x ~ x ~ x ~ x ~ | hihat ~ x ~ x ~ x ~ x ~ x ~ x ~ x ~ x\n}\narrange one two\n";

struct Out {
    code: i32,
    stdout: Vec<u8>,
    stderr: String,
}

fn render(args: &[&str], stdin: Option<&str>) -> Out {
    let mut cmd = Command::new(BIN);
    cmd.arg("render").args(args).stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    if let Some(t) = stdin {
        // a usage error exits before it reads stdin: the pipe may be closed by then (EPIPE), which is not this test's business
        let _ = child.stdin.take().unwrap().write_all(t.as_bytes());
    }
    let o = child.wait_with_output().unwrap();
    Out { code: o.status.code().unwrap_or(-1), stdout: o.stdout, stderr: String::from_utf8(o.stderr).unwrap() }
}

fn events(s: &str) -> Vec<djengine::json::JsonValue> {
    s.lines().map(|l| djengine::json::JsonValue::parse(l).unwrap_or_else(|e| panic!("not JSON: {l}: {e:?}"))).collect()
}

fn name(v: &djengine::json::JsonValue) -> &str {
    v.get("event").and_then(djengine::json::JsonValue::as_str).unwrap_or("")
}

#[test]
fn text_on_stdin_renders_and_exits_0() {
    let o = render(&[], Some(SONG));
    assert_eq!(o.code, 0, "{}", o.stderr);
    let ev = events(&o.stderr);
    assert_eq!(name(&ev[0]), "start");
    assert_eq!(name(ev.last().unwrap()), "end");
    // 4 bars at 140 bpm, 44.1 kHz, s16le stereo
    assert_eq!(o.stdout.len(), (4.0 * 4.0 * 60.0 / 140.0 * 44_100.0_f64).round() as usize * 4);
}

#[test]
fn file_and_stdin_give_the_same_bytes() {
    let dir = std::env::temp_dir().join(format!("btwd-render-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("song.btw");
    std::fs::write(&p, SONG).unwrap();
    let a = render(&["--file", p.to_str().unwrap(), "--chunk-ms", "40"], None);
    let b = render(&["--chunk-ms=500"], Some(SONG));
    let c = render(&["-"], Some(SONG));
    assert_eq!((a.code, b.code, c.code), (0, 0, 0));
    assert!(a.stdout == b.stdout && b.stdout == c.stdout);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn errors_are_one_event_line_and_exit_1() {
    // a mistake in the program
    let o = render(&[], Some("tempo 140\ndeck B bass c2\n"));
    assert_eq!(o.code, 1);
    assert!(o.stdout.is_empty());
    let ev = events(&o.stderr);
    assert_eq!(ev.len(), 1);
    assert_eq!(name(&ev[0]), "error");
    assert!(ev[0].get("message").and_then(djengine::json::JsonValue::as_str).unwrap().contains("2:"));
    // usage
    for args in [&["--sr", "7"][..], &["--format", "mp3"], &["--bogus"], &["--chunk-ms", "0"], &["--max-bars", "0"], &["--from-bar", "x"]] {
        let o = render(args, Some(SONG));
        assert_eq!(o.code, 1, "{args:?}");
        assert!(o.stdout.is_empty(), "{args:?}");
        let ev = events(&o.stderr);
        assert_eq!(ev.len(), 1, "{args:?}: {}", o.stderr);
        assert_eq!(name(&ev[0]), "error");
    }
    // a file that is not there
    let o = render(&["--file", "/no/such/file.btw"], None);
    assert_eq!(o.code, 1);
    assert_eq!(name(&events(&o.stderr)[0]), "error");
}

#[test]
fn restricted_and_asset_programs_are_refused_on_stderr() {
    for (text, code) in [("use restricted\ntempo 120\ndeck A: kick x ~ x ~\n", "restricted"), ("sample harp from \"/x.wav\" license cc0 else bright-pluck\ntempo 120\ndeck A: kick x ~ x ~\n", "uses_assets")] {
        let o = render(&[], Some(text));
        assert_eq!(o.code, 1, "{}", o.stderr);
        assert!(o.stdout.is_empty());
        let ev = events(&o.stderr);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].get("code").and_then(djengine::json::JsonValue::as_str), Some(code));
    }
}

#[test]
fn the_first_bytes_arrive_long_before_the_render_ends() {
    // a whole corpus song (about two minutes of audio): read stdout as it comes and note when the
    // first byte and the last one were seen
    let text = std::fs::read_to_string(format!("{CORPUS}/markers-demo.btw")).unwrap();
    let t0 = Instant::now();
    let mut child = Command::new(BIN).arg("render").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(text.as_bytes()).unwrap();
    let mut out = child.stdout.take().unwrap();
    let mut first = [0u8; 1];
    out.read_exact(&mut first).unwrap();
    let ttfb = t0.elapsed();
    let mut n = 1usize;
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let k = out.read(&mut buf).unwrap();
        if k == 0 {
            break;
        }
        n += k;
    }
    let total = t0.elapsed();
    assert!(child.wait().unwrap().success());
    assert!(n > 8_000_000, "{n} bytes");
    // the contract is under 1 s on a release build (scripts/bench-render.sh measures it); here, with
    // whatever profile the tests run in and a busy machine, require only that the first byte is
    // early in absolute terms and well ahead of the end
    assert!(ttfb < Duration::from_secs(3), "first byte after {ttfb:?}");
    assert!(ttfb.as_secs_f64() < total.as_secs_f64() * 0.5, "first byte at {ttfb:?} of {total:?}");
}

#[test]
fn a_reader_that_leaves_ends_the_render_quietly() {
    // the server closes the pipe when the listener goes: no error line, exit 0, and not a minute of waiting
    let text = std::fs::read_to_string(format!("{CORPUS}/trance-138.btw")).unwrap();
    let t0 = Instant::now();
    let mut child = Command::new(BIN).arg("render").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(text.as_bytes()).unwrap();
    let mut out = child.stdout.take().unwrap();
    let mut buf = vec![0u8; 4096];
    out.read_exact(&mut buf).unwrap();
    drop(out);
    let o = child.wait_with_output().unwrap();
    assert!(t0.elapsed() < Duration::from_secs(20));
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let err = String::from_utf8(o.stderr).unwrap();
    assert!(!err.contains("\"error\""), "{err}");
}
