//! btwd: btw from a shell and for the btw.bitwrap.io server. Program text on stdin (`-`) or `--file`;
//! JSON on stdout; exit 0 whenever the tool ran (a program with errors is an answer, not a failure),
//! non-zero only for usage, I/O or a panic. `render` is render.rs's.

mod export;
mod listen;
mod lsp;
mod render;
mod selftest;

use std::io::{Read, Write};

const USAGE: &str = "btwd: btw from a shell (docs/BTW.md)

usage: btwd <command> [options]

  check        [--file F | -] [--same-as F|LINK] [--expand]  the CheckReport as JSON (errors with line, column, help; the CID;
                                         the nets; expanded_cid, and refs and resolution for a program with references);
                                         with --same-as, a `same_as` field: is the other program (a file, or a share link that
                                         carries its text) the same song, by its nets; with --expand, each entry of refs carries
                                         its expansion (the BeatScript the reference is written out as)
  diff         A B                       are A and B (each a file or a share link) the same song, and if not, what changed in
                                         music: same_as's report with its `diff` (song settings, arrangement, transforms, notes)
  same-as      [--file F | -]            {\"a\": text, \"b\": text} as JSON in; are they the same song (by their nets), as JSON
  fmt          [--file F | -] [--expand] [--lineage]   the canonical text, its CID and the comments, as JSON; with --expand,
                                         every seed(...) and every <cid>.section borrowed from a program given with --with written out as the BeatScript it
                                         stands for (`expand`), to edit from there
  expand       [--file F | -] [--lineage]  the same as fmt --expand: {ok, text, cid, from_cid, refs}; --lineage opens the text
                                         with `#! forked-from: <from_cid>`
  highlight    [--file F | -]            [[start, end, \"class\"], ...] with UTF-8 byte offsets
  nets         [--file F | -] [--brief]  the nets as CIDs (and envelopes), for debugging and tests
  share        [--file F | -] [--sig S] [--notes] [--origin URL]   the self-sealing link, as JSON
  open         <url | cid | #fragment | ->  decode a link, recompile, verify its CID, as JSON
  generation   [--file F | -] [--scope NAME] -n N    the CID of generation N of an evolve, as JSON
  instruments                            the voices, family words, kits and palettes, as JSON
  cheatsheet                             the compact grammar generated from docs/BTW.md (markdown)
  render       [--sr N] [--format s16le|f32le] [--chunk-ms N] [--max-bars N] [--from-bar N] [--file F | -]
                                         stream raw PCM to stdout while rendering (events as JSON lines on stderr)
  export       --format midi|stems [--sr N] [--max-bars N] [--license L] [--credit C]... [--file F | -]
                                         for a DAW: the Standard MIDI File, or a zip of stems (a WAV per part and
                                         per deck, the master, the .mid and manifest.json), on stdout
  lsp                                    the language server (LSP over stdio): diagnostics, quick-fixes, outline, navigation,
                                         rename, completion, hover, signature help, inlay hints, refactors (crates/btwls)
  listen       [--section NAME] [--bars N] [--max-bars N] [--budget SECONDS] [--sr N] [--file F | -]
                                         render offline and summarise it as JSON: loudness per deck, part and section,
                                         silent parts and why, detected tempo and key, clipping (listen.rs)
  selftest                               run the built-in corpus in-process; prints ok
  --version

Text comes from --file, or from stdin when there is no --file (or with -).

--with FILE, before or after any command (as often as needed): a program the others may import by CID (`import z4EBG...`), as a
.btw file, a directory of them, or a share link that carries it (and what it carries). btwd never fetches: a program that
draws on one it was not given says `is not here`.";

struct Opts {
    file: Option<String>,
    stdin: bool,
    sig: Option<String>,
    notes: bool,
    origin: String,
    scope: Option<String>,
    n: Option<u32>,
    brief: bool,
    same_as: Option<String>,
    expand: bool,
    lineage: bool,
    positional: Vec<String>,
}

fn parse_opts(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts { file: None, stdin: false, sig: None, notes: false, origin: String::new(), scope: None, n: None, brief: false, same_as: None, expand: false, lineage: false, positional: Vec::new() };
    let mut i = 0;
    let value = |i: &mut usize, flag: &str| -> Result<String, String> {
        *i += 1;
        args.get(*i).cloned().ok_or_else(|| format!("{flag} needs a value"))
    };
    while i < args.len() {
        match args[i].as_str() {
            "--file" | "-f" => o.file = Some(value(&mut i, "--file")?),
            "--sig" => o.sig = Some(value(&mut i, "--sig")?),
            "--expand" => o.expand = true,
            "--lineage" => o.lineage = true,
            "--origin" => o.origin = value(&mut i, "--origin")?,
            "--scope" => o.scope = Some(value(&mut i, "--scope")?),
            "-n" | "--n" => o.n = Some(value(&mut i, "-n")?.parse().map_err(|_| "-n takes a whole number".to_string())?),
            "--same-as" => o.same_as = Some(value(&mut i, "--same-as")?),
            "--notes" => o.notes = true,
            "--brief" => o.brief = true,
            "-" => o.stdin = true,
            a if a.starts_with("--") => return Err(format!("unknown option {a}")),
            a => o.positional.push(a.to_string()),
        }
        i += 1;
    }
    Ok(o)
}

/// The program text: `--file`, else stdin.
fn text(o: &Opts) -> Result<String, String> {
    if let Some(f) = &o.file {
        return std::fs::read_to_string(f).map_err(|e| format!("{f}: {e}"));
    }
    let mut bytes = Vec::new();
    std::io::stdin().read_to_end(&mut bytes).map_err(|e| format!("stdin: {e}"))?;
    String::from_utf8(bytes).map_err(|_| "stdin is not UTF-8".to_string())
}

/// The program `--same-as` (or `diff`) names: a file, else a share link (or `#fragment`) that carries its text. A bare CID
/// has no text here (btwd has no store): the server resolves those.
fn other_program(s: &str) -> Result<String, String> {
    if std::path::Path::new(s).is_file() {
        return std::fs::read_to_string(s).map_err(|e| format!("{s}: {e}"));
    }
    let v = djengine::json::JsonValue::parse(&btwapi::open_json(s)).map_err(|_| "open answered no JSON".to_string())?;
    match v.get("canonical").and_then(|c| c.as_str()) {
        Some(t) => Ok(t.to_string()),
        None if v.get("has_text").and_then(|h| h.as_bool()) == Some(false) && v.get("claimed_cid").and_then(|c| c.as_str()).is_some() => {
            Err(format!("{s}: a bare CID carries no text and btwd has no store; pass the program's file or a share link"))
        }
        None => Err(format!("{s}: not a file, and not a link btwd can open ({})", v.get("error").and_then(|e| e.as_str()).unwrap_or("no text"))),
    }
}

fn out(s: &str) -> Result<(), String> {
    let mut o = std::io::stdout().lock();
    o.write_all(s.as_bytes()).and_then(|_| if s.ends_with('\n') { Ok(()) } else { o.write_all(b"\n") }).and_then(|_| o.flush()).map_err(|e| format!("stdout: {e}"))
}

fn run(args: &[String]) -> Result<(), String> {
    let cmd = args.first().map(String::as_str).unwrap_or("");
    let rest = &args[args.len().min(1)..];
    match cmd {
        "check" | "fmt" | "expand" | "highlight" | "nets" | "share" | "generation" | "open" | "same-as" => {
            let o = parse_opts(rest)?;
            let result = match cmd {
                "open" => {
                    // a link or CID on the command line, or on stdin
                    let s = match o.positional.first() {
                        Some(s) if !o.stdin => s.clone(),
                        _ => text(&o)?,
                    };
                    btwapi::open_json(&s)
                }
                _ => {
                    let t = text(&o)?;
                    match cmd {
                        "check" => match &o.same_as {
                            None => btwapi::check_json_with(&t, o.expand),
                            Some(other) => {
                                let b = other_program(other)?;
                                let mut r = btwapi::check_report_with(&t, o.expand);
                                if let djengine::json::JsonValue::Object(m) = &mut r {
                                    m.set("same_as", btwapi::same_as_report(&t, &b));
                                }
                                r.js_stringified()
                            }
                        },
                        "same-as" => btwapi::same_as_json_pair(&t),
                        "fmt" if !o.expand => btwapi::fmt_json(&t),
                        "fmt" | "expand" => btwapi::expand_json(&t, o.lineage),
                        "highlight" => btwapi::highlight_json(&t),
                        "nets" => btwapi::nets_json(&t, !o.brief),
                        "share" => btwapi::share_json(&t, o.sig.as_deref(), o.notes, &o.origin),
                        _ => {
                            let n = o.n.or_else(|| o.positional.first().and_then(|p| p.parse().ok())).ok_or("generation needs -n N")?;
                            btwapi::generation_json(&t, o.scope.as_deref(), n)
                        }
                    }
                }
            };
            out(&result)
        }
        "diff" => {
            let o = parse_opts(rest)?;
            let [a, b] = o.positional.as_slice() else { return Err("diff takes two programs: btwd diff A B (each a file or a share link)".into()) };
            out(&btwapi::same_as_json(&other_program(a)?, &other_program(b)?))
        }
        "instruments" => out(&btwapi::instruments_json()),
        "cheatsheet" => out(&btwapi::cheatsheet()),
        "selftest" => selftest::run(),
        "--version" | "-V" | "version" => out(&format!("btwd {}", env!("CARGO_PKG_VERSION"))),
        "" | "help" | "--help" | "-h" => {
            println!("{USAGE}");
            if cmd.is_empty() {
                Err("a command is needed".into())
            } else {
                Ok(())
            }
        }
        other => Err(format!("unknown command `{other}`; `btwd help` lists them")),
    }
}

/// Every `--with FILE` taken out of `args`, and what each names told to the checker: the programs a text may import by CID.
/// A file is read as the program it is (its canonical text and CID are made here), a directory is every `.btw` in it, and
/// anything else is a share link, whose own bundle is kept too. A file that imports another given after it is read once that one
/// is known, so the order they are given in does not matter.
fn take_with(args: Vec<String>) -> Result<Vec<String>, String> {
    let (mut rest, mut given) = (Vec::new(), Vec::new());
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        if a == "--with" {
            given.push(it.next().ok_or("--with needs a file, a directory or a share link")?);
        } else {
            rest.push(a);
        }
    }
    let mut files: Vec<(String, String)> = Vec::new();
    for g in &given {
        let p = std::path::Path::new(g);
        if p.is_dir() {
            let mut names: Vec<_> = std::fs::read_dir(p).map_err(|e| format!("{g}: {e}"))?.flatten().map(|e| e.path()).filter(|f| f.extension().is_some_and(|x| x == "btw")).collect();
            names.sort();
            for f in names {
                files.push((f.display().to_string(), std::fs::read_to_string(&f).map_err(|e| format!("{}: {e}", f.display()))?));
            }
        } else if p.is_file() {
            files.push((g.clone(), std::fs::read_to_string(p).map_err(|e| format!("{g}: {e}"))?));
        } else {
            let v = djengine::json::JsonValue::parse(&btwapi::open_json(g)).map_err(|_| "open answered no JSON".to_string())?;
            let Some(canonical) = v.get("canonical").and_then(|c| c.as_str()) else {
                return Err(format!("--with {g}: not a file, a directory or a link that carries a program ({})", v.get("error").and_then(|e| e.as_str()).unwrap_or("no text")));
            };
            if let Some(b) = v.get("bundle").and_then(|b| b.as_object()) {
                for (cid, t) in b.iter() {
                    btwapi::known::know(cid, t.as_str().unwrap_or("")).map_err(|e| format!("--with {g}: {e}"))?;
                }
            }
            let cid = v.get("cid").and_then(|c| c.as_str()).unwrap_or("");
            btwapi::known::know(cid, canonical).map_err(|e| format!("--with {g}: {e}"))?;
        }
    }
    // a file that imports one not read yet waits: read them again until none is left that can be
    while !files.is_empty() {
        let before = files.len();
        let mut later = Vec::new();
        for (name, text) in files {
            let parsed = djlive::parse::parse_with(&text, &btwapi::web_env().env());
            if parsed.ok() {
                let canonical = djlive::print::canonical(&parsed.program);
                btwapi::known::know(&djlive::compile::program_cid(&canonical), &canonical).map_err(|e| format!("--with {name}: {e}"))?;
            } else {
                later.push((name, text));
            }
        }
        if later.len() == before {
            let (name, text) = &later[0];
            let d = djlive::parse::parse_with(text, &btwapi::web_env().env()).diagnostics.into_iter().find(|d| d.is_error());
            let (line, col, _) = d.as_ref().map_or((1, 1, 1), |d| d.position(text));
            return Err(format!("--with {name}: does not read ({line}:{col}: {})", d.map_or(String::new(), |d| d.message)));
        }
        files = later;
    }
    Ok(rest)
}

fn main() {
    let args: Vec<String> = match take_with(std::env::args().skip(1).collect()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("btwd: {e}");
            std::process::exit(2);
        }
    };
    if args.first().map(String::as_str) == Some("lsp") {
        std::process::exit(lsp::main());
    }
    if args.first().map(String::as_str) == Some("render") {
        std::process::exit(render::main(args[1..].to_vec()));
    }
    if args.first().map(String::as_str) == Some("export") {
        std::process::exit(export::main(args[1..].to_vec()));
    }
    if args.first().map(String::as_str) == Some("listen") {
        std::process::exit(listen::main(args[1..].to_vec()));
    }
    if let Err(e) = run(&args) {
        eprintln!("btwd: {e}");
        std::process::exit(2);
    }
}
