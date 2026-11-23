//! Saved BeatScript programs (docs/BEATSCRIPT.md): licensed objects, as `.beatscript` files.
//!
//! A saved program is a track object (docs/LICENSING.md): the program's canonical text and its
//! rights, CC0-1.0 and public unless said, with an author, `forkedFrom` (the parent program's CID)
//! and the credits CC BY ancestors and CC BY imports bring down the fork chain. Two CIDs:
//!
//! - the **program CID** (`djlive::program_cid`: of the canonical text alone) is what links, imports
//!   and `forkedFrom` name, so formatting and licensing never fork a program;
//! - the **object CID** (of the text and the rights, like a track's) names the file.
//!
//! A file `<object cid>.beatscript` is a BeatScript program with `#!` header lines, which the language reads as the
//! comments they are, so any `.beatscript` runs as it is:
//!
//! ```text
//! #! btw 1
//! #! name: hand-dubstep-140
//! #! license: CC0-1.0
//! #! visibility: public
//! #! author: did:key:z6Mk...
//! #! forked-from: z4EBG9...
//! #! credit: z4EBG9... | did:key:z6Mk... | CC-BY-4.0 | night bus
//! #! genre: https://ontology.bitwrap.io/genre/techno
//! #! tags: https://ontology.bitwrap.io/instrument/acid, https://ontology.bitwrap.io/section/drop
//! tempo 140
//! ...
//! ```
//!
//! `genre` and `tags` (docs/beatscript/ext-linked.md) link the program to the bitwrap ontology: written as full IRIs, read in
//! any spelling the vocabulary knows (`techno`, `genre:techno`, `bw:Techno`), checked against the cached vocabulary
//! (`ontology.rs`) when they are saved or imported (an unknown one is an error with a did-you-mean). They are part of
//! the object, not of the program: a change of genre or tags is a new object of the same program, as a new license is.
//! Next to each object a `<object cid>.jsonld` is written: the program as linked data (`djlive::linked::jsonld`).
//!
//! Stored in `<support>/live` (`$DJ_LIVE_DIR`); files from before there were rights (`<cid>.json`)
//! are still read, as CC0 public programs. A file whose name is not its content's object CID is not
//! listed. `export` makes the file for anywhere on disk and `import_file` takes one in.

use std::path::{Path, PathBuf};

use djengine::json::{JsonObject, JsonValue};
use djengine::rights::{short_cid, Credit, License, Rights, Visibility};
use djlive::diag::{Diagnostic, Severity};
use djlive::env::Snippets;
use djlive::linked::{self, Links};

use crate::beatscript_env::BeatscriptEnv;
use crate::stores::{env_or_support, pretty_sorted, read_json, write_atomic};

pub struct LiveStore {
    pub directory: PathBuf,
}

/// The most programs a store keeps from the links it opened, and the most text of one: what a link carries is bounded
/// (`djlive::share::MAX_CARRIED`), and what is kept from many links is too.
pub const MAX_LINKED: usize = 1024;
pub const MAX_LINKED_TEXT: usize = 1 << 20;

/// The object's `type`.
pub const TYPE: &str = "djrust.btw";

/// One saved program, as a list shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// The program's CID (of its text): what a link names.
    pub cid: String,
    /// The object's CID (text and rights): what the file is named.
    pub object: String,
    pub name: String,
    pub lines: usize,
    pub rights: Rights,
    /// The genre and tags it is linked to (full IRIs).
    pub links: Links,
}

/// A program read back.
#[derive(Clone, Debug, PartialEq)]
pub struct Snippet {
    pub cid: String,
    pub name: String,
    /// The canonical text.
    pub text: String,
    pub rights: Rights,
    pub links: Links,
}

/// A program with its rights, before it is kept.
#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    pub name: String,
    /// The canonical text.
    pub text: String,
    pub rights: Rights,
    /// The genre and tags, as full IRIs once they are kept (as written, when read from a file not yet checked).
    pub links: Links,
}

impl Object {
    pub fn program_cid(&self) -> String {
        djlive::program_cid(&self.text)
    }

    pub fn to_json(&self) -> JsonValue {
        let mut o = JsonObject::new();
        o.set("format", JsonValue::int(1));
        o.set("type", TYPE.into());
        o.set("name", self.name.clone().into());
        o.set("text", self.text.clone().into());
        self.rights.write_into(&mut o);
        // only when there is something to say, so an object that links to nothing is the object it always was
        if let Some(g) = &self.links.genre {
            o.set("genre", g.clone().into());
        }
        if !self.links.tags.is_empty() {
            o.set("tags", JsonValue::Array(self.links.tags.iter().cloned().map(JsonValue::str).collect()));
        }
        JsonValue::Object(o)
    }

    /// The object's CID: of the whole object, rights included.
    pub fn cid(&self) -> String {
        djengine::cid::share_cid_canonical(&self.to_json().canonical_js())
    }

    /// The `.beatscript` file's text: the header, then the program.
    pub fn to_file(&self) -> String {
        // the version line first: a BeatScript 2 program's own (taken off its text, said once), else the old spelling's, as
        // every file this wrote before the rename began (djlive::version)
        let v2 = djlive::version::of_source(&self.text) == djlive::version::Version::V2;
        let body = if v2 { djlive::version::without_version_line(&self.text) } else { self.text.clone() };
        let mut s = format!("{}\n", if v2 { djlive::version::LINE } else { djlive::version::LEGACY_LINE });
        s.push_str(&format!("#! name: {}\n", self.name.replace('\n', " ")));
        s.push_str(&format!("#! license: {}\n", self.rights.license.id()));
        s.push_str(&format!("#! visibility: {}\n", self.rights.visibility.id()));
        if let Some(a) = &self.rights.author {
            s.push_str(&format!("#! author: {a}\n"));
        }
        if let Some(p) = &self.rights.forked_from {
            s.push_str(&format!("#! forked-from: {p}\n"));
        }
        for c in &self.rights.credits {
            s.push_str(&format!("#! credit: {} | {} | {} | {}\n", c.cid, c.author.clone().unwrap_or_default(), c.license.id(), c.name.clone().unwrap_or_default()));
        }
        s.push_str(&self.links.header_text());
        s.push_str(&body);
        s
    }

    /// Read a `.beatscript` file (any program text, with or without the header): the header's name and
    /// rights, the program as written. Rights not in the header are the defaults. The program is
    /// canonicalised by the caller's check.
    pub fn from_file(file: &str) -> Result<(Object, String), String> {
        let mut name = String::new();
        let mut license = License::default();
        let mut visibility = Visibility::default();
        let mut author = None;
        let mut forked_from = None;
        let mut credits = Vec::new();
        let mut genre = None;
        let mut tags = Vec::new();
        let mut body_from = 0;
        for line in file.split_inclusive('\n') {
            let t = line.trim_end();
            let Some(h) = t.strip_prefix("#!") else { break };
            body_from += line.len();
            let h = h.trim();
            let Some((k, v)) = h.split_once(':') else { continue };
            let v = v.trim();
            match k.trim() {
                "name" => name = v.to_string(),
                "license" => license = License::parse(v).ok_or_else(|| format!("the file's license {v:?}: one of {}", License::ids().join(", ")))?,
                "visibility" => visibility = Visibility::parse(v).ok_or_else(|| format!("the file's visibility {v:?}: public or private"))?,
                "author" => author = Some(v.to_string()).filter(|a| !a.is_empty()),
                "forked-from" => forked_from = Some(v.to_string()).filter(|a| !a.is_empty()),
                "genre" => genre = Some(v.to_string()).filter(|a| !a.is_empty()),
                "tags" => tags.extend(linked::split_tags(v)),
                "credit" => {
                    let f: Vec<&str> = v.split('|').map(str::trim).collect();
                    if let Some(cid) = f.first().filter(|c| !c.is_empty()) {
                        credits.push(Credit {
                            cid: cid.to_string(),
                            author: f.get(1).filter(|a| !a.is_empty()).map(|a| a.to_string()),
                            license: f.get(2).and_then(|l| License::parse(l)).unwrap_or(License::CcBy4),
                            name: f.get(3).filter(|a| !a.is_empty()).map(|a| a.to_string()),
                        });
                    }
                }
                _ => {}
            }
        }
        // a BeatScript 2 file's version line is part of its program (djlive::version), so it stays with the text
        let text = match djlive::version::of_source(&file[..body_from]) {
            djlive::version::Version::V2 => format!("{}\n{}", djlive::version::LINE, &file[body_from..]),
            djlive::version::Version::V1 => file[body_from..].to_string(),
        };
        Ok((Object { name, text, rights: Rights { license, visibility, author, forked_from, credits }, links: Links::new(genre, tags) }, file[..body_from].to_string()))
    }
}

/// A change to a saved program's links (`btw save {genre, tags}`): None leaves it, a blank genre or no tags clears it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkEdit {
    pub genre: Option<String>,
    pub tags: Option<Vec<String>>,
}

impl LinkEdit {
    /// From the verb's words: a genre, and tags as `a, b, c`.
    pub fn new(genre: Option<&str>, tags: Option<&str>) -> LinkEdit {
        LinkEdit { genre: genre.map(|g| g.trim().to_string()), tags: tags.map(linked::split_tags) }
    }

    pub fn is_empty(&self) -> bool {
        self.genre.is_none() && self.tags.is_none()
    }

    /// Every spelling in it is a term of the vocabulary, or why not (so a caller can refuse before it keeps anything).
    pub fn check(&self) -> Result<(), String> {
        let genre = self.genre.as_deref().filter(|g| !g.trim().is_empty());
        linked::canonical_links(crate::ontology::vocabulary(), genre, self.tags.as_deref().unwrap_or(&[])).map(|_| ()).map_err(|ps| format!("btw: {}", ps.iter().map(linked::Problem::text).collect::<Vec<_>>().join("; ")))
    }
}

impl LiveStore {
    pub fn new(directory: impl Into<PathBuf>) -> LiveStore {
        LiveStore { directory: directory.into() }
    }

    pub fn default_directory() -> PathBuf {
        env_or_support("DJ_LIVE_DIR", "live")
    }

    pub fn open_default() -> LiveStore {
        LiveStore::new(LiveStore::default_directory())
    }

    fn path(&self, object: &str) -> PathBuf {
        self.directory.join(format!("{object}.beatscript"))
    }

    /// The name every object had before the rename (`<object cid>.btw`): read, listed and removed as the object it is, never
    /// written, so the programs saved before it are all still here (a phone's store, the desktop's) and a saved-again one is
    /// `.beatscript`. (rename-to-beatscript: keep.)
    fn legacy_path(&self, object: &str) -> PathBuf {
        self.directory.join(format!("{object}.btw"))
    }

    /// Where the programs a link carried are kept, text only: `.linked/<program cid>.beatscript`. A dot-directory is no saved
    /// program, so none of them is listed, renamed or credited; they are only found again by their whole CID, by an `import`.
    fn linked_path(&self, cid: &str) -> PathBuf {
        self.directory.join(".linked").join(format!("{cid}.beatscript"))
    }

    /// A program a link carried, kept before the rename (`.linked/<cid>.btw`): read, never written.
    fn linked_legacy_path(&self, cid: &str) -> PathBuf {
        self.directory.join(".linked").join(format!("{cid}.btw"))
    }

    /// Keep the programs a link carried (CID to canonical text, `djlive::share::Link::tracks`), so that a program that imports
    /// them checks here again, offline, when it is edited. Each is kept only as the program its CID names; one that is not
    /// is left out. A link carries text, not rights: nothing here is a saved program with an author or a licence, and
    /// saving one (`btw save`) is a person's choice. Returns how many are kept now that were not.
    pub fn keep_linked(&self, programs: &std::collections::BTreeMap<String, String>) -> usize {
        let mut kept = 0;
        for (cid, text) in programs {
            let plain = !cid.is_empty() && cid.len() <= 128 && cid.chars().all(|c| c.is_ascii_alphanumeric());
            if !plain || djlive::program_cid(text) != *cid || text.len() > MAX_LINKED_TEXT {
                continue;
            }
            let path = self.linked_path(cid);
            if path.exists() || self.linked_legacy_path(cid).exists() || self.linked_count() >= MAX_LINKED {
                continue;
            }
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if write_atomic(&path, text.as_bytes()).is_ok() {
                kept += 1;
            }
        }
        kept
    }

    fn linked_count(&self) -> usize {
        std::fs::read_dir(self.directory.join(".linked")).map_or(0, |rd| rd.flatten().count())
    }

    /// The program a link carried and this store kept, by its whole CID: its canonical text, as the program its CID names.
    pub fn linked(&self, cid: &str) -> Option<String> {
        if cid.is_empty() || !cid.chars().all(|c| c.is_ascii_alphanumeric()) {
            return None;
        }
        let text = std::fs::read_to_string(self.linked_path(cid)).or_else(|_| std::fs::read_to_string(self.linked_legacy_path(cid))).ok()?;
        (djlive::program_cid(&text) == cid).then_some(text)
    }

    /// Why a program that draws on `plays` (`Compiled::plays`) must not be shared: a saved copy of one of them is private (a
    /// link would carry its text), or None.
    pub fn private_among(&self, plays: &[(String, String)]) -> Option<String> {
        let saved = self.list();
        for (cid, _) in plays {
            let copies: Vec<&Entry> = saved.iter().filter(|e| e.cid == *cid).collect();
            if !copies.is_empty() && copies.iter().all(|e| e.rights.check_shareable().is_err()) {
                return Some(format!("it draws on {} ({}), which is private: a link carries the text of every program it draws on", copies[0].name, short_cid(cid)));
            }
        }
        None
    }

    /// The JSON-LD beside an object: `<object cid>.jsonld`.
    pub fn sidecar(&self, object: &str) -> PathBuf {
        self.directory.join(format!("{object}.jsonld"))
    }

    /// An object's file and its JSON-LD go (not a legacy `.json`: the callers know about those).
    fn remove_object(&self, object: &str) {
        let _ = std::fs::remove_file(self.path(object));
        let _ = std::fs::remove_file(self.legacy_path(object));
        let _ = std::fs::remove_file(self.sidecar(object));
    }

    /// Check a program against the registry, the engine's instruments and this store's snippets.
    pub fn check(&self, text: &str) -> djlive::Check {
        let be = BeatscriptEnv::new(Some(self));
        djlive::compile::check_with(text, &be.env())
    }

    /// A program checked and made canonical: its text and CID, or why not.
    fn canonical(&self, text: &str) -> Result<djlive::Compiled, String> {
        let c = self.check(text);
        c.compiled.ok_or_else(|| {
            let n = c.diagnostics.iter().filter(|d| d.is_error()).count();
            format!("btw: not saved, the program has {n} error{}", if n == 1 { "" } else { "s" })
        })
    }

    /// Save `text` as `name` with the default rights (CC0, public, `$DJ_AUTHOR`); the same program saved
    /// again under another name is renamed, keeping its rights.
    pub fn save(&self, name: &str, text: &str) -> Result<Entry, String> {
        self.save_with(name, text, None).map(|x| x.0)
    }

    /// Save with rights (None: those of the original already saved, else the defaults). The credits
    /// of the snippets the program imports (CC BY ones, and what they credit) are added; an
    /// all-rights-reserved import is warned of.
    pub fn save_with(&self, name: &str, text: &str, rights: Option<Rights>) -> Result<(Entry, Vec<String>), String> {
        // `#! genre:` / `#! tags:` lines at the top of the text are the program's links; none: the original's stay
        let links = self.links_in(text, "not saved")?;
        self.save_inner(name, text, rights, links)
    }

    /// `save_with` with the links given (None: those of the original already saved, else none).
    fn save_inner(&self, name: &str, text: &str, rights: Option<Rights>, links: Option<Links>) -> Result<(Entry, Vec<String>), String> {
        let comp = self.canonical(text)?;
        // a name is one line: a newline in it was written as a space by the file but was in the CID (the object then no longer matched its
        // own name and was unlisted, and a second save of the program duplicated it: R-17), so it is a space before the CID is made
        let name: String = name.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
        let name = name.trim();
        let name = if name.is_empty() { format!("btw {}", &comp.cid[comp.cid.len().saturating_sub(6)..]) } else { name.to_string() };
        let existing: Vec<Entry> = self.list().into_iter().filter(|e| e.cid == comp.cid).collect();
        let original = existing.iter().find(|e| e.rights.forked_from.is_none()).cloned();
        let mut rights = match (rights, &original) {
            (Some(r), _) => r,
            (None, Some(o)) => o.rights.clone(),
            (None, None) => Rights::new(crate::tracks::default_author().as_deref()),
        };
        let mut warnings = Vec::new();
        self.add_import_credits(text, &mut rights, &mut warnings);
        let links = links.unwrap_or_else(|| original.as_ref().map(|o| o.links.clone()).unwrap_or_default());
        let obj = Object { name: name.clone(), text: comp.text.clone(), rights, links };
        let entry = self.write(&obj)?;
        if let Some(o) = original {
            if o.object != entry.object {
                self.remove_object(&o.object);
            }
        }
        Ok((entry, warnings))
    }

    fn write(&self, obj: &Object) -> Result<Entry, String> {
        let object = obj.cid();
        write_atomic(&self.path(&object), obj.to_file().as_bytes()).map_err(|e| format!("btw: {e}"))?;
        self.write_sidecar(&object, obj);
        Ok(Entry { cid: obj.program_cid(), object, name: obj.name.clone(), lines: obj.text.lines().count(), rights: obj.rights.clone(), links: obj.links.clone() })
    }

    /// The program as JSON-LD beside its object. It is derived (`btw jsonld` makes it again), so a failure to write it
    /// does not fail the save; a program that no longer compiles (an import that is gone) has none.
    fn write_sidecar(&self, object: &str, obj: &Object) {
        match self.jsonld_object(obj) {
            Some(doc) => {
                let _ = write_atomic(&self.sidecar(object), format!("{}\n", linked::pretty(&doc)).as_bytes());
            }
            None => {
                let _ = std::fs::remove_file(self.sidecar(object));
            }
        }
    }

    /// The program of a saved object as linked data, or None when it does not compile.
    fn jsonld_object(&self, obj: &Object) -> Option<JsonValue> {
        let comp = self.check(&obj.text).compiled?;
        Some(linked::jsonld(&comp, &linked::Facts { name: Some(&obj.name), rights: Some(&obj.rights), links: Some(&obj.links) }, Some(crate::ontology::vocabulary())))
    }

    /// The links the text's `#! genre:` / `#! tags:` header lines say, checked against the vocabulary: None when it
    /// says none, or the errors (with line and column and what to write instead) when a term is not one.
    fn links_in(&self, text: &str, what: &str) -> Result<Option<Links>, String> {
        if !linked::header_lines(text).iter().any(|h| h.key == "genre" || h.key == "tags") {
            return Ok(None);
        }
        let (links, diags) = linked::check_headers(text, crate::ontology::vocabulary(), Severity::Error);
        let errors: Vec<&Diagnostic> = diags.iter().filter(|d| d.is_error()).collect();
        if errors.is_empty() {
            return Ok(Some(links));
        }
        let said: Vec<String> = errors
            .iter()
            .map(|d| {
                let (line, col, _) = d.position(text);
                format!("{} (line {line}, column {col}){}", d.message, d.help.as_ref().map(|h| format!(": {h}")).unwrap_or_default())
            })
            .collect();
        Err(format!("btw: {what}, {}", said.join("; ")))
    }

    /// Genre and tags in any spelling, checked against the vocabulary and made the IRIs they name.
    fn resolve_links(&self, genre: Option<&str>, tags: &[String]) -> Result<Links, String> {
        linked::canonical_links(crate::ontology::vocabulary(), genre, tags).map_err(|ps| format!("btw: {}", ps.iter().map(linked::Problem::text).collect::<Vec<_>>().join("; ")))
    }

    /// Every snippet the program imports is credited the way a fork credits its parent.
    fn add_import_credits(&self, text: &str, rights: &mut Rights, warnings: &mut Vec<String>) {
        let be = BeatscriptEnv::new(Some(self));
        let parsed = djlive::parse::parse_with(text, &be.env());
        // the programs a set plays, and the ones a part or a section is borrowed from, are credited the same way
        let cids: Vec<String> = parsed.imports.iter().map(|i| i.cid.clone()).chain(parsed.plays.iter().map(|p| p.cid.clone())).chain(djlive::stored::cids_of(text)).collect();
        for cid in &cids {
            let Some(e) = self.list().into_iter().find(|e| e.cid == *cid) else { continue };
            let f = e.rights.fork(&e.cid, Some(&e.name), rights.author.as_deref());
            for c in f.rights.credits {
                if !rights.credits.iter().any(|x| x.cid == c.cid) {
                    rights.credits.push(c);
                }
            }
            if e.rights.license == License::AllRightsReserved {
                warnings.push(format!("it draws on {} ({}), which is all rights reserved: ask its author before sharing this", e.name, short_cid(&e.cid)));
            }
        }
    }

    fn read_object(&self, object: &str) -> Option<Object> {
        if let Ok(file) = std::fs::read_to_string(self.path(object)).or_else(|_| std::fs::read_to_string(self.legacy_path(object))) {
            // what is kept is canonical text; the file's name is its content's CID, or it is not ours
            // (an edit shows). Not re-checked here: a program that imports would read the store
            // it is being read from.
            let (o, _) = Object::from_file(&file).ok()?;
            return (o.cid() == object).then_some(o);
        }
        // a snippet saved before there were rights
        let v = read_json(&self.directory.join(format!("{object}.json")))?;
        let text = v.get("text")?.as_str()?.to_string();
        let name = v.get("name").and_then(JsonValue::as_str).unwrap_or("").to_string();
        // its file name is the CID of the form it was saved in, before the canonical form took pitch
        // spelling and param order in: it is read as the program it is, under the CID it has now
        let c = djlive::check(&text).compiled?;
        Some(Object { name, text: c.text, rights: Rights::default(), links: Links::default() })
    }

    /// Every saved program, by name then CID; a file that is not one (or whose name is not its
    /// content's CID: someone edited it) is left out.
    pub fn list(&self) -> Vec<Entry> {
        let Ok(rd) = std::fs::read_dir(&self.directory) else { return Vec::new() };
        let mut v: Vec<Entry> = rd
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                n.strip_suffix(".beatscript").or_else(|| n.strip_suffix(".btw")).or_else(|| n.strip_suffix(".json")).map(str::to_string)
            })
            .filter(|n| !n.starts_with('.') && !n.ends_with(".perf"))
            .filter_map(|id| {
                let o = self.read_object(&id)?;
                // (a legacy file is named by its program's CID: that stays its id until it is saved again)
                Some(Entry { cid: o.program_cid(), object: id, name: o.name.clone(), lines: o.text.lines().count(), rights: o.rights, links: o.links })
            })
            .collect();
        v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.object.cmp(&b.object)));
        v.dedup_by(|a, b| a.object == b.object);
        v
    }

    /// Which program `query` means: a whole CID (the program's or the object's), a head or tail of
    /// one, or a name (exact first, then a part). One match, or an error saying why not.
    pub fn resolve(&self, query: &str) -> Result<Entry, String> {
        let q = query.trim();
        if q.is_empty() {
            return Err("btw: which one? (a cid, or a name; `btw list`)".into());
        }
        let all = self.list();
        let lq = q.to_lowercase();
        let tiers: [Box<dyn Fn(&Entry) -> bool>; 4] = [
            Box::new(|e| e.object == q || e.cid == q),
            Box::new(|e| e.name.to_lowercase() == lq),
            Box::new(|e| e.cid.starts_with(q) || e.cid.ends_with(q) || e.object.starts_with(q) || e.object.ends_with(q)),
            Box::new(|e| e.name.to_lowercase().contains(&lq)),
        ];
        for t in &tiers {
            let hits: Vec<&Entry> = all.iter().filter(|e| t(e)).collect();
            match hits.len() {
                0 => continue,
                1 => return Ok(hits[0].clone()),
                _ => {
                    // the original of a program with forks is the one a program CID means
                    let originals: Vec<&&Entry> = hits.iter().filter(|e| e.rights.forked_from.is_none()).collect();
                    if originals.len() == 1 && hits.iter().all(|e| e.cid == hits[0].cid) {
                        return Ok((**originals[0]).clone());
                    }
                    let names: Vec<String> = hits.iter().take(6).map(|e| format!("{} ({})", e.name, &e.object[e.object.len().saturating_sub(8)..])).collect();
                    return Err(format!("btw {q:?} matches {}: {}", hits.len(), names.join(", ")));
                }
            }
        }
        Err(format!("no saved program matches {q:?} (`btw list`)"))
    }

    pub fn load_object(&self, query: &str) -> Result<Object, String> {
        let e = self.resolve(query)?;
        self.read_object(&e.object).ok_or_else(|| format!("btw {}: unreadable", e.object))
    }

    pub fn load(&self, query: &str) -> Result<Snippet, String> {
        let o = self.load_object(query)?;
        Ok(Snippet { cid: o.program_cid(), name: o.name, text: o.text, rights: o.rights, links: o.links })
    }

    pub fn delete(&self, query: &str) -> Result<Entry, String> {
        let e = self.resolve(query)?;
        // (a snippet saved before there were rights is a `<cid>.json` and has no `.beatscript`: it is gone when that file is, not an error
        // because the other is missing: R-18)
        // (and an object saved before the rename is a `<object>.btw`: whichever of the names it has goes)
        let legacy = self.directory.join(format!("{}.json", e.object));
        let had_legacy = legacy.exists() || self.legacy_path(&e.object).exists();
        let _ = std::fs::remove_file(&legacy);
        let _ = std::fs::remove_file(self.legacy_path(&e.object));
        let _ = std::fs::remove_file(self.sidecar(&e.object));
        std::fs::remove_file(self.path(&e.object)).or_else(|err| if had_legacy && err.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(err) }).map_err(|err| format!("btw: {err}"))?;
        Ok(e)
    }

    /// A new license or visibility: a new object (the rights are in what is addressed); the old
    /// file goes.
    pub fn set_rights(&self, query: &str, license: Option<License>, visibility: Option<Visibility>) -> Result<Entry, String> {
        let e = self.resolve(query)?;
        let mut o = self.read_object(&e.object).ok_or_else(|| format!("btw {}: unreadable", e.object))?;
        if let Some(l) = license {
            o.rights.license = l;
        }
        if let Some(v) = visibility {
            o.rights.visibility = v;
        }
        let new = self.write(&o)?;
        if new.object != e.object {
            self.remove_object(&e.object);
            let _ = std::fs::remove_file(self.directory.join(format!("{}.json", e.object)));
        }
        Ok(new)
    }

    /// A new genre or tags: a new object of the same program (the links are in what is addressed, as the rights are);
    /// the old file goes. Only what `edit` names is looked up: the other half is kept as it is.
    pub fn set_links(&self, query: &str, edit: &LinkEdit) -> Result<Entry, String> {
        let e = self.resolve(query)?;
        let mut o = self.read_object(&e.object).ok_or_else(|| format!("btw {}: unreadable", e.object))?;
        if let Some(g) = &edit.genre {
            o.links.genre = if g.trim().is_empty() { None } else { self.resolve_links(Some(g), &[])?.genre };
        }
        if let Some(t) = &edit.tags {
            o.links.tags = self.resolve_links(None, t)?.tags;
        }
        let new = self.write(&o)?;
        if new.object != e.object {
            self.remove_object(&e.object);
            let _ = std::fs::remove_file(self.directory.join(format!("{}.json", e.object)));
        }
        Ok(new)
    }

    /// A saved program as linked data (`btw jsonld {id}`): the document that sits beside its object.
    pub fn jsonld(&self, query: &str) -> Result<JsonValue, String> {
        let o = self.load_object(query)?;
        self.jsonld_object(&o).ok_or_else(|| "btw jsonld: the program does not compile here (an import is missing?): `btw check` says why".to_string())
    }

    /// A program given as text, or as a `.beatscript` file with `#!` header lines, as linked data (`btw jsonld {text}`). Its
    /// genre and tags are its header lines' (an unknown term is an error); it makes a claim about rights only when the
    /// header has some (a license, visibility, author, forked-from or credit line).
    pub fn jsonld_text(&self, text: &str) -> Result<JsonValue, String> {
        let links = self.links_in(text, "no document")?.unwrap_or_default();
        let (o, _) = Object::from_file(text)?;
        let c = self.check(text);
        let comp = c.compiled.ok_or_else(|| {
            let n = c.diagnostics.iter().filter(|d| d.is_error()).count();
            format!("btw jsonld: the program has {n} error{}: `btw check` says where", if n == 1 { "" } else { "s" })
        })?;
        let has_rights = linked::header_lines(text).iter().any(|h| matches!(h.key.as_str(), "license" | "visibility" | "author" | "forked-from" | "credit"));
        let name = Some(o.name.as_str()).filter(|n| !n.is_empty());
        Ok(linked::jsonld(&comp, &linked::Facts { name, rights: has_rights.then_some(&o.rights), links: Some(&links) }, Some(crate::ontology::vocabulary())))
    }

    /// A fork by `author`: the same program, `forkedFrom` its CID, the license and visibility
    /// and credits carried (a CC BY parent is credited). Saved; answers it and the warnings.
    pub fn fork(&self, query: &str, name: Option<&str>, author: Option<&str>) -> Result<(Entry, Vec<String>), String> {
        let parent = self.resolve(query)?;
        let o = self.read_object(&parent.object).ok_or_else(|| format!("btw {}: unreadable", parent.object))?;
        let f = o.rights.fork(&parent.cid, Some(&o.name), author.map(str::to_string).or_else(crate::tracks::default_author).as_deref());
        let name = name.map(str::trim).filter(|n| !n.is_empty()).map_or_else(|| format!("{} (fork)", o.name), str::to_string);
        let child = Object { name, text: o.text.clone(), rights: f.rights, links: o.links.clone() };
        let e = self.write(&child)?;
        Ok((e, f.warnings))
    }

    /// The `.beatscript` file for a saved program.
    pub fn export(&self, query: &str) -> Result<(Entry, String), String> {
        let e = self.resolve(query)?;
        let o = self.read_object(&e.object).ok_or_else(|| format!("btw {}: unreadable", e.object))?;
        Ok((e, o.to_file()))
    }

    pub fn export_to(&self, query: &str, path: &Path) -> Result<Entry, String> {
        let (e, file) = self.export(query)?;
        let path = if path.is_dir() { path.join(format!("{}.beatscript", file_name(&e.name))) } else { path.to_path_buf() };
        std::fs::write(&path, file).map_err(|err| format!("btw: {}: {err}", path.display()))?;
        Ok(e)
    }

    /// Take a `.beatscript` file's text in: checked, made canonical, kept with the rights its header gives.
    pub fn import_text(&self, file: &str, fallback_name: &str) -> Result<(Entry, Vec<String>), String> {
        let (o, _) = Object::from_file(file)?;
        // the file's genre and tags are looked up like a save's: in any spelling, an unknown one is an error
        let links = self.links_in(file, "not imported")?.unwrap_or_default();
        let name = if o.name.is_empty() { fallback_name.to_string() } else { o.name.clone() };
        let comp = self.canonical(&o.text)?;
        let mut warnings = Vec::new();
        if o.rights.visibility == Visibility::Private {
            warnings.push("the file says it is private: it stays on this machine".to_string());
        }
        let (e, w) = self.save_inner(&name, &comp.text, Some(o.rights), Some(links))?;
        warnings.extend(w);
        Ok((e, warnings))
    }

    pub fn import_file(&self, path: &Path) -> Result<(Entry, Vec<String>), String> {
        let file = std::fs::read_to_string(path).map_err(|e| format!("btw: {}: {e}", path.display()))?;
        // (an editor's byte-order mark is not part of the program, nor of its `#!` header lines: R-19)
        let file = file.trim_start_matches('\u{feff}').to_string();
        let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        self.import_text(&file, &stem)
    }
}

/// One saved performance, as a list shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct PerfEntry {
    /// The performance's own CID: what its file is named by and what `replay` names.
    pub cid: String,
    /// The program it was played on (its CID).
    pub program: String,
    /// The name of that program, when it is saved here.
    pub program_name: Option<String>,
    pub readings: usize,
    /// The first and last program bar read.
    pub bars: Option<(i64, i64)>,
    /// The inputs it has values for (`knob(3)` ...).
    pub inputs: Vec<String>,
}

/// A performance (docs/beatscript/ext-perform.md): `<cid>.perf.json`, the object `{type: djrust.performance, program, inputs}` named by its
/// own CID, next to the programs it was played on. A file whose name is not its content's CID has been edited, and is not listed.
impl LiveStore {
    fn perf_path(&self, cid: &str) -> PathBuf {
        self.directory.join(format!("{cid}.perf.json"))
    }

    /// Keep a performance; answers its CID. The same performance saved again is the same file.
    pub fn save_performance(&self, perf: &djlive::perform::Performance) -> Result<String, String> {
        if perf.inputs.is_empty() && perf.launches.is_empty() {
            return Err("btw: nothing was read: a performance is the values a program's inputs read (and the scenes a pad brought in), and none has been".into());
        }
        let cid = perf.cid();
        write_atomic(&self.perf_path(&cid), pretty_sorted(&perf.to_json()).as_bytes()).map_err(|e| format!("btw: {e}"))?;
        Ok(cid)
    }

    fn read_performance(&self, cid: &str) -> Option<djlive::perform::Performance> {
        let v = read_json(&self.perf_path(cid))?;
        let p = djlive::perform::Performance::from_json(&v).ok()?;
        // the file's name is its content's CID, or it is not ours (an edit shows)
        (p.cid() == cid).then_some(p)
    }

    /// Every saved performance, by program then CID.
    pub fn list_performances(&self) -> Vec<PerfEntry> {
        let Ok(rd) = std::fs::read_dir(&self.directory) else { return Vec::new() };
        let programs = self.list();
        let mut v: Vec<PerfEntry> = rd
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".perf.json").map(str::to_string))
            .filter(|n| !n.starts_with('.'))
            .filter_map(|cid| {
                let p = self.read_performance(&cid)?;
                Some(PerfEntry {
                    program_name: programs.iter().find(|e| e.cid == p.program).map(|e| e.name.clone()),
                    program: p.program.clone(),
                    readings: p.inputs.len(),
                    bars: p.span_bars(),
                    inputs: p.input_ids(),
                    cid,
                })
            })
            .collect();
        v.sort_by(|a, b| a.program.cmp(&b.program).then(a.cid.cmp(&b.cid)));
        v
    }

    /// Which performance `query` means: a whole CID, or the head or tail of one. One match, or an error saying why not.
    pub fn resolve_performance(&self, query: &str) -> Result<PerfEntry, String> {
        let q = query.trim();
        if q.is_empty() {
            return Err("btw replay: which performance? (a cid; `btw perform` says the take's)".into());
        }
        let all = self.list_performances();
        for tier in [|e: &PerfEntry, q: &str| e.cid == q, |e: &PerfEntry, q: &str| e.cid.starts_with(q) || e.cid.ends_with(q)] {
            let hits: Vec<&PerfEntry> = all.iter().filter(|e| tier(e, q)).collect();
            match hits.len() {
                0 => continue,
                1 => return Ok(hits[0].clone()),
                n => return Err(format!("btw replay {q:?} matches {n} performances: {}", hits.iter().take(6).map(|e| e.cid.chars().rev().take(8).collect::<Vec<_>>().into_iter().rev().collect::<String>()).collect::<Vec<_>>().join(", "))),
            }
        }
        Err(format!("no saved performance matches {q:?} (a performance is saved with `btw perform {{action: save}}`)"))
    }

    pub fn load_performance(&self, query: &str) -> Result<djlive::perform::Performance, String> {
        let e = self.resolve_performance(query)?;
        self.read_performance(&e.cid).ok_or_else(|| format!("btw {}: unreadable", e.cid))
    }

    /// The program a performance was played on, from the store: its text, checked to be the program the performance names.
    pub fn performance_program(&self, perf: &djlive::perform::Performance) -> Result<Snippet, String> {
        let s = self.load(&perf.program).map_err(|e| format!("the performance is of program {}, which is not saved here ({e}): save it, or give its file", perf.program))?;
        if s.cid != perf.program {
            return Err(format!("refused: the performance is of program {}, and storage answered {}", perf.program, s.cid));
        }
        Ok(s)
    }
}

fn file_name(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
    if s.is_empty() {
        "program".into()
    } else {
        s
    }
}

impl Snippets for LiveStore {
    fn label(&self, cid: &str) -> Option<String> {
        self.list().into_iter().find(|e| e.cid == cid).map(|e| e.name)
    }

    fn resolve(&self, query: &str) -> Result<(String, String), String> {
        match self.load_object(query) {
            Ok(o) => Ok((o.program_cid(), o.text)),
            // not saved here: a program a link carried and this store kept is found by its whole CID
            Err(e) => self.linked(query).map(|t| (query.to_string(), t)).ok_or(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stores::pretty_sorted;

    fn store() -> (LiveStore, PathBuf) {
        store_named("a")
    }

    fn store_named(tag: &str) -> (LiveStore, PathBuf) {
        let dir = std::env::temp_dir().join(format!("djlive-store-{tag}-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&dir);
        (LiveStore::new(&dir), dir)
    }

    #[test]
    fn save_list_load_by_cid_part_and_name() {
        let (s, dir) = store();
        let a = s.save("backbeat", "deck A: kick x ~ x ~   # four on the floor\n").unwrap();
        assert!(a.cid.starts_with('z'));
        // the same program in other spacing is the same snippet: renamed, not added
        let b = s.save("the backbeat", "deck A:   kick x ~ x ~\n").unwrap();
        assert_eq!(a.cid, b.cid);
        assert_eq!(s.list().len(), 1);
        assert_eq!(s.list()[0].name, "the backbeat");
        let c = s.save("acid", "deck A: bass markov(am, 0.7)\n").unwrap();
        assert_eq!(s.list().len(), 2);
        // the saved text is canonical
        assert_eq!(s.load(&a.cid).unwrap().text, "deck A: kick x ~ x ~\n");
        assert_eq!(s.load("acid").unwrap().cid, c.cid);
        assert_eq!(s.load(&c.cid[c.cid.len() - 8..]).unwrap().cid, c.cid);
        assert!(s.load("zzz-nothing").is_err());
        let d = s.delete("acid").unwrap();
        assert_eq!(d.cid, c.cid);
        assert_eq!(s.list().len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn programs_saved_before_the_rename_as_btw_files_are_still_listed_read_and_deleted() {
        let (s, dir) = store();
        let e = s.save("old one", "deck A: kick x ~ x ~\n").unwrap();
        // as the store wrote it before the rename: `<object>.btw`
        std::fs::rename(dir.join(format!("{}.beatscript", e.object)), dir.join(format!("{}.btw", e.object))).unwrap();
        let listed = s.list();
        assert_eq!(listed.len(), 1, "the .btw object is listed");
        assert_eq!((listed[0].object.as_str(), listed[0].name.as_str()), (e.object.as_str(), "old one"));
        assert!(s.load_object(&e.object).is_ok(), "and read");
        // a new licence makes a new object, written .beatscript; the .btw one goes
        let moved = s.set_rights(&e.object, Some(License::CcBy4), None).unwrap();
        assert!(dir.join(format!("{}.beatscript", moved.object)).exists() && !dir.join(format!("{}.btw", e.object)).exists());
        // and one deleted while it is still .btw is gone
        let e2 = s.save("another", "deck A: kick x x ~ ~\n").unwrap();
        std::fs::rename(dir.join(format!("{}.beatscript", e2.object)), dir.join(format!("{}.btw", e2.object))).unwrap();
        s.delete(&e2.object).unwrap();
        assert!(!dir.join(format!("{}.btw", e2.object)).exists());
        // a program a link carried, kept before the rename
        let text = "tempo 120\n";
        std::fs::create_dir_all(dir.join(".linked")).unwrap();
        std::fs::write(dir.join(".linked").join(format!("{}.btw", djlive::program_cid(text))), text).unwrap();
        assert_eq!(s.linked(&djlive::program_cid(text)).as_deref(), Some(text));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_program_with_errors_is_not_saved_and_an_edited_file_is_not_listed() {
        let (s, dir) = store();
        assert!(s.save("bad", "deck A: kick q\n").unwrap_err().contains("1 error"));
        assert!(s.list().is_empty());
        let e = s.save("ok", "deck A: kick x ~ x ~\n").unwrap();
        let path = dir.join(format!("{}.beatscript", e.object));
        let edited = std::fs::read_to_string(&path).unwrap().replace("kick x ~ x ~", "kick x x x x");
        std::fs::write(&path, edited).unwrap();
        assert!(s.list().is_empty(), "its text is no longer what its name says");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn ambiguous_names_say_which() {
        let (s, dir) = store();
        s.save("loop one", "deck A: kick x ~ x ~\n").unwrap();
        s.save("loop two", "deck A: snare ~ x ~ x\n").unwrap();
        let e = s.resolve("loop").unwrap_err();
        assert!(e.contains("matches 2"), "{e}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_saved_program_is_a_licensed_object_and_a_fork_records_its_parent() {
        let (s, dir) = store();
        let a = s.save("night bus", "deck A: kick x ~ x ~\n").unwrap();
        assert_eq!(a.rights.license, License::Cc0);
        assert_eq!(a.rights.visibility, Visibility::Public);
        // a new license is a new object, the same program
        let b = s.set_rights("night bus", Some(License::CcBy4), None).unwrap();
        assert_eq!(b.cid, a.cid);
        assert_ne!(b.object, a.object);
        assert_eq!(s.list().len(), 1);
        let (f, w) = s.fork("night bus", Some("night bus 2"), Some("did:key:z6MkFork")).unwrap();
        assert!(w.iter().all(|x| !x.contains("all rights reserved")), "{w:?}");
        assert_eq!(f.rights.forked_from.as_deref(), Some(a.cid.as_str()));
        assert_eq!(f.rights.license, License::CcBy4);
        assert_eq!(f.rights.credits.len(), 1, "a CC BY parent is credited");
        assert_eq!(f.rights.credits[0].cid, a.cid);
        assert_eq!(s.list().len(), 2);
        // the program CID still names the original
        assert_eq!(s.resolve(&a.cid).unwrap().name, "night bus");
        // a fork of an all-rights-reserved program warns
        s.set_rights("night bus", Some(License::AllRightsReserved), None).unwrap();
        let (_, w) = s.fork("night bus", Some("third"), None).unwrap();
        assert!(w.iter().any(|x| x.contains("all rights reserved")), "{w:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_import_carries_credit_down_the_fork_chain() {
        let (s, dir) = store();
        let lib = s.save_with("riffs", "let riff = c2 e2 g2 b2\n", Some(Rights { license: License::CcBy4, author: Some("did:key:z6MkLib".into()), ..Rights::default() })).unwrap().0;
        let (song, _) = s.save_with("song", &format!("import {}\ndeck A: bass riff\n", lib.cid), None).unwrap();
        assert_eq!(song.rights.credits.len(), 1, "{:?}", song.rights);
        assert_eq!(song.rights.credits[0].cid, lib.cid);
        assert_eq!(song.rights.credits[0].author.as_deref(), Some("did:key:z6MkLib"));
        // a fork of the song carries it on
        let (child, _) = s.fork("song", None, None).unwrap();
        assert!(child.rights.credits.iter().any(|c| c.cid == lib.cid));
        // the import is pinned: the canonical text holds the CID, not the name
        let named = s.save("by name", "import \"riffs\"\ndeck A: bass riff | kick x\n").unwrap();
        let text = s.load(&named.cid).unwrap().text;
        assert!(text.starts_with(&format!("import {}", lib.cid)), "{text}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_borrowed_section_credits_its_source_and_warns_of_one_that_is_all_rights_reserved() {
        let (s, dir) = store_named("borrow-credit");
        let src = "tempo 100\nsection verse 2 bars {\n  deck A: kick x ~ x ~ | bass c2 ~ e2 ~\n}\narrange verse\n";
        let lib = s.save_with("verses", src, Some(Rights { license: License::CcBy4, author: Some("did:key:z6MkLib".into()), ..Rights::default() })).unwrap().0;
        let (song, _) = s.save_with("song", &format!("tempo 100\nsection hook from {}.verse\narrange hook\n", lib.cid), None).unwrap();
        assert_eq!(song.rights.credits.len(), 1, "{:?}", song.rights);
        assert_eq!((song.rights.credits[0].cid.as_str(), song.rights.credits[0].author.as_deref()), (lib.cid.as_str(), Some("did:key:z6MkLib")));
        // the saved text keeps the reference, CID and all
        assert!(s.load(&song.cid).unwrap().text.contains(&format!("section hook from {}.verse", lib.cid)));
        // an all-rights-reserved source is warned of, as an import's is
        let closed = s.save_with("closed", "tempo 90\nsection chorus 2 bars {\n  deck A: snare x ~ x ~\n}\narrange chorus\n", Some(Rights { license: License::AllRightsReserved, ..Rights::default() })).unwrap().0;
        let (_, warnings) = s.save_with("mine", &format!("tempo 90\nsection hook from {}.chorus\narrange hook\n", closed.cid), None).unwrap();
        assert!(warnings.iter().any(|w| w.contains("which is all rights reserved")), "{warnings:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_btw_file_round_trips_with_its_rights_and_runs_as_it_is() {
        let (s, dir) = store();
        let a = s.save_with("drop", "deck A: kick x x x x\n", Some(Rights { license: License::CcBy4, author: Some("matt".into()), ..Rights::default() })).unwrap().0;
        let out = dir.join("export");
        std::fs::create_dir_all(&out).unwrap();
        s.export_to("drop", &out).unwrap();
        let file = std::fs::read_to_string(out.join("drop.beatscript")).unwrap();
        assert!(file.starts_with("#! btw 1\n#! name: drop\n#! license: CC-BY-4.0"), "{file}");
        // it is a program as it stands
        assert!(djlive::check(&file).ok());
        assert_eq!(djlive::check(&file).compiled.unwrap().cid, a.cid);
        let (s2, dir2) = store_named("b");
        let (e, _) = s2.import_file(&out.join("drop.beatscript")).unwrap();
        assert_eq!(e.cid, a.cid);
        assert_eq!(e.rights.license, License::CcBy4);
        assert_eq!(e.rights.author.as_deref(), Some("matt"));
        assert_eq!(e.object, a.object, "the same object on the other side");
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(dir2);
    }

    #[test]
    fn the_header_lines_for_links_follow_the_rights_and_read_back_as_written() {
        let o = Object {
            name: "n".into(),
            text: "deck A: kick x\n".into(),
            rights: Rights { author: Some("a".into()), ..Rights::default() },
            links: Links::new(Some("https://ontology.bitwrap.io/genre/techno".into()), vec!["https://ontology.bitwrap.io/section/drop".into(), "https://ontology.bitwrap.io/instrument/acid".into()]),
        };
        let file = o.to_file();
        assert_eq!(
            file,
            "#! btw 1\n#! name: n\n#! license: CC0-1.0\n#! visibility: public\n#! author: a\n#! genre: https://ontology.bitwrap.io/genre/techno\n#! tags: https://ontology.bitwrap.io/instrument/acid, https://ontology.bitwrap.io/section/drop\ndeck A: kick x\n"
        );
        let (back, header) = Object::from_file(&file).unwrap();
        assert_eq!(back, o);
        assert_eq!(header.lines().count(), 7);
        // as written, not looked up: the spelling is kept until a store resolves it
        let (raw, _) = Object::from_file("#! genre: Techno\n#! tags: drop,  acid ,,acid\ndeck A: kick x\n").unwrap();
        assert_eq!((raw.links.genre.as_deref(), raw.links.tags.as_slice()), (Some("Techno"), ["acid".to_string(), "drop".to_string()].as_slice()));
        // an object that links to nothing has no `genre` / `tags` and so the CID it always had
        let bare = Object { links: Links::default(), ..o.clone() };
        assert!(!bare.to_json().canonical_js().contains("genre") && !bare.to_json().canonical_js().contains("tags"));
        assert_ne!(bare.cid(), o.cid(), "a genre or tags change is a new object");
        assert_eq!(bare.program_cid(), o.program_cid(), "of the same program");
    }

    #[test]
    fn a_performance_is_saved_by_its_own_cid_and_listed_next_to_its_program() {
        use djlive::perform::{Performance, Reading};
        let (s, dir) = store_named("perf");
        let prog = s.save("wob", "every 1 bars: master: level knob(1)\n").unwrap();
        let perf = Performance::new(prog.cid.clone(), vec![Reading { bar: 1, at: 0, input: "knob(1)".into(), value: 0.5 }, Reading { bar: 2, at: 0, input: "knob(1)".into(), value: 0.25 }]);
        let cid = s.save_performance(&perf).unwrap();
        assert_eq!(cid, perf.cid());
        // a performance is not a program: the program list is what it was
        assert_eq!(s.list().len(), 1);
        let l = s.list_performances();
        assert_eq!(l.len(), 1);
        assert_eq!((l[0].cid.as_str(), l[0].program.as_str(), l[0].program_name.as_deref(), l[0].readings, l[0].bars), (cid.as_str(), prog.cid.as_str(), Some("wob"), 2, Some((1, 2))));
        assert_eq!(l[0].inputs, vec!["knob(1)".to_string()]);
        // round trip, by whole CID, its head and its tail
        assert_eq!(s.load_performance(&cid).unwrap(), perf);
        assert_eq!(s.load_performance(&cid[..12]).unwrap().cid(), cid);
        assert_eq!(s.load_performance(&cid[cid.len() - 8..]).unwrap().cid(), cid);
        assert!(s.load_performance("zNope").unwrap_err().contains("no saved performance"));
        // the same performance again is the same file
        assert_eq!(s.save_performance(&perf).unwrap(), cid);
        assert_eq!(s.list_performances().len(), 1);
        // the program it was played on is found from it
        assert_eq!(s.performance_program(&perf).unwrap().text, "every 1 bars: master: level knob(1)\n");
        let lost = Performance::new("zNoSuchProgram", perf.inputs.clone());
        assert!(s.performance_program(&lost).unwrap_err().contains("not saved here"));
        // nothing read is not a performance
        assert!(s.save_performance(&Performance::new("zProg", vec![])).unwrap_err().contains("nothing was read"));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// F13 meets F14: a performance names its program by the program's CID, which is of the canonical text. The `#! genre:` and
    /// `#! tags:` header lines are comments to the language, so linking a program (or changing its links) makes another object
    /// of the same program: a performance saved before is still found, with the program it was played on, and replays.
    #[test]
    fn a_performance_follows_its_program_through_links_added_after_and_replays() {
        use djlive::perform::{Performance, Reading};
        let (s, dir) = store_named("perf-links");
        let plain = "tempo 124\ndeck A: kick x ~ x ~\nevery 1 bars: master: filter lp 200 + knob(1) * 7800\n";
        let linked_text = format!("#! genre: techno\n#! tags: acid, drop\n{plain}");
        // a program saved with its links: the header lines are not in its CID
        let e = s.save("acid", &linked_text).unwrap();
        assert!(e.links.genre.is_some() && e.links.tags.len() == 2, "{:?}", e.links);
        assert_eq!(e.cid, djlive::program_cid(plain), "the header lines are comments to the language");
        assert!(s.sidecar(&e.object).exists(), "its JSON-LD is beside it");
        // a performance of it: knob(1) read at bars 1..4
        let readings: Vec<Reading> = (1..=4).map(|bar| Reading { bar, at: 0, input: "knob(1)".into(), value: bar as f64 / 8.0 }).collect();
        let perf = Performance::new(e.cid.clone(), readings);
        let cid = s.save_performance(&perf).unwrap();
        let listed = s.list_performances();
        assert_eq!((listed.len(), listed[0].program_name.as_deref(), listed[0].program.as_str()), (1, Some("acid"), e.cid.as_str()));
        // the program is found from the performance, with its links, and is the program the performance names
        let snip = s.performance_program(&perf).unwrap();
        assert_eq!(snip.cid, perf.program);
        assert_eq!(snip.links, e.links);
        let env = BeatscriptEnv::new(Some(&s));
        let want = crate::beatscript_render::replay_controls(&snip.text, &env.env(), &perf, 6).unwrap();
        let sweeps: Vec<_> = want.iter().filter(|c| c.noun == "inserts" && c.params.get("param").and_then(JsonValue::as_str) == Some("filter.sweep")).collect();
        assert!(sweeps.len() >= 3, "{want:?}");
        // the links change (a tag added): a new object of the same program, one program in the list, the performance still found
        let (e2, _) = s.save_with("acid", &format!("#! genre: techno\n#! tags: acid, drop, syncopation\n{plain}"), None).unwrap();
        assert_eq!(e2.cid, e.cid);
        assert_ne!(e2.object, e.object, "a tag change is a new object");
        assert_eq!(s.list().len(), 1);
        assert_eq!(s.list_performances().len(), 1);
        assert_eq!(s.load_performance(&cid).unwrap(), perf);
        let snip2 = s.performance_program(&perf).unwrap();
        assert_eq!((snip2.cid.as_str(), snip2.links.tags.len()), (perf.program.as_str(), 3));
        // and it replays to the very same calls
        let again = crate::beatscript_render::replay_controls(&snip2.text, &env.env(), &perf, 6).unwrap();
        assert_eq!(again, want);
        // a program saved without links first, linked after: the performance saved before is still its performance
        let (s2, dir2) = store_named("perf-links-after");
        let e0 = s2.save("acid", plain).unwrap();
        assert!(e0.links.genre.is_none());
        let p0 = Performance::new(e0.cid.clone(), vec![Reading { bar: 2, at: 0, input: "knob(1)".into(), value: 0.5 }]);
        s2.save_performance(&p0).unwrap();
        s2.save_with("acid", &linked_text, None).unwrap();
        assert_eq!(s2.performance_program(&p0).unwrap().links.genre, e.links.genre);
        assert_eq!(s2.list_performances()[0].program_name.as_deref(), Some("acid"));
        // the store's listing of programs does not mistake a performance or a JSON-LD sidecar for one
        assert_eq!(s.list().len(), 1);
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(dir2);
    }

    #[test]
    fn an_edited_performance_file_is_not_listed_and_not_replayed() {
        use djlive::perform::{Performance, Reading};
        let (s, dir) = store_named("perf-edit");
        let perf = Performance::new("zProg", vec![Reading { bar: 1, at: 0, input: "knob(1)".into(), value: 0.5 }]);
        let cid = s.save_performance(&perf).unwrap();
        let path = dir.join(format!("{cid}.perf.json"));
        let edited = std::fs::read_to_string(&path).unwrap().replace("0.5", "0.75");
        std::fs::write(&path, edited).unwrap();
        assert!(s.list_performances().is_empty(), "its name is no longer its content's CID");
        assert!(s.load_performance(&cid).unwrap_err().contains("no saved performance"));
        // a file that is not a performance at all
        std::fs::write(dir.join("zJunk.perf.json"), "{\"type\":\"djrust.live\"}").unwrap();
        assert!(s.list_performances().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn programs_saved_before_there_were_rights_are_still_read() {
        let (s, dir) = store();
        std::fs::create_dir_all(&dir).unwrap();
        let c = djlive::check("deck A: kick x ~ x ~\n").compiled.unwrap();
        let doc = JsonValue::Object(JsonObject::from_pairs([("cid", JsonValue::str(c.cid.clone())), ("format", JsonValue::int(1)), ("name", JsonValue::str("old")), ("text", JsonValue::str(c.text.clone())), ("type", JsonValue::str("djrust.live"))]));
        std::fs::write(dir.join(format!("{}.json", c.cid)), pretty_sorted(&doc)).unwrap();
        let l = s.list();
        assert_eq!(l.len(), 1);
        assert_eq!((l[0].name.as_str(), l[0].rights.license), ("old", License::Cc0));
        assert_eq!(s.load("old").unwrap().cid, c.cid);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_older_snippet_whose_text_was_not_yet_canonical_is_still_listed() {
        let (s, dir) = store();
        std::fs::create_dir_all(&dir).unwrap();
        // saved when `Bb2` and `eb1` were kept as written: its file is named by that older CID
        let text = "deck A: bass Bb2 eb1 vel=100 oct=2\n";
        let doc = JsonValue::Object(JsonObject::from_pairs([("cid", JsonValue::str("zOlderCid")), ("format", JsonValue::int(1)), ("name", JsonValue::str("older")), ("text", JsonValue::str(text)), ("type", JsonValue::str("djrust.live"))]));
        std::fs::write(dir.join("zOlderCid.json"), pretty_sorted(&doc)).unwrap();
        let l = s.list();
        assert_eq!(l.len(), 1, "{l:?}");
        assert_eq!(l[0].name, "older");
        let loaded = s.load("older").unwrap();
        assert_eq!(loaded.text, "deck A: bass a#2 d#1 oct=2 vel=100\n");
        assert_eq!(loaded.cid, djlive::check(text).compiled.unwrap().cid);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_program_a_link_carried_is_kept_unlisted_and_found_by_its_whole_cid() {
        let (s, dir) = store_named("linked");
        let riffs = djlive::check("let riff = c2 e2 g2 b2\n").compiled.unwrap();
        let mut bundle = std::collections::BTreeMap::new();
        bundle.insert(riffs.cid.clone(), riffs.text.clone());
        // a text that is not the program its CID names, and a name that is not a CID at all, are not kept
        bundle.insert("zNotTheOne".to_string(), riffs.text.clone());
        bundle.insert("../escape".to_string(), riffs.text.clone());
        assert_eq!(s.keep_linked(&bundle), 1);
        assert_eq!(s.keep_linked(&bundle), 0, "kept once");
        // not a saved program: no listing, no rename, no rights; and an import by its CID finds it, so an edit checks offline
        assert!(s.list().is_empty());
        assert_eq!(s.linked(&riffs.cid).as_deref(), Some(riffs.text.as_str()));
        assert!(s.linked("zNotTheOne").is_none());
        let text = format!("import {}\ndeck A: bass riff\n", riffs.cid);
        let c = s.check(&text);
        assert!(c.compiled.is_some(), "{:?}", c.diagnostics);
        // a name still needs a saved program, and a partial CID too
        assert!(s.check("import \"riffs\"\ndeck A: bass riff\n").compiled.is_none());
        // a file someone edited is not the program its name says: not found
        std::fs::write(dir.join(".linked").join(format!("{}.beatscript", riffs.cid)), "let riff = c2\n").unwrap();
        assert!(s.linked(&riffs.cid).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_program_that_draws_on_a_private_saved_one_is_not_shared() {
        let (s, dir) = store_named("private-among");
        let e = s.save("secret riffs", "let riff = c2 e2 g2 b2\n").unwrap();
        let plays = vec![(e.cid.clone(), "let riff = c2 e2 g2 b2\n".to_string())];
        assert_eq!(s.private_among(&plays), None);
        s.set_rights(&e.cid, None, Some(Visibility::Private)).unwrap();
        let why = s.private_among(&plays).expect("private");
        assert!(why.contains("secret riffs") && why.contains("which is private"), "{why}");
        // a program the store has not saved says nothing here
        assert_eq!(s.private_among(&[("zElsewhere".to_string(), String::new())]), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
