//! The `listen` tool: what a program sounds like, without the audio. beatscript's `listen` (crates/beatscript-cli/src/listen.rs, linked
//! in by path as the render is) renders it offline through the same engine and answers a small JSON summary: loudness per
//! deck, part and section, the parts that have notes and are not heard (and why, when the mixer says), the detected tempo
//! and key next to the declared ones, and clipping.
//!
//! Read-only and open to everyone (no sign-in): it stores nothing. It is limited like `render` (per caller an hour, four
//! times that per address), at most [`CONCURRENT`] run at once, and answers are cached by (CID, section, bars), so asking
//! again is free. A child of `tools` so it shares the argument rules and the checker.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::{bad, is_cid, name_ok, needs_fixing, private, q, sprop, tool, Args, Caller, Fail, Rep, Tools, CID_PAT, IP_CEILING, NAME_PAT, R};
use crate::mcp::limiter::Limiter;

/// Listens running at once (each is a render or a few: CPU for seconds).
pub const CONCURRENT: usize = 2;
/// Answers kept.
const CACHE: usize = 256;
/// Bars a listen renders at most (from the program's start), and seconds of audio its part renders may take.
const MAX_BARS: u32 = 96;
const PART_BUDGET_SECONDS: f64 = 240.0;

pub struct Listen {
    per_hour: usize,
    lim: Limiter,
    lim_ip: Limiter,
    running: AtomicUsize,
    cache: Mutex<HashMap<String, (Instant, Value)>>,
}

impl Listen {
    pub fn new(per_hour: usize) -> Listen {
        let hour = Duration::from_secs(3600);
        Listen { per_hour, lim: Limiter::new(per_hour, hour), lim_ip: Limiter::new(IP_CEILING * per_hour, hour), running: AtomicUsize::new(0), cache: Mutex::new(HashMap::new()) }
    }

    fn cached(&self, key: &str) -> Option<Value> {
        self.cache.lock().unwrap().get(key).map(|(_, v)| v.clone())
    }

    fn keep(&self, key: String, v: Value) {
        let mut c = self.cache.lock().unwrap();
        if c.len() >= CACHE {
            if let Some(old) = c.iter().min_by_key(|(_, (t, _))| *t).map(|(k, _)| k.clone()) {
                c.remove(&old);
            }
        }
        c.insert(key, (Instant::now(), v));
    }
}

/// tools/list's entry.
pub fn tool_def(max_prog: usize, per_hour: usize) -> Value {
    let desc = format!(
        "Listen to a BeatScript program without audio: it is rendered offline through the same engine as render, and you get a \
small JSON summary to check your work by ear, as numbers. Per deck and per part (a role of a deck's line: drums, bass, ...): \
RMS and peak in dBFS and loudness in LUFS. `silent` lists every deck or part that has notes in the window but is not heard \
(under -60 LUFS), with `reason` when the mixer explains it (master_zero, mute, level_0, crossfader, filter_closed, \
eq_killed; voice_missing when the mixer lets a part through and its voice made nothing; no_sound otherwise) and `why` in \
words. `detected` gives the tempo and key heard against the declared ones (bpm_match same|half|double|two_thirds|three_halves|differs; \
key_match same|relative|differs|unsure|undeclared). `sections` gives each section's loudness (and `control_off` on a section an `arrange {{ }}` switch such as `~ control off` \
plays with lines of its control deck held back: the calls that do not fire there), `bar_lufs` a loudness per bar, and \
master.clipped_samples counts samples over full scale. `warnings` is the short list to act on. A part that came from a \
reference (a gen@2 call, a seed(...), a template, a stored program's section) says so in `from` {{call, kind, resolved, place, \
generator, sections}} (`also_from` when several made it); a gen call fitted to the program's key or tempo says so in `notes`, \
in check's words; and the answer gains `expanded_cid` and `resolution`, as check's. Give exactly one of text, \
cid, example; optionally `section` (listen to that section only) and `bars` (at most this many bars of the window). \
Nothing is stored and no sign-in is needed. At most {per_hour} new listens an hour per caller; the same program, section \
and bars again is answered from the cache. Programs are at most {max_prog} bytes; the render covers at most {MAX_BARS} \
bars from the start. Like render, the offline engine plays the notes, the mixer and insert moves it has ops for and the \
`macro` lines (the DJ macro catalogue: see the macros tool); `notes` names the control lines it leaves out. A program with \
a macro line gains `macros`: each call in the window, `at_bar` (the bar the engine starts it on, as the time words count), \
and `played`, with `refused` (the engine's reason, also a warning) when it was not."
    );
    tool(
        "listen",
        "Listen to a program (a summary, no audio)",
        desc,
        [true, false, true, false],
        vec![
            ("text", sprop("The program text. It must pass check. Not stored.", &[("maxLength", json!(max_prog))])),
            ("cid", sprop("CID of a stored program or an example.", &[("maxLength", json!(100)), ("pattern", json!(CID_PAT))])),
            ("example", sprop("Name of an example, from list_examples.", &[("maxLength", json!(64)), ("pattern", json!(NAME_PAT))])),
            ("section", sprop("Listen to this section only (its name in the program).", &[("maxLength", json!(64))])),
            ("bars", json!({"type": "integer", "minimum": 1, "maximum": MAX_BARS, "description": "At most this many bars of the window, from its start."})),
        ],
        &[],
    )
}

impl Tools {
    pub(super) fn listen_tool(&self, c: &Caller, a: Args) -> R {
        a.only(&["text", "cid", "example", "section", "bars"])?;
        let mut text = a.str("text", self.max_prog)?.unwrap_or_default();
        let cid = a.str("cid", 100)?.unwrap_or_default();
        let example = a.str("example", 64)?.unwrap_or_default();
        let n = [!text.trim().is_empty(), !cid.is_empty(), !example.is_empty()].iter().filter(|b| **b).count();
        if n != 1 {
            return Err(bad("give exactly one of text, cid, example"));
        }
        let section = a.str("section", 64)?.filter(|s| !s.trim().is_empty()).map(|s| s.trim().to_string());
        let bars = a.num("bars", 1.0, f64::from(MAX_BARS))?.map(|b| b as u32);
        if !cid.is_empty() {
            if !is_cid(&cid) {
                return Err(bad("argument \"cid\" is not a CID (it starts with z)"));
            }
            text = match self.stored(&cid) {
                Some((t, ..)) => t,
                None => match self.app.examples.list.iter().find(|e| e.cid == cid).and_then(|e| self.example_text(&e.name)) {
                    Some(t) => t,
                    None => return Err(Fail::new(404, "not_found", format!("no stored program has CID {cid}. Pass its text as `text`."))),
                },
            };
        } else if !example.is_empty() {
            if !name_ok(&example) {
                return Err(bad("argument \"example\" is an example name, as list_examples gives"));
            }
            text = self.example_text(&example).ok_or_else(|| Fail::new(404, "not_found", format!("no example is named {}; call list_examples", q(&example))))?;
        }
        let rep = self.check_program(c, &text)?;
        if !Rep(&rep).ok() {
            return Err(needs_fixing(&rep, "it cannot be listened to"));
        }
        if let Some(e) = private(&rep, "it is not rendered here") {
            return Err(e);
        }
        let pcid = rep["cid"].as_str().unwrap_or("").to_string();
        let lexicon = self.harvest(&text, Some(&rep));
        let key = format!("{pcid}|{}|{}", section.as_deref().unwrap_or(""), bars.map_or(String::new(), |b| b.to_string()));
        if let Some(v) = (!pcid.is_empty()).then(|| self.listen.cached(&key)).flatten() {
            let mut v = v;
            v["cached"] = json!(true);
            v["lexicon"] = lexicon;
            return Ok(v);
        }
        let l = &self.listen;
        if let Err(r) = l.lim.allow(&c.key).and_then(|_| self.ip_ceiling(&l.lim_ip, c)) {
            return Err(Fail::from(crate::ApiError::new(429, "rate_limited", format!("at most {} new listens an hour; the same program again is answered from the cache", l.per_hour)).retry(r)));
        }
        if l.running.fetch_add(1, Ordering::SeqCst) >= CONCURRENT {
            l.running.fetch_sub(1, Ordering::SeqCst);
            return Err(Fail::from(crate::ApiError::new(429, "busy", "the listeners are busy; try again in a few seconds").retry(5)));
        }
        struct Done<'a>(&'a AtomicUsize);
        impl Drop for Done<'_> {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let _done = Done(&l.running);
        let opts = crate::listen::ListenOptions { section, bars, max_bars: MAX_BARS, budget_seconds: PART_BUDGET_SECONDS, sr: crate::listen::SR };
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| crate::listen::listen(&text, &opts)));
        let v = match r {
            Ok(Ok(v)) => v,
            Ok(Err(e)) if e.code == "usage" => return Err(bad(e.message)),
            Ok(Err(e)) if e.code == "restricted" || e.code == "uses_assets" => return Err(Fail::new(422, e.code, e.message)),
            Ok(Err(e)) => return Err(Fail::new(422, "listen_failed", e.message)),
            Err(_) => return Err(Fail::new(500, "internal", "the listen could not complete; try again")),
        };
        let mut v: Value = serde_json::from_str(&v.js_stringified()).map_err(|e| Fail::new(500, "internal", e.to_string()))?;
        v["license"] = json!("CC0-1.0");
        if !pcid.is_empty() {
            l.keep(key, v.clone());
        }
        v["cached"] = json!(false);
        v["lexicon"] = lexicon;
        Ok(v)
    }
}

/// `get`: the reported mixer (`mixer`, or null from a page or phone too old to send it) and what it means, as `warnings`:
/// a playing deck the listener cannot hear ("deck B inaudible: the crossfader is all the way to A ..."), a deck heard
/// through the vocoder. The reasons are listen's ([`crate::listen::inaudible`]), so both tools say the same thing.
pub(super) fn mixer_report(out: &mut serde_json::Map<String, Value>) {
    let m = match out.get("mixer") {
        Some(m @ Value::Object(_)) => m.clone(),
        _ => {
            out.insert("mixer".into(), Value::Null);
            return;
        }
    };
    let Ok(jv) = djengine::json::JsonValue::parse(&m.to_string()) else { return };
    let playing = out.get("playing").and_then(Value::as_bool).unwrap_or(false);
    let mut warnings: Vec<Value> = Vec::new();
    if let Some(decks) = m.get("decks").and_then(Value::as_object) {
        for (name, d) in decks {
            // a deck that is not playing is silent on purpose
            if !playing || d.get("playing").and_then(Value::as_bool) == Some(false) {
                continue;
            }
            let Some(mix) = crate::listen::deck_mix_from_json(&jv, name) else { continue };
            if let Some((_, why)) = crate::listen::inaudible(name, &mix) {
                warnings.push(json!(format!("deck {name} inaudible: {why}")));
            } else if mix.vocoder {
                warnings.push(json!(format!("deck {name} is heard through the vocoder (it is the carrier), not on its own")));
            }
        }
    }
    out.insert("warnings".into(), Value::Array(warnings));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_warns_of_a_deck_the_crossfader_shuts() {
        let mut out = serde_json::Map::new();
        out.insert("playing".into(), json!(true));
        out.insert("mixer".into(), json!({"crossfader": 0.0, "master": 1.0, "decks": {
            "A": {"playing": true, "volume": 0.8, "mute": false, "crossfade_gain": 1.0, "kills": [], "filter": 0.5, "insert_filter": null, "vocoder": false},
            "B": {"playing": true, "volume": 0.8, "mute": false, "crossfade_gain": 0.0, "kills": [], "filter": 0.5, "insert_filter": null, "vocoder": false}}}));
        mixer_report(&mut out);
        let w = out["warnings"].as_array().unwrap();
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].as_str().unwrap().starts_with("deck B inaudible: the crossfader is all the way to A"), "{w:?}");
    }

    #[test]
    fn an_old_client_gets_null_and_no_warning() {
        let mut out = serde_json::Map::new();
        out.insert("playing".into(), json!(true));
        mixer_report(&mut out);
        assert_eq!(out["mixer"], Value::Null);
        assert!(out.get("warnings").is_none());
    }

    #[test]
    fn a_stopped_deck_is_not_a_warning() {
        let mut out = serde_json::Map::new();
        out.insert("playing".into(), json!(true));
        out.insert("mixer".into(), json!({"crossfader": 0.0, "master": 1.0, "decks": {"B": {"playing": false, "volume": 0.0, "crossfade_gain": 0.0}}}));
        mixer_report(&mut out);
        assert!(out["warnings"].as_array().unwrap().is_empty());
    }

    #[test]
    fn the_tool_is_listed() {
        let d = tool_def(16384, 30);
        assert_eq!(d["name"], "listen");
        assert_eq!(d["annotations"]["readOnlyHint"], true);
    }
}
