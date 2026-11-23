//! `beatscript-dump`: write the open-data archive of every public CC0 program (beatscript_serve::opendata, docs/OPEN-DATA.md).
//!
//!   beatscript-dump -programs <data>/programs -out <data>/dumps -public-url https://beatscript.app
//!
//! Reads the programs store (never writes it), builds `beatscript-cc0-<sha256 prefix>.tar.gz` (programs/<cid>.beatscript,
//! programs/<cid>.jsonld, manifest.jsonld), writes it once into `-out` and points `latest.json` at it. The same programs
//! are the same archive: run it again with nothing new and nothing changes. Prints the pointer. It schedules nothing:
//! tools/open-data/ has the job definition.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::SystemTime;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |k: &str, d: &str| -> String {
        let key = format!("-{k}");
        let long = format!("--{k}");
        let mut it = args.iter();
        while let Some(a) = it.next() {
            if *a == key || *a == long {
                return it.next().cloned().unwrap_or_default();
            }
            if let Some(v) = a.strip_prefix(&format!("{key}=")).or_else(|| a.strip_prefix(&format!("{long}="))) {
                return v.to_string();
            }
        }
        beatscript_serve::flag_env(k).unwrap_or_else(|| d.to_string())
    };
    if args.iter().any(|a| a == "-h" || a == "--help") {
        eprintln!("usage: beatscript-dump -programs DIR -out DIR [-public-url URL]");
        return ExitCode::SUCCESS;
    }
    let programs = PathBuf::from(flag("programs", "./data/programs"));
    let out = PathBuf::from(flag("out", "./data/dumps"));
    let origin = flag("public-url", "https://beatscript.app");
    let origin = origin.trim_end_matches('/');
    if !programs.is_dir() {
        eprintln!("beatscript-dump: {} is not a directory", programs.display());
        return ExitCode::from(2);
    }
    let entries = beatscript_serve::opendata::scan(&programs);
    let onto: &dyn djlive::linked::Ontology = djhost::ontology::vocabulary();
    let dump = beatscript_serve::opendata::build_dump(origin, &entries, Some(onto));
    match beatscript_serve::opendata::write_dump(&out, origin, &dump, SystemTime::now()) {
        Ok(latest) => {
            println!("{}", serde_json::to_string_pretty(&latest).unwrap());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("beatscript-dump: writing {}: {e}", out.display());
            ExitCode::FAILURE
        }
    }
}
