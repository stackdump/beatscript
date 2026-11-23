//! What BeatScript is checked against, from the real tables: the control-language registry (the same one
//! `describe`, the MCP tools and docs/CONTROL.md are made from), the engine's instrument names, and
//! the saved snippets an `import` reads. Plain adapters to `djlive::env`'s traits, so the language
//! crate stays pure.

use djengine::json::JsonValue;
use djlive::env::{ActionSpec, Catalog, Env, NounSpec, ParamSpec, Registry, Snippets, Voices};

use crate::control_registry::{self, Noun};

/// The registry as BeatScript sees it.
pub struct RegistryView;

fn param_spec(p: &control_registry::Param) -> ParamSpec {
    ParamSpec { name: p.name.to_string(), types: p.ty.split('|').map(str::to_string).collect(), options: Vec::new(), required: p.required, doc: p.doc.to_string(), ..Default::default() }
}

/// The schema's properties of a noun, with their types and allowed words.
fn schema_params(n: &Noun) -> Vec<ParamSpec> {
    let schema = n.input_schema();
    let Some(props) = schema.get("properties").and_then(JsonValue::as_object) else { return Vec::new() };
    let mut out = Vec::new();
    for (name, v) in props.iter() {
        if name == "action" {
            continue;
        }
        let types: Vec<String> = match v.get("type") {
            Some(JsonValue::String(t)) => vec![t.clone()],
            Some(JsonValue::Array(ts)) => ts.iter().filter_map(JsonValue::as_str).map(str::to_string).collect(),
            _ => Vec::new(),
        };
        let options: Vec<String> = match v.get("enum") {
            Some(JsonValue::Array(es)) => es.iter().filter_map(JsonValue::as_str).map(str::to_string).collect(),
            _ => Vec::new(),
        };
        let doc = v.get("description").and_then(JsonValue::as_str).unwrap_or("").to_string();
        let (minimum, maximum) = (v.get("minimum").and_then(JsonValue::as_f64), v.get("maximum").and_then(JsonValue::as_f64));
        out.push(ParamSpec { name: name.to_string(), types, options, required: false, doc, minimum, maximum });
    }
    // params the verbs declare that the schema does not list
    for a in n.actions {
        for p in a.params {
            if !out.iter().any(|x| x.name == p.name) {
                out.push(param_spec(p));
            }
        }
    }
    out
}

impl Registry for RegistryView {
    fn noun(&self, name: &str) -> Option<NounSpec> {
        let n = control_registry::noun(name)?;
        Some(NounSpec {
            name: n.name.to_string(),
            default_action: n.default_action.to_string(),
            actions: n
                .actions
                .iter()
                .map(|a| ActionSpec { name: a.name.to_string(), effect: a.effect.name().to_string(), params: a.params.iter().map(param_spec).collect() })
                .collect(),
            params: schema_params(n),
        })
    }

    fn noun_names(&self) -> Vec<String> {
        let mut v: Vec<String> = control_registry::nouns().iter().filter(|n| !n.local).map(|n| n.name.to_string()).collect();
        v.extend(control_registry::nouns().iter().filter(|n| !n.local).flat_map(|n| n.aliases.iter().map(|a| a.to_string())));
        v
    }
}

/// The words that make an instrument name play a family's generic voice (`wave::synth::tonal_voice`
/// matches a name by these, so `reese` and `sub-bass` and `warm-pad` are heard though they have no
/// entry of their own). A test pins this list to the engine.
pub const FAMILIES: [&str; 29] = [
    "pad", "strings", "choir", "organ", "sub", "808", "bass", "reese", "acid", "bell", "vibes", "marimba", "kalimba", "music-box", "steel", "glass", "square", "chiptune", "pwm", "clav", "pluck", "stab", "piano", "guitar", "harpsi", "drums",
    "kick", "snare", "hat",
];

/// The engine's sound names.
pub struct CatalogView;

impl Catalog for CatalogView {
    fn voices(&self) -> Vec<String> {
        let mut v: Vec<String> = djengine::wave::instruments::InstrumentTable::shared().names().into_iter().map(str::to_string).collect();
        v.extend(djengine::fx::formant::instrument_names().iter().map(|s| s.to_string()));
        v.extend(djengine::gen::tables::BeatsTables::shared().stinger_instrument_set.iter().cloned());
        v.sort();
        v.dedup();
        v
    }

    fn families(&self) -> Vec<String> {
        FAMILIES.iter().map(|s| s.to_string()).collect()
    }

    fn kits(&self) -> Vec<String> {
        djengine::wave::synth::synth_drum_kits().into_iter().map(str::to_string).collect()
    }

    fn palettes(&self) -> Vec<String> {
        djengine::sound::PaletteSet::shared().palettes.iter().map(|p| p.id.clone()).collect()
    }
}

/// The singing voices as BeatScript sees them (`sing ... voice=NAME`, `vocal create preset=NAME`): the formant presets and the neural
/// voices (`vocals::NEURAL_VOICES`), whether the neural one is installed (it is not: there is no neural engine in this build,
/// `vocals::NEURAL_ENGINE_BUILT`), and the dictionary's own syllable counts, so a `sing` line gives a word as many notes as the
/// voice will sing it in.
#[derive(Clone, Copy, Debug)]
pub struct VoicesView {
    /// The neural voice can sing here. Set by [`VoicesView::probe`]; a test sets it to see the other answer.
    pub neural: bool,
    /// A `sing` line is sung as a part of its deck's net (djlive sing.rs, "In the nets"), as the browser sings it: the
    /// offline render (beatscript render, listen) sets it, so a render sings what the page sings. The app keeps the vocal noun.
    pub in_nets: bool,
}

impl VoicesView {
    /// What this machine has.
    pub fn probe() -> VoicesView {
        VoicesView { neural: crate::vocals::neural_installed(), in_nets: false }
    }
}

impl Voices for VoicesView {
    fn names(&self) -> Vec<String> {
        crate::vocals::voice_names()
    }

    fn is_neural(&self, name: &str) -> bool {
        crate::vocals::is_neural_voice(name)
    }

    fn neural_installed(&self) -> bool {
        self.neural
    }

    fn default_voice(&self) -> String {
        crate::vocals::default_voice(self.neural).to_string()
    }

    fn fallback_voice(&self) -> String {
        crate::vocals::FALLBACK_VOICE.to_string()
    }

    fn sung_syllables(&self, word: &str) -> Option<usize> {
        Some(djengine::vocal::fit::syllables(word).len()).filter(|n| *n > 0)
    }

    fn sings_in_nets(&self) -> bool {
        self.in_nets
    }
}

/// Everything BeatScript is checked against, to lend as an `Env`.
pub struct BeatscriptEnv<'a> {
    pub registry: RegistryView,
    pub catalog: CatalogView,
    pub snippets: Option<&'a dyn Snippets>,
    pub voices: VoicesView,
    /// How many knobs and pads the connected controllers have (F13): `knob(N)` and `pad(N)` are checked against it. None: the default.
    pub bank: Option<djlive::perform::InputBank>,
}

impl<'a> BeatscriptEnv<'a> {
    pub fn new(snippets: Option<&'a dyn Snippets>) -> BeatscriptEnv<'a> {
        BeatscriptEnv { registry: RegistryView, catalog: CatalogView, snippets, voices: VoicesView::probe(), bank: None }
    }

    /// The connected controllers' knob and pad counts, for the check of the performed inputs.
    pub fn with_bank(mut self, bank: djlive::perform::InputBank) -> BeatscriptEnv<'a> {
        self.bank = Some(bank);
        self
    }

    pub fn env(&self) -> Env<'_> {
        Env { registry: Some(&self.registry), catalog: Some(&self.catalog), snippets: self.snippets, voices: Some(&self.voices), ontology: Some(crate::ontology::vocabulary()), bank: self.bank, ..Env::default() }
    }
}

/// The instruments, palettes and kits as `btw instruments` answers them.
pub fn instruments_json() -> JsonValue {
    let c = CatalogView;
    let strs = |v: Vec<String>| JsonValue::Array(v.into_iter().map(JsonValue::str).collect());
    JsonValue::Object(djengine::json::JsonObject::from_pairs([
        ("voices", strs(c.voices())),
        ("families", strs(c.families())),
        ("kits", strs(c.kits())),
        ("palettes", strs(c.palettes())),
        (
            "note",
            JsonValue::str("inst=NAME takes a voice (its own sound) or contains a family word (it plays that family's generic voice, and says so); kit=NAME a drum kit; sound=OPTION a role option (own, synth, synth:NAME, inst:NAME); acid and acid-square are a 303-style voice: an accent (!) is louder and opens its filter further, a slide (^) glides into the next note without striking it again (60 ms, or glide=), and decay= is its filter's decay"),
        ),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registry_view_has_every_noun_with_its_verbs_and_params() {
        let reg = RegistryView;
        let n = reg.noun("vocal").unwrap();
        assert!(n.action("create").is_some());
        assert!(n.action("create").unwrap().params.iter().any(|p| p.name == "lyrics" && p.required));
        assert!(n.param("lyrics").is_some());
        // aliases resolve
        assert_eq!(reg.noun("livecode").unwrap().name, "btw");
        // the language's name since the rename is an alias of the noun, which keeps its upstream spelling
        assert_eq!(reg.noun("beatscript").unwrap().name, "btw");
        assert_eq!(reg.noun("live").unwrap().name, "btw");
        for n in control_registry::nouns().iter().filter(|n| !n.local) {
            assert!(reg.noun(n.name).is_some(), "{}", n.name);
        }
    }

    #[test]
    fn the_family_words_are_the_engines_own() {
        use djengine::wave::synth::tonal_voice;
        let plain = tonal_voice(Some("zzzz"), None);
        for w in FAMILIES.iter().filter(|w| !matches!(**w, "drums" | "kick" | "snare" | "hat")) {
            let v = tonal_voice(Some(w), None);
            assert!((v.level, v.attack, v.decay_scale) != (plain.level, plain.attack, plain.decay_scale) || v.wave != plain.wave, "{w} is not a family word of tonal_voice");
        }
    }

    #[test]
    fn real_voices_are_listed_and_reese_is_only_a_family() {
        let c = CatalogView;
        let v = c.voices();
        for want in ["wobble-lead", "hoover", "detuned-saw", "voice-robot", "pwm-lead"] {
            assert!(v.iter().any(|x| x == want), "{want} in {v:?}");
        }
        assert!(!v.iter().any(|x| x == "reese"));
        assert!(c.families().iter().any(|x| x == "reese"));
        assert!(c.kits().iter().any(|k| k == "drums-808"));
        assert!(c.palettes().iter().any(|p| p == "studio"));
    }

    /// Every verb of every noun of the control language is reachable from a program: `do <noun>
    /// <verb> <its required params>` checks clean against the registry, and the call it makes is the
    /// one the verb names. (The values are made from the param's own type, so this is the reach of
    /// the grammar and the validation, not what each verb does.)
    #[test]
    fn every_registry_verb_is_reachable_through_do() {
        use djlive::compile::{check_with, Effect};
        let be = BeatscriptEnv::new(None);
        let env = be.env();
        let mut tried = 0;
        for n in control_registry::nouns().iter().filter(|n| !n.local) {
            let spec = RegistryView.noun(n.name).unwrap();
            let verbs: Vec<Option<&str>> = if n.actions.is_empty() { vec![None] } else { n.actions.iter().map(|a| Some(a.name)).collect() };
            for v in verbs {
                let mut line = format!("do {}", n.name);
                if let Some(v) = v {
                    line.push_str(&format!(" {v}"));
                }
                if let Some(a) = v.and_then(|v| spec.action(v)) {
                    for p in a.params.iter().filter(|p| p.required) {
                        let full = spec.param(&p.name).unwrap_or(p);
                        let value = if let Some(o) = full.options.first() {
                            o.clone()
                        } else if full.types.iter().any(|t| t == "string") {
                            "\"x\"".to_string()
                        } else if full.types.iter().any(|t| t == "integer") {
                            "1".to_string()
                        } else if full.types.iter().any(|t| t == "number") {
                            "0.5".to_string()
                        } else if full.types.iter().any(|t| t == "boolean") {
                            "on".to_string()
                        } else if full.types.iter().any(|t| t == "array") {
                            "[1]".to_string()
                        } else {
                            "1".to_string()
                        };
                        line.push_str(&format!(" {}={value}", p.name));
                    }
                }
                line.push('\n');
                let c = check_with(&line, &env);
                // (the `btw` verbs that act on the running program are refused in a program, R-11: they are the one exception)
                if n.name == "btw" && matches!(v, Some("eval" | "stop" | "load" | "open" | "replay" | "perform")) {
                    assert!(!c.ok() && c.diagnostics.iter().any(|d| d.message.contains("cannot be called from a program")), "{line}");
                    tried += 1;
                    continue;
                }
                assert!(c.ok(), "{line}{}", djlive::diag::render_all(&c.diagnostics, &line));
                let comp = c.compiled.unwrap();
                let Effect::Calls(cs) = &comp.controls[0].effect else { panic!("{line}") };
                assert_eq!(cs[0].noun, n.name, "{line}");
                if let Some(v) = v {
                    assert_eq!(cs[0].params.get("action").and_then(JsonValue::as_str), Some(v), "{line}");
                }
                tried += 1;
            }
        }
        assert!(tried > 250, "only {tried} verbs tried");
    }

    #[test]
    fn a_do_line_names_what_is_wrong_in_the_registrys_words() {
        use djlive::compile::check_with;
        let be = BeatscriptEnv::new(None);
        let env = be.env();
        let e = |src: &str| {
            let c = check_with(src, &env);
            assert!(!c.ok(), "{src}");
            c.diagnostics.iter().find(|d| d.is_error()).map(|d| (d.position(src).0, d.position(src).1, d.message.clone(), d.help.clone())).unwrap()
        };
        assert_eq!(e("do vocal creat lyrics=\"x\"\n").3.as_deref(), Some("did you mean `create`?"));
        assert!(e("do loopsets load_pair group=\"two\"\n").2.contains("whole number") || e("do loopsets load_pair group=\"two\"\n").2.contains("`group`"));
        assert_eq!(e("do mixer decks=A\n").2, "`mixer` has no param `decks`");
        assert!(e("do btw check text=\"deck A: kick x\" nme=\"x\"\n").2.contains("no param `nme`"));
        // a verb that acts on the running program is refused, at the verb, with the way out (R-11)
        let refused = e("do btw eval text=\"deck A: kick x\"\n");
        assert!(refused.2.contains("`do btw eval` cannot be called from a program"), "{refused:?}");
        assert_eq!((refused.0, refused.1), (1, 8), "at the verb");
        assert!(e("at bar 4: do btw stop\n").2.contains("cannot be called from a program"));
        assert!(e("do livecode load id=\"x\"\n").2.contains("cannot be called from a program"));
    }

    /// The example song checks clean against the real registry and instruments, has its six sections, and
    /// its CID does not move with a re-save (docs/BEATSCRIPT.md).
    #[test]
    fn the_v2_example_checks_against_the_real_registry_and_voices() {
        let src = include_str!("../../../docs/beatscript/hand-dubstep-140-v2.beatscript");
        let be = BeatscriptEnv::new(None);
        let c = djlive::compile::check_with(src, &be.env());
        assert!(c.ok(), "{}", djlive::diag::render_all(&c.diagnostics, src));
        assert!(c.diagnostics.is_empty(), "no warnings: every name is a real voice: {:?}", c.diagnostics);
        let comp = c.compiled.unwrap();
        assert_eq!(comp.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["intro", "build", "drop", "breakdown", "drop2", "outro"]);
        assert_eq!(comp.song_bars(), Some(64));
        let again = djlive::compile::check_with(&comp.text, &be.env()).compiled.unwrap();
        assert_eq!(again.cid, comp.cid);
        // the riser ends on the phrase line the drop starts on: build is bars 8..16, its ramp is 8 bars
        let riser = comp.controls.iter().find(|c| c.key.contains("filter hp")).unwrap();
        assert_eq!(riser.timing, djlive::compile::Timing::At(8));
        let drop = comp.segments.iter().find(|s| s.name == "drop").unwrap();
        assert_eq!(drop.start, 16);
        // the roll leads in four bars ahead of the drop
        let roll = comp.controls.iter().find(|c| c.key.contains("loop_roll")).unwrap();
        assert_eq!(roll.timing, djlive::compile::Timing::At(12));
    }

    #[test]
    fn the_trance_example_checks_against_the_real_registry_and_voices() {
        let src = include_str!("../../../examples/trance-138.beatscript");
        let be = BeatscriptEnv::new(None);
        let c = djlive::compile::check_with(src, &be.env());
        assert!(c.ok(), "{}", djlive::diag::render_all(&c.diagnostics, src));
        let comp = c.compiled.unwrap();
        assert_eq!(comp.song_bars(), Some(64));
        assert_eq!(comp.end, Some(64));
    }

    /// The duck and the mix are made of verbs the real registry has (`trick on kind=pump source=A amount div`, `mixer`): the
    /// example checks clean against it, voices and all, and every call it makes is one `describe` knows.
    #[test]
    fn the_mix_moves_check_against_the_real_registry() {
        use djlive::compile::{check_with, Effect};
        let be = BeatscriptEnv::new(None);
        let env = be.env();
        let src = include_str!("../../../examples/beatscript/mix.beatscript");
        let c = check_with(src, &env);
        assert!(c.ok(), "{}", djlive::diag::render_all(&c.diagnostics, src));
        assert!(c.diagnostics.is_empty(), "{:?}", c.diagnostics);
        let comp = c.compiled.unwrap();
        assert_eq!(comp.mix_report().len(), 5);
        // the pump and the mixer's params are the registry's own
        let trick = control_registry::noun("trick").unwrap();
        let on = comp.controls.iter().find_map(|c| if let Effect::Duck(d) = &c.effect { Some(d.on_call(124.0)) } else { None }).unwrap();
        let schema = trick.input_schema();
        let props = schema.get("properties").and_then(JsonValue::as_object).unwrap();
        for (k, _) in on.params.as_object().unwrap().iter() {
            assert!(props.contains_key(k), "`trick` has no `{k}`");
        }
        assert!(trick.actions.iter().any(|a| a.name == "on") && trick.actions.iter().any(|a| a.name == "off"));
        let mixer = control_registry::noun("mixer").unwrap().input_schema();
        let props = mixer.get("properties").and_then(JsonValue::as_object).unwrap();
        for k in ["deck", "volume", "low", "mid", "high", "master"] {
            assert!(props.contains_key(k), "`mixer` has no `{k}`");
        }
        // the pump kind and the deck words are among the allowed ones
        let kinds = props_enum(&schema, "kind");
        assert!(kinds.iter().any(|k| k == "pump"), "{kinds:?}");
        // and a deck that is not one is the registry's complaint too
        let e = check_with("deck A: kick x\nduck C under A.kick 6db\n", &env);
        assert!(!e.ok() && e.diagnostics[0].message.contains("`A` or `B`"), "{:?}", e.diagnostics);
    }

    fn props_enum(schema: &JsonValue, name: &str) -> Vec<String> {
        schema.get("properties").and_then(|p| p.get(name)).and_then(|p| p.get("enum")).and_then(JsonValue::as_array).map(|a| a.iter().filter_map(JsonValue::as_str).map(str::to_string).collect()).unwrap_or_default()
    }

    // ---- sing (docs/beatscript/ext-sing.md)

    /// The calls of a program's controls, as (noun, params).
    fn controls_calls(c: &djlive::compile::Compiled) -> Vec<Vec<(String, JsonValue)>> {
        c.controls
            .iter()
            .map(|k| match &k.effect {
                djlive::compile::Effect::Calls(cs) => cs.iter().map(|c| (c.noun.clone(), c.params.clone())).collect(),
                _ => panic!("a ramp"),
            })
            .collect()
    }

    /// What the vocal noun's handler does with a `create` call's `notes` (djapp `control/vocal.rs`, `places_param`), so a test can
    /// make the line the voice would make.
    fn places(create: &JsonValue) -> Vec<djengine::vocal::fit::Place> {
        create
            .get("notes")
            .and_then(JsonValue::as_array)
            .expect("notes")
            .iter()
            .map(|e| djengine::vocal::fit::Place {
                start: e.get("start").and_then(JsonValue::as_f64).unwrap(),
                dur: e.get("dur").and_then(JsonValue::as_f64).unwrap(),
                midi: e.get("midi").and_then(JsonValue::as_f64).unwrap() as u8,
            })
            .collect()
    }

    /// The line the real vocal noun makes from a `create` call, and what it warned of.
    fn vocal_line(create: &JsonValue) -> (djengine::vocal::VocalLine, Vec<String>) {
        let s = |k: &str| create.get(k).and_then(JsonValue::as_str).map(str::to_string);
        let key = djengine::key::Key::parse(&s("key").unwrap_or_else(|| "A minor".into())).expect("a key the voice reads");
        let mut p = djengine::vocal::Params::new(&s("lyrics").unwrap(), key, create.get("bpm").and_then(JsonValue::as_f64).unwrap_or(120.0), create.get("bars").and_then(JsonValue::as_f64).unwrap() as u32);
        p.preset = s("preset").unwrap_or_else(|| djengine::vocal::DEFAULT_PRESET.to_string());
        p.places = Some(places(create));
        djengine::vocal::VocalLine::create(p).expect("the vocal noun takes the line")
    }

    const SING: &str = "tempo 120\nkey am\ndeck B: melody c4 e4 g4 e4\ndeck B: sing \"lamps in the harbor, glowing slow\" on melody bars=2 voice=robot\n";

    /// The calls a `sing` line makes are calls the registry's own `vocal` noun takes: every verb and every param is one it
    /// lists, with a type it allows, and what `create` requires is there.
    #[test]
    fn a_sing_line_makes_calls_the_registry_takes() {
        let be = BeatscriptEnv::new(None);
        let c = djlive::compile::check_with(SING, &be.env());
        assert!(c.ok(), "{}", djlive::diag::render_all(&c.diagnostics, SING));
        let comp = c.compiled.unwrap();
        let calls = controls_calls(&comp);
        assert_eq!(calls.len(), 1);
        let spec = RegistryView.noun("vocal").unwrap();
        for (noun, params) in &calls[0] {
            assert_eq!(noun, "vocal");
            let verb = params.get("action").and_then(JsonValue::as_str).unwrap();
            let action = spec.action(verb).unwrap_or_else(|| panic!("`vocal {verb}` is not a verb"));
            for r in action.params.iter().filter(|p| p.required) {
                assert!(params.get(&r.name).is_some(), "`vocal {verb}` needs `{}`", r.name);
            }
            for (k, v) in params.as_object().unwrap().iter() {
                if k == "action" {
                    continue;
                }
                let p = spec.param(k).or_else(|| action.params.iter().find(|p| p.name == *k)).unwrap_or_else(|| panic!("`vocal {verb}` has no param `{k}`"));
                let ty = match v {
                    JsonValue::String(_) => "string",
                    JsonValue::Bool(_) => "boolean",
                    JsonValue::Array(_) => "array",
                    JsonValue::Number(n) if n.fract() == 0.0 => "integer",
                    JsonValue::Number(_) => "number",
                    _ => "other",
                };
                let ok = p.types.is_empty() || p.types.iter().any(|t| t == ty || (ty == "integer" && t == "number") || t == "any");
                assert!(ok, "`vocal {verb} {k}` is {ty}, the registry says {:?}", p.types);
            }
        }
        // and a `do` line of the same call passes the same check, with the same params
        let (_, create) = &calls[0][0];
        let do_line = format!("do vocal create lyrics=\"{}\" bars=2 preset=robot key=\"A minor\"\n", create.get("lyrics").and_then(JsonValue::as_str).unwrap());
        assert!(djlive::compile::check_with(&do_line, &be.env()).ok());
    }

    /// The syllables the language counted are the ones the voice sings, so every note has its syllable and none is left over: the
    /// real vocal noun takes the line with nothing to say about notes it could not use.
    #[test]
    fn the_notes_and_the_lyric_come_out_one_for_one_in_the_real_vocal_noun() {
        let be = BeatscriptEnv::new(None);
        let comp = djlive::compile::check_with(SING, &be.env()).compiled.unwrap();
        let (_, create) = controls_calls(&comp).remove(0).remove(0);
        let (line, warnings) = vocal_line(&create);
        // (the words the BeatScript lexicon lacks are reported as sounded out: not a note problem)
        assert!(warnings.iter().all(|w| w.starts_with("not in the lexicon")), "{warnings:?}");
        let want = places(&create);
        assert_eq!(want.len(), 8);
        assert_eq!(line.notes.len(), 8);
        for (n, w) in line.notes.iter().zip(&want) {
            assert_eq!((n.start, n.dur, n.midi), (w.start, w.dur, w.midi));
        }
        // eight syllables on eight notes: lamps in the har-bor glow-ing slow
        let words: Vec<&str> = line.notes.iter().map(|n| n.word.as_str()).collect();
        assert_eq!(words, ["lamps", "in", "the", "harbor", "", "glowing", "", "slow"]);
        // a word the language would have counted short, and the dictionary's count made it right
        let src = "key am\ndeck B: melody c4 e4 g4 e4\ndeck B: sing \"fire on the road\" on melody bars=2 voice=robot\n";
        let comp = djlive::compile::check_with(src, &be.env()).compiled.unwrap();
        let (_, create) = controls_calls(&comp).remove(0).remove(0);
        let (line, warnings) = vocal_line(&create);
        // (the words the BeatScript lexicon lacks are reported as sounded out: not a note problem)
        assert!(warnings.iter().all(|w| w.starts_with("not in the lexicon")), "{warnings:?}");
        assert_eq!(line.notes.len(), 8);
        // fire (two syllables) on notes one and two, on the third, the fourth ...: the lyric wraps from `fire` again on note six
        let words: Vec<&str> = line.notes.iter().map(|n| n.word.as_str()).collect();
        assert_eq!(words, ["fire", "", "on", "the", "road", "fire", "", "on"]);
    }

    /// The neural voice is not installed in this build: a line that names one says so, once, and sings in the formant voice.
    #[test]
    fn the_neural_voice_is_not_here_and_a_line_that_asks_for_it_says_so_and_sings_in_the_formant_voice() {
        let be = BeatscriptEnv::new(None);
        assert!(!be.voices.neural, "there is no neural voice engine in this build (vocals::NEURAL_ENGINE_BUILT)");
        let src = "tempo 120\nkey am\ndeck B: melody c4 e4 g4 e4\ndeck B: sing \"robots on the road\" on melody bars=2 voice=alto\nvocal create \"robots on the road\" key=am bars=2 preset=tenor\n";
        let c = djlive::compile::check_with(src, &be.env());
        assert!(c.ok(), "{}", djlive::diag::render_all(&c.diagnostics, src));
        let said: Vec<&djlive::diag::Diagnostic> = c.diagnostics.iter().filter(|d| d.message == djlive::sing::FALLBACK).collect();
        assert_eq!(said.len(), 2, "{:?}", c.diagnostics);
        assert_eq!(said.iter().map(|d| d.position(src).0).collect::<Vec<_>>(), [4, 5], "the sing line and the sugar line, each at its voice");
        let comp = c.compiled.unwrap();
        let calls = controls_calls(&comp);
        assert!(calls.iter().all(|c| c[0].1.get("preset").and_then(JsonValue::as_str) == Some("robot")), "both play on the formant voice");
        assert_eq!(comp.controls.len(), 2);
        assert!(comp.controls.iter().all(|c| c.notice.as_deref() == Some(djlive::sing::FALLBACK)));
        // the program is what was written: its canonical text keeps the neural voice's name
        assert!(comp.text.contains("voice=alto") && comp.text.contains("preset=tenor"), "{}", comp.text);
        // with the neural voice installed (a test says so) nothing is said and the names go through
        let mut be = BeatscriptEnv::new(None);
        be.voices.neural = true;
        let c = djlive::compile::check_with(src, &be.env());
        let comp = c.compiled.unwrap();
        assert!(comp.warnings.is_empty(), "{:?}", comp.warnings);
        // (the sing line's control comes after the program's other lines: it needs the decks' nets)
        let preset_of = |needle: &str| {
            let i = comp.controls.iter().position(|c| c.key.contains(needle)).unwrap();
            controls_calls(&comp)[i][0].1.get("preset").and_then(JsonValue::as_str).map(str::to_string)
        };
        assert_eq!(preset_of("sing").as_deref(), Some("alto"));
        assert_eq!(preset_of("vocal create").as_deref(), Some("tenor"));
        // and a name that is no voice is an error with the nearest
        let bad = "deck B: melody c4\ndeck B: sing \"la\" on melody voice=sopran\n";
        let c = djlive::compile::check_with(bad, &BeatscriptEnv::new(None).env());
        let d = c.diagnostics.iter().find(|d| d.is_error()).unwrap();
        assert_eq!(d.message, "`sopran` is not a voice");
        assert!(d.help.as_deref().unwrap().contains("`soprano`"), "{:?}", d.help);
    }

    /// The vocal sugar with no voice named is the formant default, silently, as it always was.
    #[test]
    fn the_vocal_sugar_with_no_voice_is_as_it_was() {
        let be = BeatscriptEnv::new(None);
        let src = "vocal create \"robots on the road\" key=am bars=4\nvocal hook lyrics=\"la la\" bars=2\n";
        let c = djlive::compile::check_with(src, &be.env());
        assert!(c.ok() && c.diagnostics.is_empty(), "{:?}", c.diagnostics);
        let comp = c.compiled.unwrap();
        for calls in controls_calls(&comp) {
            assert!(calls[0].1.get("preset").is_none());
        }
    }

    /// The example checks against the real registry and voices: the fallback is said once, for `alto`, and the line it makes is a
    /// line the vocal noun takes whole.
    #[test]
    fn the_sing_example_checks_against_the_real_registry_and_voices() {
        let src = include_str!("../../../examples/beatscript/sing.beatscript");
        let be = BeatscriptEnv::new(None);
        let c = djlive::compile::check_with(src, &be.env());
        assert!(c.ok(), "{}", djlive::diag::render_all(&c.diagnostics, src));
        let comp = c.compiled.unwrap();
        let warnings: Vec<&str> = comp.warnings.iter().map(|d| d.message.as_str()).collect();
        assert_eq!(warnings, [djlive::sing::FALLBACK], "only alto is a voice that is not here");
        assert_eq!(comp.segments.iter().map(|s| (s.name.as_str(), s.bars)).collect::<Vec<_>>(), [("verse", 8), ("answer", 4)]);
        assert_eq!(comp.controls.len(), 2);
        let calls = controls_calls(&comp);
        // verse: eight syllables on the melody's eight notes, over the section's eight bars (32 notes)
        let (verse, w) = vocal_line(&calls[0][0].1);
        assert!(w.iter().all(|w| w.starts_with("not in the lexicon")), "{w:?}");
        assert_eq!(verse.bars, 8);
        assert_eq!(verse.notes.len(), 32);
        assert_eq!(verse.preset, "robot", "alto falls back");
        // answer: a rest and a hyphenated word: "hush _ lan-tern" is four syllables on eight notes, two passes, a rest each
        let (answer, w) = vocal_line(&calls[1][0].1);
        assert!(w.iter().all(|w| w.starts_with("not in the lexicon")), "{w:?}");
        assert_eq!((answer.bars, answer.preset.as_str()), (2, "robot-choir"));
        assert_eq!(answer.notes.len(), 6, "two of the eight are the rests");
        assert_eq!(answer.notes.iter().map(|n| n.word.as_str()).collect::<Vec<_>>(), ["hush", "lan", "tern", "hush", "lan", "tern"]);
    }

    /// The wheel from code: sync to a deck with a ratio, step a rung, follow a deck, and a drop interval in
    /// beats, checked against the registry and written back the same way.
    #[test]
    fn the_wheel_is_driven_from_a_program_with_ratios_and_beats() {
        use djlive::compile::{check_with, Effect};
        let be = BeatscriptEnv::new(None);
        let env = be.env();
        let src = "wheel sync deck=A ratio=1/4\nwheel step deck=B up=off\nwheel follow deck=A\nat bar 4: wheel interval beats=1/2 deck=A\nevery 8 bars: wheel sync deck=A\n";
        let c = check_with(src, &env);
        assert!(c.ok(), "{}", djlive::diag::render_all(&c.diagnostics, src));
        let comp = c.compiled.unwrap();
        // a ratio is written as it is spoken and is the same program whatever its spacing
        assert!(comp.text.contains("wheel sync deck=A ratio=1/4"), "{}", comp.text);
        let again = check_with(&comp.text, &env).compiled.unwrap();
        assert_eq!(again.cid, comp.cid);
        let Effect::Calls(cs) = &comp.controls[0].effect else { panic!() };
        assert_eq!((cs[0].noun.as_str(), cs[0].params.get("action").and_then(JsonValue::as_str), cs[0].params.get("ratio").and_then(JsonValue::as_str)), ("wheel", Some("sync"), Some("1/4")));
        let Effect::Calls(cs) = &comp.controls[3].effect else { panic!() };
        assert_eq!(cs[0].params.get("beats").and_then(JsonValue::as_str), Some("1/2"));
        // a number works as well as a fraction, and the registry still says what is wrong
        assert!(check_with("wheel interval beats=0.25\n", &env).ok());
        assert!(check_with("wheel interval seconds=0.25\n", &env).ok());
        let e = check_with("wheel sync dek=A\n", &env);
        assert!(!e.ok() && e.diagnostics[0].message.contains("no param `dek`"), "{:?}", e.diagnostics);
        let e = check_with("wheel sinc deck=A\n", &env);
        assert_eq!(e.diagnostics[0].help.as_deref(), Some("did you mean `sync`?"));
    }
}
