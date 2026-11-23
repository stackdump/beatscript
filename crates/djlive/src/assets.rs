//! Local sounds a program may use without owning the right to give them away (docs/BEATSCRIPT.md, "Restricted
//! sounds"): `sample NAME from "SRC" [license TAG] [else VOICE]`, `loop "NAME-OR-HASH" as NAME [else VOICE]` (an
//! Apple Loop) and `use restricted`.
//!
//! A sample is a *name for a file*. The program holds the file's SHA-256, never its bytes and never its path
//! (the canonical text says `from "sha256:..."`), so a link, the repo or a store cannot carry the sound. Nothing
//! here fetches a file: the host (`djhost::assets`) finds it on this machine or says it is missing.
//!
//! Rights are a tier of the assets a program names, the most restrictive winning:
//!
//! | tier | assets | the program may |
//! |---|---|---|
//! | `open` | `cc0`, `cc-by` | be shared and made public as its own license says |
//! | `apple-loops` | Apple Loops | be shared (hash references; the recipient needs the loops), rendered and released; all rights reserved plus an Apple Loops credit; never CC0 or CC BY |
//! | `all-rights-reserved` | a local sample tagged so | as `apple-loops` without the credit |
//! | `private` | `restricted-private` (the default for a local file) | only be private: `use restricted` (or `#! visibility: private`) is required, and it is never shared, made public or released |
//!
//! The tier comes from the *text* (each declaration carries its tag), so it is known without the files; the files
//! are looked up only to hash them and to say what is missing.

use std::collections::BTreeMap;

use crate::ast::{Program, SampleDecl, SampleKind, StmtKind};
use crate::diag::{Diagnostic, Span};
use crate::env::Env;
use crate::parse::Parsed;

/// What the host knows about a file it found.
#[derive(Clone, Debug, PartialEq)]
pub struct AssetInfo {
    /// SHA-256 of the file's bytes, lower-case hex (64 characters).
    pub hash: String,
    /// The file's name without its extension, for messages.
    pub name: String,
    pub origin: Origin,
    pub bpm: Option<f64>,
    pub key: Option<String>,
    pub beats: Option<f64>,
    pub genre: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Origin {
    /// A file the user supplied from their own copy.
    Local,
    /// An Apple Loop of GarageBand / Logic.
    AppleLoops,
}

/// Finds files on this machine. The app's index (`djhost::assets::AssetStore`) implements it.
pub trait Assets {
    /// `SRC` of a `sample` (a path, or `sha256:HEX`), or of a `loop` (an Apple Loop's name, or its hash): the
    /// file's info, or why it is not found.
    fn resolve(&self, src: &str, kind: SampleKind) -> Result<AssetInfo, String>;
}

/// The license tag of an asset, from most open to most restricted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AssetRights {
    Cc0,
    CcBy,
    AllRightsReserved,
    RestrictedPrivate,
}

impl AssetRights {
    pub const IDS: [&'static str; 4] = ["cc0", "cc-by", "all-rights-reserved", "restricted-private"];

    pub fn id(self) -> &'static str {
        Self::IDS[self as usize]
    }

    pub fn parse(s: &str) -> Option<AssetRights> {
        match s.to_ascii_lowercase().as_str() {
            "cc0" | "cc0-1.0" => Some(AssetRights::Cc0),
            "cc-by" | "cc-by-4.0" | "ccby" => Some(AssetRights::CcBy),
            "all-rights-reserved" | "arr" => Some(AssetRights::AllRightsReserved),
            "restricted-private" | "restricted" | "private" => Some(AssetRights::RestrictedPrivate),
            _ => None,
        }
    }
}

/// The tier a program's assets put it in (see the module's table).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    Open,
    AppleLoops,
    AllRightsReserved,
    Private,
}

impl Tier {
    pub fn id(self) -> &'static str {
        match self {
            Tier::Open => "open",
            Tier::AppleLoops => "apple-loops",
            Tier::AllRightsReserved => "all-rights-reserved",
            Tier::Private => "private",
        }
    }
}

/// The credit line an Apple Loops program carries.
pub const APPLE_CREDIT: &str = "Apple Loops © Apple Inc., used under the GarageBand / Logic license (royalty-free in your own compositions; the loops are not redistributed)";

/// One asset a program names.
#[derive(Clone, Debug, PartialEq)]
pub struct AssetUse {
    /// The name the program gives it (`inst=NAME`).
    pub name: String,
    /// `sha256:HEX` once the file is found; what the text says until then.
    pub src: String,
    pub kind: SampleKind,
    pub rights: AssetRights,
    pub origin: Origin,
    /// The original stand-in voice: `else VOICE`.
    pub otherwise: Option<String>,
    /// Where it is declared (the `sample` line, or the `import` line it came in by).
    pub span: Span,
    /// The snippet it came from, when it is imported.
    pub via_import: Option<String>,
    /// What the host found (None: not looked up, or not found).
    pub info: Option<AssetInfo>,
}

impl AssetUse {
    /// `harp (a local sample, sha256:9f2c…)`: for a sentence that names the asset.
    pub fn label(&self) -> String {
        let what = match (self.kind, self.origin) {
            (SampleKind::Loop, _) => "an Apple Loop",
            (_, _) => "a local sample",
        };
        let h = self.src.strip_prefix("sha256:").map_or(self.src.clone(), |h| format!("sha256:{}…", &h[..h.len().min(8)]));
        format!("`{}` ({what}, {h})", self.name)
    }
}

/// A program's effective rights, from its assets.
#[derive(Clone, Debug, PartialEq)]
pub struct RightsReport {
    pub tier: Tier,
    /// The most restrictive tag among the assets (None: it names none).
    pub license: Option<AssetRights>,
    /// The program says it is private: `use restricted` or `#! visibility: private`.
    pub declared_private: bool,
    /// The tier needs it private.
    pub private_required: bool,
    /// One sentence per asset that sets the tier, and why.
    pub why: Vec<String>,
    /// Credit lines the program must carry (Apple Loops).
    pub credits: Vec<String>,
    pub assets: Vec<AssetUse>,
}

impl RightsReport {
    pub fn none() -> RightsReport {
        RightsReport { tier: Tier::Open, license: None, declared_private: false, private_required: false, why: Vec::new(), credits: Vec::new(), assets: Vec::new() }
    }

    /// Why the program cannot be shared or made public, or None. Names the asset.
    pub fn refusal(&self) -> Option<String> {
        let a = self.assets.iter().find(|a| a.rights == AssetRights::RestrictedPrivate)?;
        Some(format!("it uses {}, which is restricted: a program that uses restricted assets is private and is never shared or made public{}", a.label(), a.otherwise.as_ref().map_or(String::new(), |v| format!(" (fork it with substitute=true to play `{v}` instead)"))))
    }

    /// Why the program cannot carry the license `l` (None: it can). A program is as restricted as its assets.
    pub fn license_refusal(&self, l: AssetRights) -> Option<String> {
        let min = self.license?;
        if l >= min {
            return None;
        }
        let a = self.assets.iter().find(|a| a.rights == min)?;
        Some(format!("it uses {}, which is {}: the program cannot be {}", a.label(), min.id(), l.id()))
    }

    pub fn to_json_pairs(&self) -> Vec<(&'static str, djengine::json::JsonValue)> {
        use djengine::json::JsonValue;
        let assets: Vec<JsonValue> = self
            .assets
            .iter()
            .map(|a| {
                let mut o = djengine::json::JsonObject::new();
                o.set("name", JsonValue::str(a.name.clone()));
                o.set("src", JsonValue::str(a.src.clone()));
                o.set("kind", JsonValue::str(if a.kind == SampleKind::Loop { "loop" } else { "sample" }));
                o.set("license", JsonValue::str(a.rights.id()));
                o.set("origin", JsonValue::str(if a.origin == Origin::AppleLoops { "apple-loops" } else { "local" }));
                o.set("else", a.otherwise.clone().map_or(JsonValue::Null, JsonValue::str));
                o.set("found", (a.info.is_some()).into());
                if let Some(i) = &a.info {
                    o.set("file", JsonValue::str(i.name.clone()));
                    if let Some(b) = i.bpm {
                        o.set("bpm", JsonValue::Number((b * 10.0).round() / 10.0));
                    }
                    if let Some(k) = &i.key {
                        o.set("key", JsonValue::str(k.clone()));
                    }
                }
                JsonValue::Object(o)
            })
            .collect();
        vec![
            ("tier", JsonValue::str(self.tier.id())),
            ("license", self.license.map_or(JsonValue::Null, |l| JsonValue::str(l.id()))),
            ("private_required", self.private_required.into()),
            ("declared_private", self.declared_private.into()),
            ("why", JsonValue::Array(self.why.iter().cloned().map(JsonValue::str).collect())),
            ("credits", JsonValue::Array(self.credits.iter().cloned().map(JsonValue::str).collect())),
            ("assets", JsonValue::Array(assets)),
        ]
    }
}

/// `#! visibility: private` on a comment line of the text.
fn header_private(parsed: &Parsed, src: &str) -> bool {
    parsed.comments.iter().any(|c| {
        let t = c.text(src);
        t.strip_prefix("#!").is_some_and(|r| {
            let r: String = r.split_whitespace().collect::<Vec<_>>().join(" ");
            r.eq_ignore_ascii_case("visibility: private")
        })
    })
}

fn decl_use(d: &SampleDecl, span: Span, via: Option<String>) -> AssetUse {
    let (rights, origin) = match d.kind {
        SampleKind::Loop => (AssetRights::AllRightsReserved, Origin::AppleLoops),
        SampleKind::Sample => (d.license.as_ref().and_then(|w| AssetRights::parse(&w.text)).unwrap_or(AssetRights::RestrictedPrivate), Origin::Local),
    };
    AssetUse { name: d.name.text.clone(), src: d.src.text.clone(), kind: d.kind, rights, origin, otherwise: d.otherwise.as_ref().map(|w| w.text.clone()), span, via_import: via, info: None }
}

/// The assets a parsed program names (its own declarations, and those of the snippets it imports), and whether it
/// says it is private. From the text alone: no file is looked at.
pub fn declared(parsed: &Parsed, src: &str) -> (Vec<AssetUse>, bool) {
    let mut uses = Vec::new();
    let mut restricted = header_private(parsed, src);
    for imp in &parsed.imports {
        for d in &imp.samples {
            uses.push(decl_use(d, imp.span, Some(imp.cid.clone())));
        }
    }
    for st in &parsed.program.stmts {
        match &st.kind {
            StmtKind::Sample(d) => uses.push(decl_use(d, st.span, None)),
            StmtKind::UseRestricted => restricted = true,
            _ => {}
        }
    }
    (uses, restricted)
}

/// The tier and the sentences, from the declared assets.
pub fn report(uses: Vec<AssetUse>, declared_private: bool) -> RightsReport {
    let license = uses.iter().map(|a| a.rights).max();
    let tier = match license {
        None | Some(AssetRights::Cc0 | AssetRights::CcBy) => Tier::Open,
        Some(AssetRights::AllRightsReserved) => {
            if uses.iter().filter(|a| a.rights == AssetRights::AllRightsReserved).all(|a| a.origin == Origin::AppleLoops) {
                Tier::AppleLoops
            } else {
                Tier::AllRightsReserved
            }
        }
        Some(AssetRights::RestrictedPrivate) => Tier::Private,
    };
    let mut why = Vec::new();
    for a in &uses {
        if Some(a.rights) == license && a.rights > AssetRights::CcBy {
            why.push(format!("{} is {}", a.label(), if a.origin == Origin::AppleLoops { "an Apple Loop: all rights reserved, with a credit".to_string() } else { a.rights.id().to_string() }));
        }
    }
    if why.is_empty() {
        why.push(match license {
            None => "it names no assets".to_string(),
            Some(l) => format!("its assets are {}", l.id()),
        });
    }
    let mut credits = Vec::new();
    if uses.iter().any(|a| a.origin == Origin::AppleLoops) {
        credits.push(APPLE_CREDIT.to_string());
    }
    RightsReport { tier, license, declared_private, private_required: tier == Tier::Private, why, credits, assets: uses }
}

/// The program's rights from its text alone (no files looked up): for a store that must decide whether a program may
/// be public without having the sounds.
pub fn rights_of_text(text: &str, env: &Env) -> RightsReport {
    let parsed = crate::parse::parse_with(text, env);
    let (uses, private) = declared(&parsed, text);
    report(uses, private || env.private)
}

/// A parsed program's assets looked up through `env.assets`: the `src` of each own declaration becomes
/// `sha256:HEX` (so the canonical text names the file by what it is, not where it is), the license tag is spelled
/// out, and a file that is not found is an error naming it. Then the rights are checked: a restricted asset needs the
/// program to say it is private.
pub fn resolve(parsed: &mut Parsed, src: &str, env: &Env, diags: &mut Vec<Diagnostic>) -> RightsReport {
    let (mut uses, declared_private) = declared(parsed, src);
    // a saved object that is private says so as well as `use restricted` does
    let declared_private = declared_private || env.private;
    // the own declarations are the last `n` uses, in statement order
    let n_own = parsed.program.stmts.iter().filter(|s| matches!(s.kind, StmtKind::Sample(_))).count();
    let skip = uses.len() - n_own;
    let mut by_name: BTreeMap<String, Span> = BTreeMap::new();
    for u in uses.iter_mut() {
        if by_name.insert(u.name.clone(), u.span).is_some() {
            diags.push(Diagnostic::error(u.span, format!("`{}` is named twice", u.name)).help("a sample or loop has one name in a program"));
        }
        if let Some(voice) = &u.otherwise {
            if let Some(cat) = env.catalog {
                let voices = cat.voices();
                if !voices.iter().any(|v| v == voice) {
                    let near = crate::env::suggest(voice, &voices, 3);
                    let mut d = Diagnostic::error(u.span, format!("`else {voice}`: not an instrument the engine has"));
                    if !near.is_empty() {
                        d = d.help(format!("did you mean {}?", near.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ")));
                    }
                    diags.push(d);
                }
            }
        }
        let Some(host) = env.assets else { continue };
        match host.resolve(&u.src, u.kind) {
            Ok(info) => {
                u.src = format!("sha256:{}", info.hash);
                u.origin = info.origin;
                u.info = Some(info);
            }
            Err(e) => {
                let mut d = Diagnostic::error(u.span, format!("{}: {e}", u.label()));
                if let Some(v) = &u.otherwise {
                    d = d.help(format!("or fork it with substitute=true to play `{v}`, which is ours"));
                }
                diags.push(d);
            }
        }
    }
    // put the hash and the tag into the canonical text
    let mut k = skip;
    for st in &mut parsed.program.stmts {
        if let StmtKind::Sample(d) = &mut st.kind {
            let u = &uses[k];
            k += 1;
            if u.info.is_some() {
                d.src.text = u.src.clone();
            }
            if d.kind == SampleKind::Sample {
                d.license = Some(crate::ast::Word { text: u.rights.id().to_string(), span: d.license.as_ref().map_or(d.name.span, |w| w.span) });
            }
        }
    }
    let rep = report(uses, declared_private);
    if rep.private_required && !rep.declared_private {
        let a = rep.assets.iter().find(|a| a.rights == AssetRights::RestrictedPrivate).expect("a restricted asset set the tier");
        let msg = match &a.via_import {
            Some(cid) => format!("it imports {}, which uses {}: the program must declare `use restricted`", djengine::rights::short_cid(cid), a.label()),
            None => format!("{} is restricted: the program must declare `use restricted` (or `#! visibility: private`) and stays private", a.label()),
        };
        diags.push(Diagnostic::error(a.span, msg).help("add `use restricted` at the top; or give the sample a license (`license cc0`, `cc-by`, `all-rights-reserved`) if you may"));
    }
    rep
}

/// The program's samples as `name -> else voice`.
pub fn voices(rep: &RightsReport) -> BTreeMap<String, Option<String>> {
    rep.assets.iter().map(|a| (a.name.clone(), a.otherwise.clone())).collect()
}

/// The same program with every restricted asset swapped for its `else` voice: the `sample` / `loop` / `use
/// restricted` lines go, and `inst=NAME` becomes `inst=VOICE`. A different text, so a different CID, with nothing
/// restricted in it. Errors name an asset that has no `else`, or one that came in by an `import` (the snippet
/// must be forked first).
pub fn substitute(text: &str, env: &Env) -> Result<String, String> {
    let parsed = crate::parse::parse_with(text, env);
    if let Some(d) = parsed.diagnostics.iter().find(|d| d.is_error()) {
        return Err(format!("the program does not read: {}", d.message));
    }
    let (uses, _) = declared(&parsed, text);
    if uses.is_empty() && !parsed.program.stmts.iter().any(|s| matches!(s.kind, StmtKind::UseRestricted)) {
        return Err("there is nothing to substitute: the program names no samples".into());
    }
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    for u in &uses {
        if u.rights < AssetRights::AllRightsReserved {
            // an open asset is not ours to remove; it stays (and so does its line)
            continue;
        }
        if let Some(cid) = &u.via_import {
            return Err(format!("it imports {}, which uses {}: fork that snippet with substitute=true first", djengine::rights::short_cid(cid), u.label()));
        }
        let Some(v) = &u.otherwise else {
            return Err(format!("{} has no `else` voice: add `else <voice>` to its line (a voice of ours: `btw instruments` lists them)", u.label()));
        };
        map.insert(u.name.clone(), v.clone());
    }
    // edits by span, from the end
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    for st in &parsed.program.stmts {
        match &st.kind {
            StmtKind::Sample(d) if map.contains_key(&d.name.text) => edits.push((st.span.start, st.span.end, String::new())),
            StmtKind::UseRestricted => edits.push((st.span.start, st.span.end, String::new())),
            _ => {}
        }
    }
    let lexed = crate::lex::lex(text);
    for (i, t) in lexed.tokens.iter().enumerate() {
        let crate::lex::Tok::Word(w) = &t.tok else { continue };
        let after_inst = i >= 2 && lexed.tokens[i - 1].tok == crate::lex::Tok::Eq && matches!(&lexed.tokens[i - 2].tok, crate::lex::Tok::Word(k) if k == "inst" || k == "sound");
        let (prefix, name) = match w.split_once(':') {
            Some((p @ ("inst" | "synth"), n)) => (format!("{p}:"), n),
            _ => (String::new(), w.as_str()),
        };
        let applies = if prefix.is_empty() { after_inst && !text[..t.span.start].ends_with("sample ") } else { true };
        if applies {
            if let Some(v) = map.get(name) {
                edits.push((t.span.start, t.span.end, format!("{prefix}{v}")));
            }
        }
    }
    edits.sort_by(|a, b| b.0.cmp(&a.0));
    let mut out = text.to_string();
    for (s, e, r) in edits {
        // a removed statement takes its newline with it
        let e = if r.is_empty() && out.as_bytes().get(e) == Some(&b'\n') { e + 1 } else { e };
        out.replace_range(s..e, &r);
    }
    Ok(out)
}

/// A program's `Program` has no assets: used where a `Program` is built without the rest of a parse.
pub fn has_assets(p: &Program) -> bool {
    p.stmts.iter().any(|s| matches!(s.kind, StmtKind::Sample(_) | StmtKind::UseRestricted))
}
