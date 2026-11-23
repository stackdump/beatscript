//! Lineage: the family tree of a stored program ("GitHub for music"). Every stored program's rights may name the
//! program it was forked from (`meta.forked_from`); read the other way, that is every program's forks. This module
//! keeps that index, walks it from one program (its ancestors, its siblings, its forks), and labels every edge with
//! what changed in music, from beatscript-api's `same_as` diff ("tempo 124 -> 126", "drop2 bass = drop bass transposed +1").
//!
//! - [`Lineage::tree`]: the tree of one program, as the JSON-LD document `/p/<cid>/tree` and the MCP `lineage` tool
//!   both answer ([`Tree::json_ld`]).
//! - [`Lineage::compare`]: any two programs, as `/p/<a>/compare/<b>` answers.
//! - [`Lineage::fork_count`]: how many programs name this one as their parent, for the program's own page.
//! - `page`: the server-rendered HTML; `routes`: the axum routes.
//!
//! **Private programs never appear.** A program is in a tree only when it is an example or a stored file that is
//! public (its meta says so, or says nothing), whose text does not say otherwise (`#! visibility:`), whose text hashes
//! to its CID, and which checks without `use restricted` or samples. A walk stops at a program that is not: its CID is
//! not shown, and nothing beyond it is reached through it.
//!
//! The store is append-only (first write wins), so the index grows by reading only the files it has not seen, at most
//! every [`RESCAN`], and a pair's diff, once computed, is kept.
//!
//! **The rename.** A stored program whose meta says `derivation: "renamed"` and names as `forked_from` its own legacy CID
//! (`djlive::compile::renamed_cid` of its text) is the BeatScript 2 twin of that BeatScript 1 program: the same music under a
//! new CID. That edge is of kind renamed ([`Edge::renamed`]): it is labelled [`RENAMED_LABEL`] (no diff is worked out), it
//! is not a fork (not in [`Lineage::fork_count`], the forks or the siblings), and a node names its twin either way
//! ([`Node::renamed_to`], [`Node::renamed_from`]). An example written in BeatScript 2 is also known by its legacy CID, so a
//! program forked from an example before the rename still has its parent.

pub mod page;
pub mod routes;

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

pub use routes::router;

/// How old the index may be before it reads the store directory again.
pub const RESCAN: Duration = Duration::from_secs(30);
/// How many ancestors a tree lists (a chain longer than this is cut, and `truncated` says so).
pub const MAX_ANCESTORS: usize = 32;
/// How many forks (all generations) a tree lists.
pub const MAX_FORKS: usize = 200;
/// How many generations of forks a tree goes down.
pub const MAX_DEPTH: usize = 12;
/// How many siblings a tree lists.
pub const MAX_SIBLINGS: usize = 50;
/// How many edges of one tree get a diff computed now (the rest say `change: null` until a later look).
pub const MAX_NEW_DIFFS: usize = 64;
const MAX_CACHED: usize = 50_000;

/// The schema the documents are written in (djlive `progld`'s, the `<beatscript-view>` block's), then schema.org.
pub const SCHEMA: &str = "https://beatscript.app/schema";

/// What a renamed edge says: the same program, spelled in BeatScript 2.
pub const RENAMED_LABEL: &str = "renamed: BeatScript 1 -> 2, same music";

/// One example program: always public.
#[derive(Clone, Debug)]
pub struct Example {
    pub cid: String,
    pub name: String,
    pub license: String,
    pub text: String,
}

/// What the index keeps of one store file (cheap to read: no compile).
#[derive(Clone, Debug, Default)]
struct Entry {
    name: String,
    license: String,
    author: String,
    forked_from: String,
    credits: Vec<String>,
    created: String,
    /// Its parent is its own legacy CID and its meta says `derivation: "renamed"`: the edge to the parent is a rename.
    renamed: bool,
    /// What it is based on (djlive::refs::based_on): (the call or reference, the CID it resolves to, `called` / `borrowed` /
    /// `vendored`). Vendored parts come from `meta.based_on` (`["<call> -> <cid>"]`, the `# from:` lines its text had when it
    /// was saved: the canonical text has no comments).
    bases: Vec<(String, Option<String>, &'static str)>,
}

/// How often public programs draw on a CID or a call: the curation signal (roadmap 6670): what is imported, called and vendored
/// is what is good.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Curation {
    /// Public programs whose text calls or borrows it.
    pub referenced_by: usize,
    /// Public programs that vendored it (an eject's `# from:` line).
    pub vendored_from: usize,
}

/// What the tree shows of one program: what its rights say and what its text compiles to.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub cid: String,
    pub name: String,
    pub license: String,
    pub author: String,
    pub credits: Vec<String>,
    pub created: String,
    pub forked_from: String,
    pub example: bool,
    pub tempo: Option<f64>,
    pub key: String,
    /// Direct forks that are public.
    pub fork_count: usize,
    /// The rights name a parent that may not be shown: `forked_from` is then empty, so its CID is not told.
    pub parent_hidden: bool,
    /// How many public programs call, borrow or vendored this one (by its CID).
    pub curation: Curation,
    /// The edge to `forked_from` is a rename (this is the BeatScript 2 twin of its parent).
    pub renamed: bool,
    /// A BeatScript 1 program whose BeatScript 2 twin may be shown: the twin's CID.
    pub renamed_to: String,
    /// A BeatScript 2 program whose legacy (BeatScript 1) CID may be shown: that CID.
    pub renamed_from: String,
}

impl Node {
    /// The name a person reads: its name, else "BeatScript z4EBG9…Wr".
    pub fn label(&self) -> String {
        let n = self.name.trim();
        if !n.is_empty() {
            return n.chars().filter(|c| !c.is_control()).take(80).collect();
        }
        format!("BeatScript {}", short_cid(&self.cid))
    }
}

/// What changed from one program to another: beatscript-api `same_as`'s answer, cut to what an edge shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    /// The same song by their compiled nets and song settings (a refactor, a renamed section).
    pub same: bool,
    /// The text is the same (only possible for a program and itself).
    pub text_same: bool,
    /// The diff's summary lines, in order: what a person reads on the edge.
    pub summary: Vec<String>,
    /// same_as's own one-line `why`.
    pub why: String,
    /// The whole same_as answer (the compare view shows its parts).
    pub report: Value,
}

/// One edge of a tree: `to` was forked from `from` (or, [`Edge::renamed`], is `from` renamed).
#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    pub from: String,
    pub to: String,
    /// `to` is the BeatScript 2 twin of `from`: the change is [`RENAMED_LABEL`], not a diff.
    pub renamed: bool,
    /// None: not computed yet (more than [`MAX_NEW_DIFFS`] new edges in one look), or the pair did not compile.
    pub change: Option<Change>,
}

/// The family tree of one program.
#[derive(Clone, Debug)]
pub struct Tree {
    pub root: Node,
    /// Nearest first: the parent, the grandparent, ...
    pub ancestors: Vec<Node>,
    /// The parent's other forks.
    pub siblings: Vec<Node>,
    /// Every fork below the root, breadth first, each with its parent.
    pub forks: Vec<Node>,
    pub edges: Vec<Edge>,
    /// The parent the rights name is not shown (not public, not stored here, or the chain is longer than the cap).
    pub ancestors_cut: bool,
    /// More forks or siblings than the caps.
    pub truncated: bool,
}

#[derive(Default)]
struct Index {
    at: Option<Instant>,
    /// Every store file seen: Some for one that may be shown (cheap checks), None for one that may not.
    files: HashMap<String, Option<Entry>>,
    /// parent CID -> the public files that name it (sorted, so trees are stable). Renamed twins are not here.
    children: HashMap<String, BTreeSet<String>>,
    /// A CID or a call (`gen@2 techno seed=42 section=drop part=bass`) -> the public files that reference it, and that vendored it.
    referenced: HashMap<String, (BTreeSet<String>, BTreeSet<String>)>,
}

pub struct Lineage {
    dir: PathBuf,
    examples: HashMap<String, Example>,
    index: Mutex<Index>,
    /// Compiled facts of a program, None when it may not be shown (does not hash, restricted, samples, errors).
    facts: Mutex<HashMap<String, Option<Facts>>>,
    diffs: Mutex<HashMap<(String, String), Option<Change>>>,
}

#[derive(Clone, Debug)]
struct Facts {
    tempo: Option<f64>,
    key: String,
    text: String,
}

static SHARED: OnceLock<Arc<Lineage>> = OnceLock::new();

/// The process's one index (the routes and the MCP tool share it): made on the first call, the later calls' arguments
/// are ignored.
pub fn shared(dir: &Path, examples: impl FnOnce() -> Vec<Example>) -> Arc<Lineage> {
    SHARED.get_or_init(|| Arc::new(Lineage::new(dir, examples()))).clone()
}

/// The shared index, when [`shared`] has made it.
pub fn installed() -> Option<Arc<Lineage>> {
    SHARED.get().cloned()
}

/// A CID as the store names files: `^z[1-9A-HJ-NP-Za-km-z]{20,80}$`.
pub fn is_cid(s: &str) -> bool {
    let b = s.as_bytes();
    b.first() == Some(&b'z') && (21..=81).contains(&b.len()) && b[1..].iter().all(|c| matches!(c, b'1'..=b'9' | b'A'..=b'H' | b'J'..=b'N' | b'P'..=b'Z' | b'a'..=b'k' | b'm'..=b'z'))
}

/// `z4EBG9…Wr`.
pub fn short_cid(cid: &str) -> String {
    let r: Vec<char> = cid.chars().collect();
    if r.len() <= 9 {
        return cid.to_string();
    }
    format!("{}…{}", r[..6].iter().collect::<String>(), r[r.len() - 2..].iter().collect::<String>())
}

/// A `#! visibility:` line that says anything but `public` (the store's `DeclaresNotPublic`).
fn text_not_public(text: &str) -> bool {
    text.split('\n').any(|l| {
        let Some(rest) = l.trim().strip_prefix("#!") else { return false };
        let squeezed: String = rest.split_whitespace().collect::<String>().to_lowercase();
        matches!(squeezed.strip_prefix("visibility:"), Some(v) if v != "public")
    })
}

/// The entry of one store file, None when it may not be shown. No compile here: the CID is checked when it is shown.
fn read_file(path: &Path, cid: &str) -> Option<Entry> {
    let b = std::fs::read(path).ok()?;
    let v: Value = serde_json::from_slice(&b).ok()?;
    let text = v["text"].as_str()?;
    if v["cid"].as_str() != Some(cid) || text.is_empty() || text_not_public(text) {
        return None;
    }
    let m = &v["meta"];
    let s = |k: &str| m.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
    let vis = s("visibility");
    if !(vis.is_empty() || vis.eq_ignore_ascii_case("public")) {
        return None;
    }
    let credits = m.get("credits").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    let forked = s("forked_from");
    // a twin: the meta says so, and the parent it names is its own legacy CID
    let renamed = s("derivation") == crate::gate::RENAMED && is_cid(&forked) && djlive::compile::renamed_cid(text) == forked;
    // what it is based on: the calls and borrows of its text, and what its saved `# from:` lines say it vendored
    let notes: Vec<String> = m.get("based_on").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(|l| format!("from: {l}")).collect()).unwrap_or_default();
    let bases = djlive::refs::based_on(text, &notes).into_iter().map(|b| (b.call, b.cid, b.how)).collect();
    Some(Entry { name: s("name"), license: s("license"), author: s("author"), forked_from: if is_cid(&forked) { forked } else { String::new() }, credits, created: s("created"), renamed, bases })
}

impl Lineage {
    pub fn new(dir: &Path, examples: Vec<Example>) -> Lineage {
        // a call's CID is the program the generator prints for it
        beatscript_api::generate::install();
        let mut examples: HashMap<String, Example> = examples.into_iter().filter(|e| is_cid(&e.cid)).map(|e| (e.cid.clone(), e)).collect();
        // an example in BeatScript 2 is also its legacy CID (what forks made before the rename name), with the legacy text
        let legacy: Vec<Example> = examples
            .values()
            .filter(|e| djlive::version::of_source(&e.text) == djlive::version::Version::V2)
            .filter_map(|e| {
                let rep: Value = serde_json::from_str(&beatscript_api::check_json(&e.text)).ok()?;
                let canon = rep["canonical"].as_str().filter(|_| rep["cid"].as_str() == Some(e.cid.as_str()))?;
                let text = djlive::version::legacy_canonical(canon).to_string();
                Some(Example { cid: djlive::compile::program_cid(&text), name: e.name.clone(), license: e.license.clone(), text })
            })
            .collect();
        for e in legacy {
            examples.entry(e.cid.clone()).or_insert(e);
        }
        // the examples draw on calls and programs too: they count like any public program
        let mut ix = Index::default();
        for (cid, e) in &examples {
            for b in djlive::refs::based_on(&e.text, &[]) {
                for k in std::iter::once(b.call).chain(b.cid) {
                    ix.referenced.entry(k).or_default().0.insert(cid.clone());
                }
            }
        }
        Lineage {
            dir: dir.to_path_buf(),
            examples,
            index: Mutex::new(ix),
            facts: Mutex::new(HashMap::new()),
            diffs: Mutex::new(HashMap::new()),
        }
    }

    /// Read the store files the index has not seen, when it is older than [`RESCAN`] (or `force`).
    pub fn refresh(&self, force: bool) {
        let mut ix = self.index.lock().unwrap();
        if !force && ix.at.is_some_and(|t| t.elapsed() < RESCAN) {
            return;
        }
        ix.at = Some(Instant::now());
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return };
        for e in rd.flatten() {
            let name = e.file_name();
            let Some(cid) = name.to_str().and_then(|n| n.strip_suffix(".json")) else { continue };
            if !is_cid(cid) || ix.files.contains_key(cid) {
                continue;
            }
            let ent = read_file(&e.path(), cid);
            if let Some(en) = &ent {
                // a twin is not a fork of the program it renames
                if !en.forked_from.is_empty() && en.forked_from != cid && !en.renamed {
                    ix.children.entry(en.forked_from.clone()).or_default().insert(cid.to_string());
                }
                for (call, resolves, how) in &en.bases {
                    for k in std::iter::once(call.clone()).chain(resolves.clone()) {
                        let e = ix.referenced.entry(k).or_default();
                        if *how == "vendored" { e.1.insert(cid.to_string()) } else { e.0.insert(cid.to_string()) };
                    }
                }
            }
            ix.files.insert(cid.to_string(), ent);
        }
    }

    /// What the text of a program compiles to, None when it may not be shown. Kept: the text of a CID never changes.
    fn facts(&self, cid: &str, text: impl FnOnce() -> Option<String>) -> Option<Facts> {
        if let Some(f) = self.facts.lock().unwrap().get(cid) {
            return f.clone();
        }
        let f = text().and_then(|t| {
            let rep: Value = serde_json::from_str(&beatscript_api::check_json(&t)).ok()?;
            if rep["ok"] != true || rep["cid"].as_str() != Some(cid) || rep["restricted"] == true || rep["uses_assets"] == true {
                return None;
            }
            Some(Facts { tempo: rep["tempo"].as_f64(), key: rep["key"].as_str().unwrap_or("").to_string(), text: rep["canonical"].as_str().map(str::to_string).unwrap_or(t) })
        });
        let mut m = self.facts.lock().unwrap();
        if m.len() >= MAX_CACHED {
            m.clear();
        }
        m.insert(cid.to_string(), f.clone());
        f
    }

    fn stored_text(&self, cid: &str) -> Option<String> {
        let b = std::fs::read(self.dir.join(format!("{cid}.json"))).ok()?;
        let v: Value = serde_json::from_slice(&b).ok()?;
        v["text"].as_str().map(str::to_string)
    }

    /// The node of `cid`, None when it may not be shown (private, unknown, not its CID's text, restricted).
    pub fn node(&self, cid: &str) -> Option<Node> {
        if !is_cid(cid) {
            return None;
        }
        let (ent, example) = if let Some(ex) = self.examples.get(cid) {
            (Entry { name: ex.name.clone(), license: ex.license.clone(), ..Entry::default() }, true)
        } else {
            let known = self.index.lock().unwrap().files.get(cid).cloned();
            let ent = match known {
                Some(e) => e,
                // stored since the last scan: read it now (the next scan finds it too)
                None => {
                    let e = read_file(&self.dir.join(format!("{cid}.json")), cid);
                    if e.is_some() {
                        self.refresh(true);
                    }
                    e
                }
            };
            (ent?, false)
        };
        let f = if example { self.facts(cid, || Some(self.examples[cid].text.clone())) } else { self.facts(cid, || self.stored_text(cid)) }?;
        let fork_count = self.forks_of(cid).len();
        let curation = self.curation(cid);
        let parent_hidden = !ent.forked_from.is_empty() && !self.shown(&ent.forked_from);
        // its twin, either way, when that may be shown
        let twin = djlive::compile::renamed_cid(&f.text);
        let twin = if self.visible(&twin) { twin } else { String::new() };
        let (renamed_to, renamed_from) = match djlive::version::of_canonical(&f.text) {
            djlive::version::Version::V1 => (twin, String::new()),
            djlive::version::Version::V2 => (String::new(), twin),
        };
        Some(Node {
            cid: cid.to_string(),
            name: ent.name,
            license: if ent.license.is_empty() { "CC0-1.0".into() } else { ent.license },
            author: ent.author,
            credits: ent.credits,
            created: ent.created,
            forked_from: if parent_hidden { String::new() } else { ent.forked_from },
            parent_hidden,
            example,
            tempo: f.tempo,
            key: f.key,
            fork_count,
            curation,
            renamed: ent.renamed && !parent_hidden,
            renamed_to,
            renamed_from,
        })
    }

    /// The canonical text of a program that may be shown.
    pub fn text(&self, cid: &str) -> Option<String> {
        self.node(cid)?;
        self.facts.lock().unwrap().get(cid).cloned().flatten().map(|f| f.text)
    }

    /// The direct forks of `cid` that may be shown, by CID.
    fn forks_of(&self, cid: &str) -> Vec<String> {
        let kids: Vec<String> = self.index.lock().unwrap().children.get(cid).map(|s| s.iter().cloned().collect()).unwrap_or_default();
        kids.into_iter().filter(|k| self.shown(k)).collect()
    }

    /// Whether a CID may be shown, without building its whole node (the facts are kept).
    fn shown(&self, cid: &str) -> bool {
        if self.examples.contains_key(cid) {
            return self.facts(cid, || Some(self.examples[cid].text.clone())).is_some();
        }
        let ok = matches!(self.index.lock().unwrap().files.get(cid), Some(Some(_)));
        ok && self.facts(cid, || self.stored_text(cid)).is_some()
    }

    /// [`Lineage::shown`], also for a file stored since the last scan (read now, as [`Lineage::node`] does).
    fn visible(&self, cid: &str) -> bool {
        if !is_cid(cid) {
            return false;
        }
        if self.examples.contains_key(cid) {
            return self.shown(cid);
        }
        let known = self.index.lock().unwrap().files.get(cid).map(Option::is_some);
        match known {
            Some(true) => self.shown(cid),
            Some(false) => false,
            None => {
                if read_file(&self.dir.join(format!("{cid}.json")), cid).is_none() {
                    return false;
                }
                self.refresh(true);
                self.shown(cid)
            }
        }
    }

    /// For a BeatScript 1 program that may be shown, the CID of its BeatScript 2 twin when that may be shown too: where its
    /// tree redirects.
    pub fn renamed_to(&self, cid: &str) -> Option<String> {
        self.refresh(false);
        let n = self.node(cid)?;
        (!n.renamed_to.is_empty()).then_some(n.renamed_to)
    }

    /// How often public programs draw on `key`, a CID or a call (`gen@2 techno seed=42 section=drop part=bass`, as its canonical
    /// text spells it): the programs that may be shown, counted once each.
    pub fn curation(&self, key: &str) -> Curation {
        self.refresh(false);
        let (r, v) = self.index.lock().unwrap().referenced.get(key).cloned().unwrap_or_default();
        Curation { referenced_by: r.iter().filter(|c| self.shown(c)).count(), vendored_from: v.iter().filter(|c| self.shown(c)).count() }
    }

    /// How many public programs were forked from `cid` (0 for one that may not be shown).
    pub fn fork_count(&self, cid: &str) -> usize {
        self.refresh(false);
        if !is_cid(cid) {
            return 0;
        }
        self.forks_of(cid).len()
    }

    /// The change from `a` to `b` (a's text and b's text): kept per pair. `compute: false` answers only from what is
    /// kept (None when it is not).
    fn change(&self, a: &str, b: &str, compute: bool) -> Option<Option<Change>> {
        let k = (a.to_string(), b.to_string());
        if let Some(c) = self.diffs.lock().unwrap().get(&k) {
            return Some(c.clone());
        }
        if !compute {
            return None;
        }
        let c = (|| {
            let (ta, tb) = (self.text(a)?, self.text(b)?);
            let rep: Value = serde_json::from_str(&beatscript_api::same_as_json(&ta, &tb)).ok()?;
            if rep["ok"] != true {
                return None;
            }
            let summary = rep["diff"]["summary"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
            Some(Change { same: rep["same"] == true, text_same: rep["text_same"] == true, summary, why: rep["why"].as_str().unwrap_or("").to_string(), report: rep })
        })();
        let mut m = self.diffs.lock().unwrap();
        if m.len() >= MAX_CACHED {
            m.clear();
        }
        m.insert(k, c.clone());
        Some(c)
    }

    /// The tree of `cid`, None when it may not be shown. Blocks (it compiles): call it off the async threads.
    pub fn tree(&self, cid: &str) -> Option<Tree> {
        self.refresh(false);
        let root = self.node(cid)?;
        let mut seen: HashSet<String> = HashSet::from([cid.to_string()]);
        // up: the chain of forked_from, while each may be shown
        let mut ancestors = Vec::new();
        let mut ancestors_cut = root.parent_hidden;
        let mut up = root.forked_from.clone();
        while !up.is_empty() {
            if ancestors.len() >= MAX_ANCESTORS || seen.contains(&up) {
                ancestors_cut = true;
                break;
            }
            match self.node(&up) {
                Some(n) => {
                    seen.insert(up.clone());
                    up = n.forked_from.clone();
                    ancestors_cut |= n.parent_hidden;
                    ancestors.push(n);
                }
                None => {
                    ancestors_cut = true;
                    break;
                }
            }
        }
        let mut truncated = false;
        // down: every generation of forks, breadth first
        let mut forks = Vec::new();
        let mut q: VecDeque<(String, usize)> = VecDeque::from([(cid.to_string(), 0)]);
        'down: while let Some((at, depth)) = q.pop_front() {
            if depth >= MAX_DEPTH {
                truncated |= !self.forks_of(&at).is_empty();
                continue;
            }
            for k in self.forks_of(&at) {
                if !seen.insert(k.clone()) {
                    continue;
                }
                if forks.len() >= MAX_FORKS {
                    truncated = true;
                    break 'down;
                }
                if let Some(n) = self.node(&k) {
                    q.push_back((k, depth + 1));
                    forks.push(n);
                }
            }
        }
        // across: the parent's other forks
        let mut siblings = Vec::new();
        if let Some(p) = ancestors.first() {
            for k in self.forks_of(&p.cid) {
                if k == cid || seen.contains(&k) {
                    continue;
                }
                if siblings.len() >= MAX_SIBLINGS {
                    truncated = true;
                    break;
                }
                if let Some(n) = self.node(&k) {
                    seen.insert(k);
                    siblings.push(n);
                }
            }
        }
        // the edges, each with its change: the chain up, the root's own, the siblings', the forks'
        let mut pairs: Vec<(String, String)> = Vec::new();
        let mut child = cid.to_string();
        for a in &ancestors {
            pairs.push((a.cid.clone(), child.clone()));
            child = a.cid.clone();
        }
        pairs.reverse();
        for s in &siblings {
            pairs.push((s.forked_from.clone(), s.cid.clone()));
        }
        for f in &forks {
            pairs.push((f.forked_from.clone(), f.cid.clone()));
        }
        // which edges are renames: the node they lead to says so
        let renamed: HashSet<String> = std::iter::once(&root).chain(&ancestors).chain(&siblings).chain(&forks).filter(|n| n.renamed).map(|n| n.cid.clone()).collect();
        let mut budget = MAX_NEW_DIFFS;
        let edges = pairs
            .into_iter()
            .map(|(from, to)| {
                if renamed.contains(&to) {
                    return Edge { from, to, renamed: true, change: Some(Change::renamed()) };
                }
                let change = match self.change(&from, &to, false) {
                    Some(c) => c,
                    None if budget > 0 => {
                        budget -= 1;
                        self.change(&from, &to, true).flatten()
                    }
                    None => None,
                };
                Edge { from, to, renamed: false, change }
            })
            .collect();
        Some(Tree { root, ancestors, siblings, forks, edges, ancestors_cut, truncated })
    }

    /// `a` against `b`: both nodes and the change from a to b. None when either may not be shown.
    pub fn compare(&self, a: &str, b: &str) -> Option<(Node, Node, Option<Change>)> {
        self.refresh(false);
        let (na, nb) = (self.node(a)?, self.node(b)?);
        let c = self.change(a, b, true).flatten();
        Some((na, nb, c))
    }
}

// ---------- JSON-LD ----------

fn program_url(origin: &str, cid: &str) -> String {
    format!("{origin}/p/{cid}")
}

/// The tree page's URL.
pub fn tree_url(origin: &str, cid: &str) -> String {
    format!("{origin}/p/{cid}/tree")
}

/// The compare page's URL.
pub fn compare_url(origin: &str, a: &str, b: &str) -> String {
    format!("{origin}/p/{a}/compare/{b}")
}

/// A licence id as its IRI (as `progld::license_iri`: the CC deeds, rightsstatements.org's In Copyright).
fn license_iri(id: &str) -> Option<&'static str> {
    match id {
        "" | "CC0-1.0" => Some("https://creativecommons.org/publicdomain/zero/1.0/"),
        "CC-BY-4.0" => Some("https://creativecommons.org/licenses/by/4.0/"),
        "CC-BY-SA-4.0" => Some("https://creativecommons.org/licenses/by-sa/4.0/"),
        "all-rights-reserved" => Some("http://rightsstatements.org/vocab/InC/1.0/"),
        _ => None,
    }
}

impl Node {
    /// The node as a `Program` (the `<beatscript-view>` block's terms, without the text), with its page and tree.
    pub fn json_ld(&self, origin: &str) -> Value {
        let mut m = Map::new();
        m.insert("@id".into(), json!(self.cid));
        m.insert("@type".into(), json!(["Program", "MusicComposition"]));
        m.insert("name".into(), json!(self.label()));
        m.insert("url".into(), json!(program_url(origin, &self.cid)));
        m.insert("lineage".into(), json!(tree_url(origin, &self.cid)));
        m.insert("licenseId".into(), json!(self.license));
        if let Some(l) = license_iri(&self.license) {
            m.insert("license".into(), json!(l));
        }
        if let Some(t) = self.tempo {
            m.insert("tempo".into(), json!(t));
        }
        if !self.key.is_empty() {
            m.insert("key".into(), json!(self.key));
        }
        if !self.author.is_empty() {
            m.insert("author".into(), json!(self.author));
        }
        if !self.credits.is_empty() {
            m.insert("credit".into(), json!(self.credits));
        }
        if !self.created.is_empty() {
            m.insert("dateCreated".into(), json!(self.created));
        }
        if !self.forked_from.is_empty() {
            m.insert("forkedFrom".into(), json!(self.forked_from));
        }
        if self.example {
            m.insert("example".into(), json!(true));
        }
        if self.renamed {
            m.insert("isBasedOn".into(), json!({"@id": self.forked_from, "derivation": crate::gate::RENAMED}));
        }
        if !self.renamed_to.is_empty() {
            m.insert("renamedTo".into(), json!(self.renamed_to));
        }
        if !self.renamed_from.is_empty() {
            m.insert("renamedFrom".into(), json!(self.renamed_from));
        }
        m.insert("forkCount".into(), json!(self.fork_count));
        // the curation signal: how many public programs call or borrow it, and vendored it
        if self.curation.referenced_by > 0 {
            m.insert("referencedBy".into(), json!(self.curation.referenced_by));
        }
        if self.curation.vendored_from > 0 {
            m.insert("vendoredFrom".into(), json!(self.curation.vendored_from));
        }
        Value::Object(m)
    }
}

impl Change {
    /// The change on a renamed edge: the same song, the same statements, a new spelling and CID.
    pub fn renamed() -> Change {
        Change {
            same: true,
            text_same: false,
            summary: vec![RENAMED_LABEL.to_string()],
            why: "the same program spelled in BeatScript 2 (the version line first): a new CID, the same music".into(),
            report: json!({"ok": true, "same": true, "derivation": crate::gate::RENAMED}),
        }
    }

    fn json(&self) -> Value {
        json!({"sameSong": self.same, "change": self.summary, "why": self.why})
    }
}

impl Edge {
    pub fn json_ld(&self, origin: &str) -> Value {
        let mut m = Map::new();
        m.insert("@type".into(), json!("Derivation"));
        m.insert("from".into(), json!(self.from));
        m.insert("to".into(), json!(self.to));
        m.insert("compare".into(), json!(compare_url(origin, &self.from, &self.to)));
        if self.renamed {
            m.insert("derivation".into(), json!(crate::gate::RENAMED));
            m.insert("isBasedOn".into(), json!({"@id": self.from, "derivation": crate::gate::RENAMED}));
        }
        match &self.change {
            Some(c) => {
                m.insert("sameSong".into(), json!(c.same));
                m.insert("change".into(), json!(c.summary));
            }
            None => {
                m.insert("sameSong".into(), Value::Null);
                m.insert("change".into(), Value::Null);
            }
        }
        Value::Object(m)
    }
}

impl Tree {
    /// The tree as one JSON-LD document: what `/p/<cid>/tree` answers a processor and the MCP `lineage` tool returns.
    /// `ancestors` (nearest first), `siblings` and `forks` are CIDs; `programs` describes each one, `derivations` the
    /// edges, each with the musical change from parent to fork.
    pub fn json_ld(&self, origin: &str) -> Value {
        let ids = |v: &[Node]| v.iter().map(|n| json!(n.cid)).collect::<Vec<_>>();
        let direct: Vec<Value> = self.forks.iter().filter(|n| n.forked_from == self.root.cid).map(|n| json!(n.cid)).collect();
        let mut programs = vec![self.root.json_ld(origin)];
        programs.extend(self.ancestors.iter().rev().map(|n| n.json_ld(origin)));
        programs.extend(self.siblings.iter().map(|n| n.json_ld(origin)));
        programs.extend(self.forks.iter().map(|n| n.json_ld(origin)));
        let mut m = Map::new();
        m.insert("@context".into(), json!([SCHEMA, "https://schema.org"]));
        m.insert("@type".into(), json!("Lineage"));
        m.insert("@id".into(), json!(tree_url(origin, &self.root.cid)));
        m.insert("program".into(), json!(self.root.cid));
        m.insert("name".into(), json!(format!("{} — family tree", self.root.label())));
        m.insert("forkCount".into(), json!(self.root.fork_count));
        m.insert("descendantCount".into(), json!(self.forks.len()));
        m.insert("ancestors".into(), Value::Array(ids(&self.ancestors)));
        m.insert("siblings".into(), Value::Array(ids(&self.siblings)));
        m.insert("forks".into(), Value::Array(direct));
        m.insert("programs".into(), Value::Array(programs));
        m.insert("derivations".into(), Value::Array(self.edges.iter().map(|e| e.json_ld(origin)).collect()));
        if self.ancestors_cut {
            m.insert("ancestorsCut".into(), json!(true));
        }
        if self.truncated {
            m.insert("truncated".into(), json!(true));
        }
        Value::Object(m)
    }

    /// The edge into `cid`, when the tree has one.
    pub fn edge_to(&self, cid: &str) -> Option<&Edge> {
        self.edges.iter().find(|e| e.to == cid)
    }
}

/// The compare document: both programs and the change from a to b (same_as's answer, cut to its musical parts).
pub fn compare_json_ld(origin: &str, a: &Node, b: &Node, c: Option<&Change>) -> Value {
    let mut m = Map::new();
    m.insert("@context".into(), json!([SCHEMA, "https://schema.org"]));
    m.insert("@type".into(), json!("Comparison"));
    m.insert("@id".into(), json!(compare_url(origin, &a.cid, &b.cid)));
    m.insert("from".into(), a.json_ld(origin));
    m.insert("to".into(), b.json_ld(origin));
    match c {
        Some(c) => {
            let j = c.json();
            for k in ["sameSong", "change", "why"] {
                m.insert(k.into(), j[k].clone());
            }
            let r = &c.report;
            if let Some(d) = r.get("diff").filter(|d| d.is_object()) {
                for (k, out) in [("song", "song"), ("arrangement", "arrangement"), ("parts", "parts")] {
                    if let Some(x) = d.get(k).filter(|x| !x.is_null()) {
                        m.insert(out.into(), x.clone());
                    }
                }
            }
        }
        None => {
            m.insert("sameSong".into(), Value::Null);
            m.insert("change".into(), Value::Null);
        }
    }
    Value::Object(m)
}

#[cfg(test)]
mod tests;
