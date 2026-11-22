//! Mixxx mappings on the host: find the user's copy, match a port to a mapping, attach it
//! (docs/MIXXX-MAPPINGS.md).
//!
//! The mappings are **not ours**: they are GPL files that stay in the user's Mixxx install (or a
//! folder they point at). This module only *reads* them, at runtime, from where they are; it
//! never copies one into the repo, a build or a cache of its own, and downloads nothing.
//!
//! [`Mixxx`] is the state the app keeps: the index of the mappings found, the config's overrides,
//! and which mapping drives which port. `consider` is the hot-plug step (a port nobody has a
//! profile for: is there a mapping by that name?), `load` / `unload` are the verbs, and `filter`
//! rewrites a message the way the mapping's options say before the router reads it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use djengine::control::router::ControllerRouter;
use djengine::control::surface::SurfaceProfile;
use djengine::mixxx::discover::{self, Candidate, Override, Pick};
use djengine::mixxx::{self, MixxxProfile, Report};

/// Where Mixxx keeps its controller mappings, by platform: the user's own folder first (it
/// overrides the install's), then the install's.
pub fn default_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if let Some(h) = &home {
        v.push(h.join("Library/Containers/org.mixxx.mixxx/Data/Library/Application Support/Mixxx/controllers"));
        v.push(h.join("Library/Application Support/Mixxx/controllers"));
        v.push(h.join(".mixxx/controllers"));
        v.push(h.join(".var/app/org.mixxx.Mixxx/data/mixxx/controllers"));
    }
    if let Some(l) = std::env::var_os("LOCALAPPDATA") {
        v.push(PathBuf::from(l).join("Mixxx/controllers"));
    }
    v.push("/Applications/Mixxx.app/Contents/Resources/controllers".into());
    v.push("/usr/share/mixxx/controllers".into());
    v.push("/usr/local/share/mixxx/controllers".into());
    v.push("/var/lib/flatpak/app/org.mixxx.Mixxx/current/active/files/share/mixxx/controllers".into());
    v.push("C:\\Program Files\\Mixxx\\controllers".into());
    v
}

/// The folders to read, in order: `$MIXXX_CONTROLLERS_DIR`, the config's `dir`, then the
/// defaults. Only those that exist.
pub fn search_dirs(config_dir: Option<&str>) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    if let Some(e) = std::env::var_os("MIXXX_CONTROLLERS_DIR") {
        v.push(e.into());
    }
    if let Some(c) = config_dir.filter(|c| !c.trim().is_empty()) {
        v.push(expand_home(c));
    }
    v.extend(default_dirs());
    let mut seen = BTreeSet::new();
    v.retain(|p| p.is_dir() && seen.insert(p.clone()));
    v
}

fn expand_home(p: &str) -> PathBuf {
    match (p.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(h)) => PathBuf::from(h).join(rest),
        _ => PathBuf::from(p),
    }
}

/// The mappings found in the search folders.
#[derive(Clone, Debug, Default)]
pub struct Library {
    pub dirs: Vec<PathBuf>,
    pub cands: Vec<Candidate>,
    /// Files that were there but not readable as a mapping, with why.
    pub skipped: Vec<(String, String)>,
}

impl Library {
    /// Read the index of `dirs` (not recursive: Mixxx's folders are flat). A file name found in an
    /// earlier folder hides the same name in a later one, as Mixxx's own search order does.
    pub fn scan(dirs: Vec<PathBuf>) -> Library {
        let mut lib = Library { dirs: dirs.clone(), ..Library::default() };
        let mut seen = BTreeSet::new();
        for d in &dirs {
            let Ok(rd) = std::fs::read_dir(d) else { continue };
            let mut files: Vec<PathBuf> = rd
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().ends_with(".midi.xml")))
                .collect();
            files.sort();
            for f in files {
                let file = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if !seen.insert(file.clone()) {
                    continue;
                }
                match std::fs::read(&f).map_err(|e| e.to_string()).and_then(|b| mixxx::parse(&b)) {
                    Ok(p) => lib.cands.push(Candidate {
                        id: f.to_string_lossy().into_owned(),
                        file,
                        name: p.name,
                        controller_id: p.controller_id,
                    }),
                    Err(e) => lib.skipped.push((file, e)),
                }
            }
        }
        lib
    }

    pub fn read(&self, c: &Candidate) -> Result<mixxx::Preset, String> {
        let bytes = std::fs::read(Path::new(&c.id)).map_err(|e| format!("{}: {e}", c.file))?;
        mixxx::parse(&bytes).map_err(|e| format!("{}: {e}", c.file))
    }
}

/// How a port came to have its mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    /// The port's name matched the mapping's.
    Name,
    /// A config override said so.
    Override,
    /// A `load`.
    Manual,
}

impl Via {
    pub fn name(self) -> &'static str {
        match self {
            Via::Name => "name",
            Via::Override => "override",
            Via::Manual => "load",
        }
    }
}

/// A mapping driving a port.
#[derive(Clone, Debug)]
pub struct Loaded {
    pub port: String,
    /// The device's name in the router (`Mixxx: …`).
    pub device: String,
    pub file: String,
    pub path: String,
    pub via: Via,
    pub mapped: MixxxProfile,
}

impl Loaded {
    pub fn report(&self) -> &Report {
        &self.mapped.report
    }
}

/// The app's Mixxx state.
#[derive(Debug)]
pub struct Mixxx {
    pub enabled: bool,
    pub config_dir: Option<String>,
    pub overrides: Vec<Override>,
    pub lib: Library,
    loaded: BTreeMap<String, Loaded>,
    /// Ports looked at with nothing found (so a scan a second does not pick again).
    tried: BTreeMap<String, String>,
}

impl Default for Mixxx {
    fn default() -> Mixxx {
        Mixxx { enabled: true, config_dir: None, overrides: Vec::new(), lib: Library::default(), loaded: BTreeMap::new(), tried: BTreeMap::new() }
    }
}

/// `"port text=mapping"` → an override.
pub fn parse_override(s: &str) -> Option<Override> {
    let (p, m) = s.split_once('=')?;
    (!p.trim().is_empty() && !m.trim().is_empty()).then(|| Override { port: p.trim().into(), mapping: m.trim().into() })
}

impl Mixxx {
    /// Read the folders now.
    pub fn rescan(&mut self) {
        self.lib = Library::scan(search_dirs(self.config_dir.as_deref()));
        self.tried.clear();
    }

    pub fn loaded(&self) -> impl Iterator<Item = &Loaded> {
        self.loaded.values()
    }

    pub fn loaded_on(&self, port: &str) -> Option<&Loaded> {
        self.loaded.get(port)
    }

    pub fn loaded_device(&self, device: &str) -> Option<&Loaded> {
        self.loaded.values().find(|l| l.device == device)
    }

    pub fn tried(&self) -> &BTreeMap<String, String> {
        &self.tried
    }

    /// An unplugged port is forgotten, so a replug looks again.
    pub fn forget_port(&mut self, port: &str) {
        self.tried.remove(port);
    }

    /// A message from `port`, as its mapping's options want it (the invert and the encoders).
    /// Untouched for a port with no mapping.
    pub fn filter<'a>(&self, port: &str, bytes: &'a [u8]) -> std::borrow::Cow<'a, [u8]> {
        match self.loaded.get(port) {
            Some(l) if !l.mapped.prefilter.is_empty() && bytes.len() >= 3 => {
                std::borrow::Cow::Owned(l.mapped.prefilter.apply(bytes).to_vec())
            }
            _ => std::borrow::Cow::Borrowed(bytes),
        }
    }

    /// A free device name: `Mixxx: X`, or `Mixxx: X (2)` for a second unit of the same mapping.
    fn unique(&self, router: &ControllerRouter, want: &str) -> String {
        let taken = |n: &str| router.surface_profiles.iter().any(|p| p.name == n);
        if !taken(want) {
            return want.to_string();
        }
        (2..).map(|i| format!("{want} ({i})")).find(|n| !taken(n)).unwrap_or_else(|| want.to_string())
    }

    fn attach(
        &mut self,
        port: &str,
        c: &Candidate,
        via: Via,
        router: &mut ControllerRouter,
    ) -> Result<String, String> {
        let preset = self.lib.read(c)?;
        let mut m = mixxx::build(&preset, port);
        let name = self.unique(router, &m.profile.name);
        if name != m.profile.name {
            let mut p: SurfaceProfile = (*m.profile).clone();
            p.name = name.clone();
            m.profile = Arc::new(p);
        }
        if !router.add_profile(m.profile.clone()) {
            return Err(format!("{name}: the router already has a device of that name"));
        }
        router.attach(port);
        self.tried.remove(port);
        self.loaded.insert(
            port.to_string(),
            Loaded { port: port.into(), device: name.clone(), file: c.file.clone(), path: c.id.clone(), via, mapped: m },
        );
        Ok(name)
    }

    /// The hot-plug step: `port` is an input nobody has looked at. If no profile owns it and a
    /// mapping suits its name, attach that mapping. `Some(Ok(device))` when one was attached,
    /// `Some(Err(..))` for a mapping chosen but unreadable, `None` otherwise (a profile owns the
    /// port, Mixxx is off, nothing matched, or it was looked at already).
    pub fn consider(&mut self, port: &str, router: &mut ControllerRouter) -> Option<Result<String, String>> {
        if !self.enabled || self.loaded.contains_key(port) || self.tried.contains_key(port) {
            return None;
        }
        // hand-made profiles, the generic devices a person added, and the Inpulse's jog: first
        if router.profile_name(port).is_some() {
            return None;
        }
        if self.lib.cands.is_empty() {
            self.tried.insert(port.to_string(), "no Mixxx mappings found".into());
            return None;
        }
        let cands = self.lib.cands.clone();
        match discover::pick(port, &cands, &self.overrides) {
            Pick::Name(c) => Some(self.attach(port, c, Via::Name, router).inspect_err(|e| {
                self.tried.insert(port.to_string(), e.clone());
            })),
            Pick::Override(c) => Some(self.attach(port, c, Via::Override, router).inspect_err(|e| {
                self.tried.insert(port.to_string(), e.clone());
            })),
            Pick::Disabled => {
                self.tried.insert(port.to_string(), "an override gives this port no mapping".into());
                None
            }
            Pick::OverrideMissing(m) => {
                let why = format!("the override names {m:?}, which is not among the mappings found");
                self.tried.insert(port.to_string(), why.clone());
                Some(Err(why))
            }
            Pick::None => {
                self.tried.insert(port.to_string(), "no mapping by this name".into());
                None
            }
        }
    }

    /// Load the mapping `mapping` (a file name or part of a name) onto `port` now, whatever
    /// the name says and whatever was there. A hand-made profile's port is refused.
    pub fn load(&mut self, port: &str, mapping: &str, router: &mut ControllerRouter) -> Result<String, String> {
        if let Some(existing) = router.profile_name(port) {
            if self.loaded.get(port).map(|l| l.device.as_str()) != Some(existing.as_str()) {
                return Err(format!("{port:?} is driven by {existing} already (hand-made profiles keep their ports)"));
            }
        }
        let c = discover::find(&self.lib.cands, mapping)
            .cloned()
            .ok_or_else(|| format!("no mapping called {mapping:?} (controllers list)"))?;
        if self.loaded.contains_key(port) {
            self.unload(port, router)?;
        }
        self.attach(port, &c, Via::Manual, router)
    }

    /// Take the mapping off a port (or device): the router forgets the device and its controller.
    pub fn unload(&mut self, port_or_device: &str, router: &mut ControllerRouter) -> Result<Loaded, String> {
        let port = self
            .loaded
            .iter()
            .find(|(p, l)| p.as_str() == port_or_device || l.device == port_or_device)
            .map(|(p, _)| p.clone())
            .ok_or_else(|| format!("{port_or_device:?} has no Mixxx mapping loaded"))?;
        let l = self.loaded.remove(&port).ok_or_else(|| "gone".to_string())?;
        router.detach(&port);
        router.surface_profiles.retain(|p| p.name != l.device);
        // looked at again at the next scan (an override or the name may take it back)
        self.tried.insert(port, "unloaded by hand".into());
        Ok(l)
    }
}
