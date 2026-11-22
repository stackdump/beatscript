//! The diagnostics log (roadmap 6659), first slice: **the lexicon harvest** (6665, 6666; docs/btw/lexicon-harvest.md).
//!
//! The BeatScript MCP tools that check a program hand the lyric pairs of what they checked here, as a side effect: every
//! annotated escape (`jingle=/j ii ng g uh l/`, a strong signal: someone wrote the pronunciation down) and every word no
//! lexicon knows with the rules' pronunciation (a weak one), from `djengine::vocal::lexicon::lyric_pairs`. The harvest
//! keeps, per word, each pronunciation it was given: its source, how many programs gave it, when it was first and last
//! given, and up to [`EXAMPLES`] of those programs' CIDs. A weekly job ranks it into a proposed CC0 lexicon diff for a
//! person to approve (`jobs/lexicon-harvest-weekly.md`).
//!
//! **What it never holds:** program text (a stored CC0 program is re-derivable from its CID; an unstored one is not, by
//! design), anything about a caller (no account, token or address), or anything at all from `push_private`, a program
//! that uses samples or loops, `use restricted`, or a `#! visibility:` other than public: [`Harvest::record`] is never
//! called for those (`mcp::tools::Tools::harvest` holds the gate).
//!
//! **The file** is append-only JSON lines, `<data>/diagnostics/lexicon-harvest.jsonl`, one record a new (word,
//! phonemes, source, program) sighting:
//!
//! `{"kind":"lexicon_pair","t":1795372800,"word":"jingle","phones":"j ii ng g uh l","source":"annotated","cid":"z4EB…"}`
//!
//! The same program checked again adds nothing (its sightings are already there), so `count` is the number of distinct
//! programs and the file grows only with new programs. `kind` leaves room for the rest of 6659's records in the same log.
//! The aggregate is rebuilt from the file at start.

use std::collections::{BTreeMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use djengine::vocal::lexicon::{LyricPair, PairSource};
use serde_json::{json, Value};

/// The file under `<data>/diagnostics/`.
pub const FILE: &str = "lexicon-harvest.jsonl";
/// Example program CIDs kept per pronunciation.
pub const EXAMPLES: usize = 5;
/// At most this many distinct words: past it, new words are not taken (known ones still count).
pub const MAX_WORDS: usize = 50_000;
/// At most this many pairs taken from one program.
pub const MAX_PAIRS: usize = 64;
const MAX_WORD: usize = 40;
const MAX_PHONES: usize = 160;

/// One pronunciation of a word, as the harvest has it.
#[derive(Clone, Debug, PartialEq)]
pub struct Variant {
    pub phones: String,
    pub source: PairSource,
    /// Distinct programs that gave it.
    pub count: u64,
    /// Unix seconds.
    pub first: i64,
    pub last: i64,
    /// The first [`EXAMPLES`] programs that gave it.
    pub cids: Vec<String>,
}

/// What one [`Harvest::record`] did: `harvested` pairs taken from the program, `new` of them a (word, pronunciation,
/// source) the harvest had not had from any program before.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Recorded {
    pub harvested: usize,
    pub new: usize,
}

impl Recorded {
    /// The `lexicon` field the tools' answers carry.
    pub fn json(self) -> Value {
        json!({"harvested": self.harvested, "new": self.new})
    }
}

#[derive(Default)]
struct State {
    words: BTreeMap<String, Vec<Variant>>,
    seen: HashSet<(String, String, PairSource, String)>,
}

impl State {
    /// Apply one sighting; whether the (word, phones, source) is new.
    fn apply(&mut self, word: &str, phones: &str, source: PairSource, cid: &str, t: i64) -> Option<bool> {
        if !self.seen.insert((word.to_string(), phones.to_string(), source, cid.to_string())) {
            return None;
        }
        if !self.words.contains_key(word) && self.words.len() >= MAX_WORDS {
            return None;
        }
        let vs = self.words.entry(word.to_string()).or_default();
        match vs.iter_mut().find(|v| v.phones == phones && v.source == source) {
            Some(v) => {
                v.count += 1;
                v.first = v.first.min(t);
                v.last = v.last.max(t);
                if v.cids.len() < EXAMPLES && !v.cids.iter().any(|c| c == cid) {
                    v.cids.push(cid.to_string());
                }
                Some(false)
            }
            None => {
                vs.push(Variant { phones: phones.to_string(), source, count: 1, first: t, last: t, cids: vec![cid.to_string()] });
                Some(true)
            }
        }
    }
}

/// The lexicon harvest: the aggregate in memory, the sightings on disk (or nowhere, for a test or a read-only server).
pub struct Harvest {
    path: Option<PathBuf>,
    file: Mutex<Option<File>>,
    state: Mutex<State>,
}

fn source_of(s: &str) -> Option<PairSource> {
    match s {
        "annotated" => Some(PairSource::Annotated),
        "rule" => Some(PairSource::Rules),
        _ => None,
    }
}

/// A CID as the site writes them (`mcp::tools::is_cid`): `z` and 20 to 100 base58 characters.
fn is_cid(c: &str) -> bool {
    let b = c.as_bytes();
    b.first() == Some(&b'z') && (21..=101).contains(&b.len()) && b[1..].iter().all(|c| matches!(c, b'1'..=b'9' | b'A'..=b'H' | b'J'..=b'N' | b'P'..=b'Z' | b'a'..=b'k' | b'm'..=b'z'))
}

fn ok_word(w: &str) -> bool {
    !w.is_empty() && w.len() <= MAX_WORD && w.bytes().all(|b| b.is_ascii_lowercase() || b == b'\'')
}

fn ok_phones(p: &str) -> bool {
    !p.is_empty() && p.len() <= MAX_PHONES && p.split(' ').all(|s| djengine::vocal::phoneme::phoneme(s).is_some())
}

impl Harvest {
    /// A harvest that keeps nothing on disk.
    pub fn memory() -> Harvest {
        Harvest { path: None, file: Mutex::new(None), state: Mutex::new(State::default()) }
    }

    /// Open (or start) `<dir>/lexicon-harvest.jsonl` and rebuild the aggregate from it. A line that does not read is
    /// skipped.
    pub fn open(dir: &Path) -> std::io::Result<Harvest> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(FILE);
        let mut st = State::default();
        if let Ok(f) = File::open(&path) {
            for line in BufReader::new(f).lines().map_while(Result::ok) {
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                if v["kind"] != "lexicon_pair" {
                    continue;
                }
                let (Some(w), Some(p), Some(s), Some(c), Some(t)) = (v["word"].as_str(), v["phones"].as_str(), v["source"].as_str().and_then(source_of), v["cid"].as_str(), v["t"].as_i64()) else { continue };
                st.apply(w, p, s, c, t);
            }
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Harvest { path: Some(path), file: Mutex::new(Some(file)), state: Mutex::new(st) })
    }

    /// Where it is kept (None: in memory).
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Record the lyric pairs of the program `cid` at `t` (unix seconds). Only the word, its phonemes, the source and the
    /// CID are kept. Call it only for a program that checked and may be kept (the gate is the caller's).
    pub fn record(&self, cid: &str, pairs: &[LyricPair], t: i64) -> Recorded {
        if !is_cid(cid) {
            return Recorded::default();
        }
        let mut out = Recorded::default();
        let mut lines = String::new();
        {
            let mut st = self.state.lock().unwrap();
            for p in pairs.iter().filter(|p| ok_word(&p.word) && ok_phones(&p.phones)).take(MAX_PAIRS) {
                out.harvested += 1;
                if let Some(new) = st.apply(&p.word, &p.phones, p.source, cid, t) {
                    out.new += usize::from(new);
                    lines.push_str(&json!({"kind": "lexicon_pair", "t": t, "word": p.word, "phones": p.phones, "source": p.source.name(), "cid": cid}).to_string());
                    lines.push('\n');
                }
            }
        }
        if !lines.is_empty() {
            if let Some(f) = self.file.lock().unwrap().as_mut() {
                if let Err(e) = f.write_all(lines.as_bytes()) {
                    eprintln!("lexicon harvest: {e}");
                }
            }
        }
        out
    }

    /// The words, most worth proposing first: a word with an annotated pronunciation before one with only the rules',
    /// then by programs, then by word. At most `n` (0: all). Each is `{word, programs, conflict, variants: [{phones,
    /// source, count, first, last, cids}]}`, the variants annotated first and then by count; `conflict` is whether two
    /// annotated pronunciations disagree (a person decides).
    pub fn top(&self, n: usize) -> Vec<Value> {
        let st = self.state.lock().unwrap();
        let mut rows: Vec<(bool, u64, &String, &Vec<Variant>)> = st
            .words
            .iter()
            .map(|(w, vs)| (vs.iter().any(|v| v.source == PairSource::Annotated), vs.iter().map(|v| v.count).sum::<u64>(), w, vs))
            .collect();
        rows.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(a.2.cmp(b.2)));
        rows.into_iter()
            .take(if n == 0 { usize::MAX } else { n })
            .map(|(_, programs, w, vs)| {
                let mut vs = vs.clone();
                vs.sort_by(|a, b| a.source.cmp(&b.source).then(b.count.cmp(&a.count)).then(a.phones.cmp(&b.phones)));
                let annotated: HashSet<&str> = vs.iter().filter(|v| v.source == PairSource::Annotated).map(|v| v.phones.as_str()).collect();
                json!({
                    "word": w,
                    "programs": programs,
                    "conflict": annotated.len() > 1,
                    "in_lexicon": djengine::vocal::phoneme::lexicon_lookup(w).map(|p| djengine::vocal::phoneme::spell(&p)),
                    "variants": vs.iter().map(|v| json!({"phones": v.phones, "source": v.source.name(), "count": v.count, "first": v.first, "last": v.last, "cids": v.cids})).collect::<Vec<_>>(),
                })
            })
            .collect()
    }

    /// How many words it holds.
    pub fn len(&self) -> usize {
        self.state.lock().unwrap().words.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use djengine::vocal::lexicon::lyric_pairs;

    const CID_A: &str = "z4EBG9jAWvWyiVNF43gMjU5wYinuLmnAfWLhAVK41gqupdYGvVo";
    const CID_B: &str = "z4EBG9j2k5FjGg1aWvWyiVNF43gMjU5wYinuLmnAfWLhAVK41gq";

    #[test]
    fn a_pair_is_recorded_with_its_word_phonemes_source_and_cid_and_no_text() {
        let dir = tempdir();
        let lyric = "zorblax=/z oh r b l ae k s/ on the road";
        let h = Harvest::open(&dir).unwrap();
        assert_eq!(h.record(CID_A, &lyric_pairs(lyric), 100), Recorded { harvested: 1, new: 1 });
        // the same program again adds nothing; another program counts
        assert_eq!(h.record(CID_A, &lyric_pairs(lyric), 200), Recorded { harvested: 1, new: 0 });
        assert_eq!(h.record(CID_B, &lyric_pairs(lyric), 300), Recorded { harvested: 1, new: 0 });
        let file = std::fs::read_to_string(dir.join(FILE)).unwrap();
        assert_eq!(file.lines().count(), 2, "{file}");
        assert!(!file.contains("on the road") && !file.contains("/z oh") && !file.contains("road"), "no program text: {file}");
        let top = h.top(0);
        assert_eq!(top.len(), 1);
        assert_eq!(top[0]["word"], "zorblax");
        assert_eq!(top[0]["variants"][0], json!({"phones": "z oh r b l ae k s", "source": "annotated", "count": 2, "first": 100, "last": 300, "cids": [CID_A, CID_B]}));
        // rebuilt from the file as it was
        let again = Harvest::open(&dir).unwrap();
        assert_eq!(again.top(0), top);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn annotated_words_rank_first_and_disagreeing_annotations_are_a_conflict() {
        let h = Harvest::memory();
        // a weak word in three programs, a strong one in one
        for (i, cid) in [CID_A, CID_B, "z4EBG9jBjFYJYx19WvWyiVNF43gMjU5wYinuLmnAfWLhAVK41gq"].iter().enumerate() {
            h.record(cid, &lyric_pairs("quexl"), i as i64);
        }
        h.record(CID_A, &lyric_pairs("tomato=/t uh m ay t oh/"), 9);
        h.record(CID_B, &lyric_pairs("tomato=/t uh m aa t oh/"), 10);
        let top = h.top(0);
        assert_eq!(top[0]["word"], "tomato");
        assert_eq!(top[0]["conflict"], true);
        assert_eq!(top[1]["word"], "quexl");
        assert_eq!(top[1]["conflict"], false);
        assert_eq!(top[1]["variants"][0]["source"], "rule");
        assert_eq!(top[1]["programs"], 3);
        assert_eq!(h.top(1).len(), 1);
    }

    #[test]
    fn a_bad_cid_or_bad_pair_is_not_recorded() {
        let h = Harvest::memory();
        assert_eq!(h.record("", &lyric_pairs("zorblax"), 1), Recorded::default());
        assert_eq!(h.record("not-a-cid", &lyric_pairs("zorblax"), 1), Recorded::default());
        let bad = [LyricPair { word: "Bad Word".into(), phones: "b ae d".into(), source: PairSource::Rules }, LyricPair { word: "ok".into(), phones: "zz".into(), source: PairSource::Rules }];
        assert_eq!(h.record(CID_A, &bad, 1), Recorded::default());
        assert!(h.is_empty());
    }

    fn tempdir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("btw-harvest-{}-{:?}", std::process::id(), std::thread::current().id()).replace(['(', ')'], ""));
        std::fs::remove_dir_all(&d).ok();
        d
    }
}
