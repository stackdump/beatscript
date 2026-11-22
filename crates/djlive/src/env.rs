//! What the language is checked against, handed in by the host so this crate stays pure: the
//! control-language registry (every noun, verb and param: the same table `describe`, the MCP tools
//! and docs/CONTROL.md come from), the engine's instrument names, and the saved snippets an
//! `import` can name. Each is plain data or a small trait; with none given a program is checked as
//! far as it can be (a `do` line is read but not validated, an `inst=` is not looked up, an
//! `import` is an error: there is nothing to import from).

use std::collections::BTreeMap;

/// One parameter of a verb, as the registry says it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParamSpec {
    pub name: String,
    /// JSON-schema types: `string`, `integer`, `number`, `boolean`, `array`, `object`, `null`.
    /// Empty: anything.
    pub types: Vec<String>,
    /// The allowed words, when the schema lists them.
    pub options: Vec<String>,
    pub required: bool,
    pub doc: String,
    /// The schema's `minimum` and `maximum`, when it gives them (what an editor's number scrubber clamps to).
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ActionSpec {
    pub name: String,
    /// Read | write | live (the registry's `Effect`).
    pub effect: String,
    /// The params the verb names itself (empty: it takes the noun's).
    pub params: Vec<ParamSpec>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NounSpec {
    /// The canonical name (an alias resolves to it).
    pub name: String,
    pub default_action: String,
    pub actions: Vec<ActionSpec>,
    /// Every param the noun's schema has (the union over its verbs, or its own for a noun with no
    /// verbs), with types and options.
    pub params: Vec<ParamSpec>,
}

impl NounSpec {
    pub fn action(&self, name: &str) -> Option<&ActionSpec> {
        self.actions.iter().find(|a| a.name == name)
    }

    pub fn param(&self, name: &str) -> Option<&ParamSpec> {
        self.params.iter().find(|p| p.name == name)
    }
}

/// The control-language registry.
pub trait Registry {
    /// The noun a name or alias means.
    fn noun(&self, name: &str) -> Option<NounSpec>;
    /// Every noun's name, for "did you mean".
    fn noun_names(&self) -> Vec<String>;
    /// Whether this host's engine plays a call: Err says what it does not do (`check` warns at the line, so a program
    /// is not silently different where it plays: the browser's engine has a smaller registry than the app's). The
    /// default plays everything.
    fn plays(&self, _noun: &str, _params: &djengine::json::JsonValue) -> Result<(), String> {
        Ok(())
    }
    /// The name of the engine `plays` speaks for (`the browser`).
    fn engine_name(&self) -> &str {
        "this engine"
    }
}

/// The engine's sound names.
pub trait Catalog {
    /// Instruments a part's `inst=` may name (the voice table, the formant voices, the categories'
    /// instruments, the stingers).
    fn voices(&self) -> Vec<String>;
    /// Words that give a name a generic voice of that family (`reese` contains `reese`: it plays
    /// the saw bass voice, with no sound of its own). Names with none of these play the default.
    fn families(&self) -> Vec<String> {
        Vec::new()
    }
    /// Drum kits a drum part's `kit=` may name.
    fn kits(&self) -> Vec<String>;
    /// Sound palettes a deck can be set to.
    fn palettes(&self) -> Vec<String>;
}

/// The singing voices (`sing "..." on melody voice=NAME`, `vocal create preset=NAME`): the formant presets
/// and, when the host has one, the neural voice (docs/VOICE.md). A host with no neural engine says so
/// (`neural_installed` false) and a line that asks for a neural voice is checked, and plays, on the formant
/// voice, with a warning (`sing::FALLBACK`).
pub trait Voices {
    /// Every voice name: the formant presets, then the neural voices.
    fn names(&self) -> Vec<String>;
    /// Whether `name` is a neural voice (one that needs the neural models).
    fn is_neural(&self, name: &str) -> bool;
    /// The neural voice can sing here: its engine is built and its models are on this machine.
    fn neural_installed(&self) -> bool;
    /// The voice a line sings in when it names none (the neural one when it is installed).
    fn default_voice(&self) -> String;
    /// The formant voice a neural voice falls back to when the neural voice is not installed.
    fn fallback_voice(&self) -> String {
        "robot".to_string()
    }
    /// How many syllables the voice sings for a word (its dictionary's count: `fire` is two). None: not
    /// known here, and the language counts vowel groups instead.
    fn sung_syllables(&self, _word: &str) -> Option<usize> {
        None
    }
    /// The host sings a `sing` line as a part of its deck's net (a formant-voice part with a syllable on each note),
    /// not through the `vocal` noun: a host with no vocal noun, the browser (sing.rs, "In the nets").
    fn sings_in_nets(&self) -> bool {
        false
    }
}

/// Saved snippets, for `import`.
pub trait Snippets {
    /// A snippet by CID (whole, head or tail) or name: its CID and its canonical text.
    fn resolve(&self, query: &str) -> Result<(String, String), String>;
    /// The name a program is saved under, to show a person which track is which (None: not known here).
    fn label(&self, _cid: &str) -> Option<String> {
        None
    }
}

/// What a host says when a program draws on a program by CID (an `import`, a borrowed part) and has no text for it. The same words
/// on every host, so that the one thing hosts differ in is whether the text is there, never what the program means (R-06).
pub fn not_here(query: &str) -> String {
    format!("`{}` is not here: this program draws on it, and `check` needs its text", djengine::rights::short_cid(query))
}

/// Is `message` what [`not_here`] says?
pub fn is_not_here(message: &str) -> bool {
    message.contains("` is not here: this program draws on it")
}

/// What a host that cannot fetch tells the person to do about [`not_here`].
pub const NOT_HERE_HELP: &str = "check it with the link that carries it, or where that program is saved: the language never fetches";

#[derive(Clone, Copy, Default)]
pub struct Env<'a> {
    pub registry: Option<&'a dyn Registry>,
    pub catalog: Option<&'a dyn Catalog>,
    pub snippets: Option<&'a dyn Snippets>,
    /// Where `sample` and `loop` find their files (None: a sample is not looked up, so it is not an error here).
    pub assets: Option<&'a dyn crate::assets::Assets>,
    /// The caller knows the program is private (a saved object whose visibility is private), as `use restricted` says.
    pub private: bool,
    /// The singing voices, and whether the neural one is installed (None: a `sing` line cannot be checked against them).
    pub voices: Option<&'a dyn Voices>,
    /// The bitwrap ontology the `#! genre:` and `#! tags:` header lines are checked against (None: they are comments,
    /// and nothing says whether a term is real). docs/btw/ext-linked.md.
    pub ontology: Option<&'a dyn crate::linked::Ontology>,
    /// How many knobs and pads the connected controllers have (F13: `knob(N)` and `pad(N)` are checked against it). None: no
    /// host says, and a program may use `InputBank::DEFAULT`.
    pub bank: Option<crate::perform::InputBank>,
}

impl<'a> Env<'a> {
    pub fn none() -> Env<'static> {
        Env { registry: None, catalog: None, snippets: None, assets: None, private: false, voices: None, ontology: None, bank: None }
    }
}

/// A fixed registry, for tests and hosts that build theirs once.
#[derive(Clone, Debug, Default)]
pub struct StaticRegistry {
    pub nouns: BTreeMap<String, NounSpec>,
    pub aliases: BTreeMap<String, String>,
}

impl Registry for StaticRegistry {
    fn noun(&self, name: &str) -> Option<NounSpec> {
        let n = self.aliases.get(name).map_or(name, String::as_str);
        self.nouns.get(n).cloned()
    }

    fn noun_names(&self) -> Vec<String> {
        self.nouns.keys().cloned().chain(self.aliases.keys().cloned()).collect()
    }
}

/// A fixed catalog.
#[derive(Clone, Debug, Default)]
pub struct StaticCatalog {
    pub voices: Vec<String>,
    pub families: Vec<String>,
    pub kits: Vec<String>,
    pub palettes: Vec<String>,
}

impl Catalog for StaticCatalog {
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

/// A fixed set of voices, for tests and hosts that build theirs once.
#[derive(Clone, Debug)]
pub struct StaticVoices {
    pub formant: Vec<String>,
    pub neural: Vec<String>,
    pub installed: bool,
    /// The voice a line with none named sings in (None: the first formant voice, or the first neural one when installed).
    pub default: Option<String>,
    pub fallback: String,
    /// The voice's own syllable counts, by word.
    pub syllables: BTreeMap<String, usize>,
}

impl Default for StaticVoices {
    fn default() -> Self {
        StaticVoices {
            formant: ["robot", "robot-choir", "android", "choir-aah", "choir-ooh", "soprano", "bass-choir", "talkbox"].map(str::to_string).to_vec(),
            neural: ["alto", "tenor", "baritone", "bass"].map(str::to_string).to_vec(),
            installed: false,
            default: None,
            fallback: "robot".to_string(),
            syllables: BTreeMap::new(),
        }
    }
}

impl Voices for StaticVoices {
    fn names(&self) -> Vec<String> {
        self.formant.iter().chain(self.neural.iter()).cloned().collect()
    }
    fn is_neural(&self, name: &str) -> bool {
        self.neural.iter().any(|n| n == name)
    }
    fn neural_installed(&self) -> bool {
        self.installed
    }
    fn default_voice(&self) -> String {
        match (&self.default, self.installed) {
            (Some(d), _) => d.clone(),
            (None, true) if !self.neural.is_empty() => self.neural[0].clone(),
            (None, _) => self.formant.first().cloned().unwrap_or_else(|| self.fallback.clone()),
        }
    }
    fn fallback_voice(&self) -> String {
        self.fallback.clone()
    }
    fn sung_syllables(&self, word: &str) -> Option<usize> {
        self.syllables.get(word).copied()
    }
}

/// Snippets held in memory: name or CID to (CID, canonical text).
#[derive(Clone, Debug, Default)]
pub struct StaticSnippets {
    pub items: Vec<(String, String, String)>,
}

impl Snippets for StaticSnippets {
    fn label(&self, cid: &str) -> Option<String> {
        self.items.iter().find(|(_, c, _)| c == cid).map(|(n, _, _)| n.clone())
    }

    fn resolve(&self, query: &str) -> Result<(String, String), String> {
        let hits: Vec<_> = self.items.iter().filter(|(name, cid, _)| name == query || cid == query || cid.starts_with(query) || cid.ends_with(query)).collect();
        match hits.len() {
            0 => Err(format!("no saved snippet matches {query:?}")),
            1 => Ok((hits[0].1.clone(), hits[0].2.clone())),
            n => Err(format!("{query:?} matches {n} snippets")),
        }
    }
}

/// The closest names to `w` among `names`: edit distance, or one containing the other, best first.
pub fn suggest(w: &str, names: &[String], max: usize) -> Vec<String> {
    let lw = w.to_lowercase();
    let mut scored: Vec<(usize, &String)> = names
        .iter()
        .filter_map(|n| {
            let ln = n.to_lowercase();
            let d = crate::parse::edit_distance(&lw, &ln);
            let sub = ln.contains(&lw) || lw.contains(&ln);
            let limit = (lw.len() / 3).max(2);
            if d <= limit || sub {
                Some((if sub { d.min(1) } else { d }, n))
            } else {
                None
            }
        })
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(b.1)));
    scored.into_iter().take(max).map(|(_, n)| n.clone()).collect()
}
