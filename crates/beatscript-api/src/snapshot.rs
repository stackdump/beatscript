//! What BeatScript is checked against on the web, as static data: the control-language registry, the engine's
//! instrument names, and the programs the host knows by CID (known.rs). `data/snapshot.json` is generated from djhost's own tables by
//! `examples/gen_snapshot.rs`, and `tests/snapshot.rs` fails when it drifts from them, so the pure crate
//! (which builds for wasm32) gives the diagnostics and CIDs the desktop app does.
//!
//! No `sample` / `loop` file is looked up anywhere (`Env::assets` is `None`): a program is checked from its
//! text and the programs it is given. There is no snippet store of names here: an `import` takes a CID, whose text the host
//! was told (a link carries it; a page fetches it), and a program that is not here says so (`env::not_here`).

use std::sync::OnceLock;

use djengine::json::JsonValue;
use djlive::env::{ActionSpec, Catalog, Env, NounSpec, ParamSpec, Registry};

/// The committed snapshot.
pub const SNAPSHOT_JSON: &str = include_str!("../data/snapshot.json");

/// The message an `import` of a name gets here (the diagnostic's help is replaced by [`IMPORT_HELP`]): a name needs a snippet
/// store, and there is none. (A CID is another matter: its text is given to the host, or it is not here. The words are pinned
/// by `tests/cid-pins.json`: a program that imports by name has them in its `nets` answer.)
pub const NO_IMPORT: &str = "`import` is not available on the web: there is no snippet store here";
pub const IMPORT_HELP: &str = "paste the snippet's `let` and `def` lines into the program instead";

struct Noun {
    spec: NounSpec,
    aliases: Vec<String>,
    local: bool,
}

/// The control-language registry as `djhost::beatscript_env::RegistryView` answers it, from the snapshot.
pub struct SnapRegistry {
    nouns: Vec<Noun>,
}

impl Registry for SnapRegistry {
    fn noun(&self, name: &str) -> Option<NounSpec> {
        // the same lookup as the real table: the first noun (in table order) with that name or alias
        self.nouns.iter().find(|n| n.spec.name == name || n.aliases.iter().any(|a| a == name)).map(|n| n.spec.clone())
    }

    fn noun_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.nouns.iter().filter(|n| !n.local).map(|n| n.spec.name.clone()).collect();
        v.extend(self.nouns.iter().filter(|n| !n.local).flat_map(|n| n.aliases.iter().cloned()));
        v
    }

    fn plays(&self, noun: &str, params: &JsonValue) -> Result<(), String> {
        browser_plays(noun, params)
    }

    fn engine_name(&self) -> &str {
        "the browser"
    }
}

/// What the browser's engine plays of a registry call: the mirror of site/web/js/engine-calls.js (`mapCall`), whose
/// HONOURED list this follows (tests/capabilities.rs pins the two together through the cases there). Err: what it does
/// not do, as `check` says it.
pub fn browser_plays(noun: &str, p: &JsonValue) -> Result<(), String> {
    let s = |k: &str| p.get(k).and_then(JsonValue::as_str);
    let has = |k: &str| p.get(k).is_some();
    let deck_ok = |d: Option<&str>| matches!(d, Some("A" | "B"));
    match noun {
        "mixer" => {
            for k in ["pfl", "aux", "headphone_mix", "headphone_level"] {
                if has(k) {
                    return Err(format!("the mixer's `{k}` (the browser has no cue or aux bus)"));
                }
            }
            if ["volume", "gain", "filter", "low", "mid", "high", "mute"].iter().any(|k| has(k)) && !deck_ok(s("deck")) {
                return Err("a mixer strip other than deck A or B".into());
            }
            Ok(())
        }
        "deck" => match (deck_ok(s("deck")), s("action")) {
            (true, Some("play" | "stop" | "pause")) => Ok(()),
            (false, _) => Err("a deck other than A or B".into()),
            (_, a) => Err(format!("`deck {}` (the browser plays, stops and pauses a deck)", a.unwrap_or("?"))),
        },
        "inserts" => {
            let target = s("target").unwrap_or("");
            let param = s("param").unwrap_or("");
            let effect = s("effect").map(str::to_string).or_else(|| param.split('.').next().map(str::to_string)).unwrap_or_default();
            let off = s("action") == Some("on");
            let master = target == "Master";
            if !(master || deck_ok(Some(target))) {
                return Err(format!("inserts on `{target}`"));
            }
            match effect.as_str() {
                // the booth's insert filter itself (djrust `live_op("inserts", ..)`): its sweep and its resonance, at the same curves
                "filter" if off || param == "filter.sweep" || param == "filter.res" => Ok(()),
                "reverb" if off || param == "reverb.mix" => Ok(()),
                "crush" if master && (off || param == "crush.mix") => Ok(()),
                _ if off && p.get("on").and_then(JsonValue::as_bool) == Some(false) => Ok(()),
                "crush" => Err("the bitcrusher on a deck (the browser has it on the master)".into()),
                "delay" => Err("the delay insert".into()),
                "filter" => Err(format!("the filter's `{param}` (the browser sweeps it and sets its resonance)")),
                e => Err(format!("the `{e}` insert")),
            }
        }
        // the FX unit: the browser plays its drive on deck A, deck B or both (`master: drive`), as `perform {op: fx}`
        "fx" => match (s("action"), s("param"), s("deck")) {
            (Some("set"), Some("drive"), None | Some("A" | "B")) => Ok(()),
            (Some("set"), Some("drive"), Some(d)) => Err(format!("the drive of deck `{d}` (the browser has the decks A and B)")),
            (Some("set"), Some(p), _) => Err(format!("the FX unit's `{p}` (the browser plays its drive)")),
            (a, _, _) => Err(format!("`fx {}` (the browser plays the FX unit's drive: `deck A: drive 0.6`)", a.unwrap_or("?"))),
        },
        "vocoder" => match s("action").unwrap_or("set") {
            "set" | "on" | "swap" | "add" | "remove" => Ok(()),
            a => Err(format!("`vocoder {a}` (the browser sets, swaps, adds and removes vocoders)")),
        },
        "moves" => match (s("action"), s("kind")) {
            (Some("arm"), Some("vocoder_handoff")) => Ok(()),
            _ => Err(format!("the `{}` move (of the moves the browser plays vocoder_handoff only)", s("kind").or(s("name")).unwrap_or("?"))),
        },
        "vocal" => Err("a timed or performed `sing` line, or a `do vocal` line: the browser sings a `sing` line that starts with its section or the program".into()),
        // a `macro` line (docs/beatscript/ext-control.md): named as the line names it, as the page's "not played here" does
        "effects" => match (s("action"), s("id")) {
            (Some("fire" | "release"), Some(id)) => Err(format!("`macro {id}` (the browser plays none of the DJ macro catalogue; render and listen do)")),
            (a, _) => Err(format!("`effects {}` (the Effects window is the booth's)", a.unwrap_or("?"))),
        },
        // the Wheel: windwheel's tonewheel, the booth's own instrument (djweb src/wheel.rs), every action of the noun
        // (engine-calls.js `wheel`); `status` and `presets` ask and play nothing
        "wheel" => Ok(()),
        other => Err(format!("`{other}` (the browser's engine has no such unit)")),
    }
}

/// The engine's sound names, from the snapshot.
pub struct SnapCatalog {
    pub voices: Vec<String>,
    pub families: Vec<String>,
    pub kits: Vec<String>,
    pub palettes: Vec<String>,
}

impl Catalog for SnapCatalog {
    fn voices(&self) -> Vec<String> {
        self.voices.clone()
    }
    fn families(&self) -> Vec<String> {
        self.families.clone()
    }
    fn kits(&self) -> Vec<String> {
        self.kits.clone()
    }
    fn palettes(&self) -> Vec<String> {
        self.palettes.clone()
    }
}

/// Everything BeatScript is checked against on the web, to lend as an `Env`.
pub struct WebEnv {
    pub registry: SnapRegistry,
    pub catalog: SnapCatalog,
    pub snippets: crate::known::Known,
}

impl WebEnv {
    pub fn env(&self) -> Env<'_> {
        Env { registry: Some(&self.registry), catalog: Some(&self.catalog), snippets: Some(&self.snippets), assets: None, private: false, voices: Some(web_voices()), ontology: None, bank: None }
    }

    /// The voices, families, kits and palettes as `btw instruments` answers them (the same JSON as
    /// `djhost::beatscript_env::instruments_json`).
    pub fn instruments(&self) -> JsonValue {
        let strs = |v: &[String]| JsonValue::Array(v.iter().cloned().map(JsonValue::str).collect());
        JsonValue::Object(djengine::json::JsonObject::from_pairs([
            ("voices", strs(&self.catalog.voices)),
            ("families", strs(&self.catalog.families)),
            ("kits", strs(&self.catalog.kits)),
            ("palettes", strs(&self.catalog.palettes)),
            (
                "note",
                JsonValue::str("inst=NAME takes a voice (its own sound) or contains a family word (it plays that family's generic voice, and says so); kit=NAME a drum kit; sound=OPTION a role option (own, synth, synth:NAME, inst:NAME); acid and acid-square are a 303-style voice: an accent (!) is louder and opens its filter further, a slide (^) glides into the next note without striking it again (60 ms, or glide=), and decay= is its filter's decay"),
            ),
        ]))
    }
}

/// The web environment, built once.
pub fn web_env() -> &'static WebEnv {
    static ENV: OnceLock<WebEnv> = OnceLock::new();
    ENV.get_or_init(|| {
        // a `gen` call resolves against the seeded generator: every check made here can make one (djlive::gen)
        crate::generate::install();
        parse(SNAPSHOT_JSON).expect("data/snapshot.json is valid (a test parses it)")
    })
}

fn strings(v: Option<&JsonValue>) -> Vec<String> {
    v.and_then(JsonValue::strings).unwrap_or_default()
}

fn params(v: Option<&JsonValue>) -> Vec<ParamSpec> {
    v.and_then(JsonValue::as_array)
        .map(|a| {
            a.iter()
                .map(|p| ParamSpec {
                    name: p.get("name").and_then(JsonValue::as_str).unwrap_or("").to_string(),
                    types: strings(p.get("types")),
                    options: strings(p.get("options")),
                    required: p.get("required").and_then(JsonValue::as_bool).unwrap_or(false),
                    doc: String::new(),
                    minimum: p.get("minimum").and_then(JsonValue::as_f64),
                    maximum: p.get("maximum").and_then(JsonValue::as_f64),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Read a snapshot (the committed one, or a candidate in a test).
pub fn parse(text: &str) -> Result<WebEnv, String> {
    let j = JsonValue::parse(text).map_err(|e| e.to_string())?;
    if j.get("version").and_then(JsonValue::as_i64) != Some(1) {
        return Err("unknown snapshot version".into());
    }
    let nouns = j
        .get("nouns")
        .and_then(JsonValue::as_array)
        .ok_or("the snapshot has no nouns")?
        .iter()
        .map(|n| Noun {
            spec: NounSpec {
                name: n.get("name").and_then(JsonValue::as_str).unwrap_or("").to_string(),
                default_action: n.get("default_action").and_then(JsonValue::as_str).unwrap_or("").to_string(),
                actions: n
                    .get("actions")
                    .and_then(JsonValue::as_array)
                    .map(|a| {
                        a.iter()
                            .map(|a| ActionSpec { name: a.get("name").and_then(JsonValue::as_str).unwrap_or("").to_string(), effect: a.get("effect").and_then(JsonValue::as_str).unwrap_or("").to_string(), params: params(a.get("params")) })
                            .collect()
                    })
                    .unwrap_or_default(),
                params: params(n.get("params")),
            },
            aliases: strings(n.get("aliases")),
            local: n.get("local").and_then(JsonValue::as_bool).unwrap_or(false),
        })
        .collect();
    Ok(WebEnv {
        registry: SnapRegistry { nouns },
        catalog: SnapCatalog { voices: strings(j.get("voices")), families: strings(j.get("families")), kits: strings(j.get("kits")), palettes: strings(j.get("palettes")) },
        snippets: crate::known::Known,
    })
}

/// The browser's voices: the formant presets (the engine's own), no neural voice, and a `sing` line sung as a part of
/// its deck's net (the browser has no `vocal` noun: djlive sing.rs, "In the nets").
pub struct WebVoices(djlive::env::StaticVoices);

impl djlive::env::Voices for WebVoices {
    fn names(&self) -> Vec<String> {
        self.0.names()
    }
    fn is_neural(&self, name: &str) -> bool {
        self.0.is_neural(name)
    }
    fn neural_installed(&self) -> bool {
        false
    }
    fn default_voice(&self) -> String {
        self.0.default_voice()
    }
    fn fallback_voice(&self) -> String {
        self.0.fallback_voice()
    }
    fn sings_in_nets(&self) -> bool {
        true
    }
}

/// The browser's voices (one, made once).
pub fn web_voices() -> &'static WebVoices {
    static V: OnceLock<WebVoices> = OnceLock::new();
    V.get_or_init(|| {
        let formant: Vec<String> = djengine::fx::formant::PRESETS.iter().map(|p| p.name.to_string()).collect();
        WebVoices(djlive::env::StaticVoices { formant, ..Default::default() })
    })
}
