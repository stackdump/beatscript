//! btwround recover <audio.wav> [--ref program.btw] [--out recovered.btw] [--btwd PATH] [--jobs N] [-v]

use std::path::PathBuf;

use btwround::btwd::Btwd;
use btwround::recover::{recover, Options};
use btwround::wav;

const USAGE: &str = "btwround: audio -> BeatScript program (docs/ROUNDTRIP.md)

usage:
  btwround recover <audio.wav> [--ref program.btw] [--out recovered.btw] [--btwd PATH] [--jobs N] [-v]
      recover a program from a render. --ref is the original program, used only to score (same_as).
  btwround render <program.btw> <out.wav> [--btwd PATH]
      render with btwd, as a WAV (the forward model, for making fixtures)

exit 0 when the recovery is sample identical to the input, or `same_as` the reference; 1 otherwise.";

fn main() {
    std::process::exit(match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("btwround: {e}");
            2
        }
    });
}

fn run() -> Result<i32, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).ok_or(USAGE)?;
    let (mut positional, mut reference, mut out, mut bin, mut jobs, mut verbose) = (Vec::new(), None, None, None, None, false);
    let mut i = 1;
    fn value(args: &[String], i: &mut usize) -> Result<String, String> {
        let flag = args[*i].clone();
        *i += 1;
        args.get(*i).cloned().ok_or(format!("{flag} needs a value"))
    }
    while i < args.len() {
        match args[i].clone().as_str() {
            "--ref" => reference = Some(value(&args, &mut i)?),
            "--out" => out = Some(value(&args, &mut i)?),
            "--btwd" => bin = Some(value(&args, &mut i)?),
            "--jobs" => jobs = Some(value(&args, &mut i)?.parse::<usize>().map_err(|e| e.to_string())?),
            "-v" => verbose = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(0);
            }
            a => positional.push(a.to_string()),
        }
        i += 1;
    }
    let bin: PathBuf = bin.map(PathBuf::from).unwrap_or_else(Btwd::find_bin);
    let scratch = std::env::temp_dir().join(format!("btwround-{}", std::process::id()));
    let btwd = Btwd::new(bin, &scratch)?;
    let result = match cmd {
        "recover" => {
            let path = positional.first().ok_or("recover needs an audio file")?;
            let audio = wav::read(path.as_ref())?;
            let reference = match &reference {
                Some(p) => Some(std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?),
                None => None,
            };
            let mut opts = Options::default();
            opts.verbose = verbose;
            if let Some(j) = jobs {
                opts.jobs = j;
            }
            let rep = recover(&audio, &btwd, &opts, reference.as_deref())?;
            match &out {
                Some(p) => std::fs::write(p, &rep.text).map_err(|e| format!("{p}: {e}"))?,
                None => print!("{}", rep.text),
            }
            let l = &rep.l0;
            eprintln!(
                "L0: {} ({} frames vs {}, max diff {}, {} samples differ, rms diff {:.3} of rms {:.1})",
                if l.identical() { "SAMPLE IDENTICAL" } else if l.within_one_lsb() { "within 1 LSB" } else { "different" },
                l.frames_a, l.frames_b, l.max_diff, l.differing, l.rms_diff, l.rms_ref
            );
            let mut ok = l.identical();
            if let Some((same, diff)) = &rep.same_as {
                eprintln!("L1 same_as: {same}");
                for d in diff {
                    eprintln!("  L2 {d}");
                }
                ok = ok || *same;
            }
            Ok(if ok { 0 } else { 1 })
        }
        "render" => {
            let (p, o) = (positional.first().ok_or("render needs a program")?, positional.get(1).ok_or("render needs an output")?);
            let text = std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?;
            let pcm = btwd.render(&text)?;
            std::fs::write(o, wav::encode(&pcm)).map_err(|e| format!("{o}: {e}"))?;
            Ok(0)
        }
        _ => Err(USAGE.to_string()),
    };
    let _ = std::fs::remove_dir_all(&scratch);
    result
}
