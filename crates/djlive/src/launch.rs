//! Scenes and launch (docs/BEATSCRIPT.md, "Scenes and launch"): `launch drop on pad(2) next phrase 16`.
//!
//! A **scene** is a section (it loops until the next launch) or a named arrangement (it plays as `start NAME` would, with its own `then`,
//! `loop` and `end`). A **launch** says which pad brings which scene in, and on which bar line: the next phrase line by default (8 bars,
//! counted from the song's first bar), or `next phrase 16` / `32` / `next bar`. A hit of the pad queues the scene, the last hit before the
//! decision wins, and the scene lands on that bar line as a lap that begins at the scene instead of at the song's loop point
//! ([`crate::runtime`]). A page, the phone and any host with no controller never see a hit: the song plays as written.
//!
//! Nothing about a launch is in the written song's nets, controls or checks: the launch lines are text (so they are in the CID), and the
//! scene each names is compiled once more, on its own, as the program `NAME: NAME loop` (a section) or `start NAME` (an arrangement):
//! [`scene_program`] makes that program from the parsed one, before it is expanded, so every span in it is still the text's.

use std::collections::{BTreeMap, BTreeSet};

use crate::algebra::SExpr;
use crate::ast::{ArrDecl, ArrEnd, Program, Stmt, StmtKind, Word};
use crate::compile::Compiled;
use crate::diag::{Diagnostic, Span};
use crate::expand::{Lower, SectionInfo};

/// The name the arrangement made for a section scene has. A name no program can write (the lexer reads no word that starts with a NUL
/// byte), so it cannot meet one of the program's own.
const SCENE_NAME: &str = "\u{0}scene";

thread_local! {
    /// The scene whose program is being compiled, while it is (check compiles each scene a launch names, on its own).
    static SCENE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// While it lives, the program being read is the scene `name` on its own (see [`current_scene`]).
pub(crate) struct SceneScope(Option<String>);

impl Drop for SceneScope {
    fn drop(&mut self) {
        SCENE.with(|s| *s.borrow_mut() = self.0.take());
    }
}

pub(crate) fn enter_scene(name: &str) -> SceneScope {
    SceneScope(SCENE.with(|s| s.borrow_mut().replace(name.to_string())))
}

/// The scene the program being read is, when it is one: a line that counts from a section the scene does not have does not fire there.
pub(crate) fn current_scene() -> Option<String> {
    SCENE.with(|s| s.borrow().clone())
}

/// What a launch brings in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneKind {
    /// A `section`: it plays on its own and loops.
    Section,
    /// A named arrangement: it plays from its first section, and ends as it says.
    Arrangement,
}

impl SceneKind {
    pub fn name(self) -> &'static str {
        match self {
            SceneKind::Section => "section",
            SceneKind::Arrangement => "arrangement",
        }
    }
}

/// One `launch` of the program, checked.
#[derive(Clone, Debug, PartialEq)]
pub struct Launch {
    pub scene: String,
    pub kind: SceneKind,
    /// The pad (1 to 128) whose hit queues the scene.
    pub pad: u32,
    /// The bar line it lands on: every this many bars, counted from the song's first bar (1: the next bar line).
    pub quantum: u32,
    /// The statement, and the scene's name in it.
    pub span: Span,
    pub scene_span: Span,
}

impl Launch {
    /// `pad(2)`.
    pub fn pad_text(&self) -> String {
        format!("pad({})", self.pad)
    }
}

/// The launches of a program, and each scene compiled on its own (the program with the song replaced by the scene).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Launches {
    pub table: Vec<Launch>,
    /// Scene name to its program: the same text and CID as the written one, whose timeline is the scene.
    pub scenes: BTreeMap<String, Box<Compiled>>,
}

impl Launches {
    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    /// The launch a pad starts, if any.
    pub fn for_pad(&self, pad: u32) -> Option<&Launch> {
        self.table.iter().find(|l| l.pad == pad)
    }

    /// The scene `name` as a program of its own.
    pub fn scene(&self, name: &str) -> Option<&Compiled> {
        self.scenes.get(name).map(|b| &**b)
    }

    /// The scenes' names, each once, in the order the launches say them.
    pub fn scene_names(&self) -> Vec<&str> {
        let mut seen = BTreeSet::new();
        self.table.iter().filter(|l| seen.insert(l.scene.as_str())).map(|l| l.scene.as_str()).collect()
    }

    /// What `check` says of them: `launch: pad(1) verse, pad(2) drop, pad(3) breakdown (read where a controller has the pad; elsewhere the song plays as written)`.
    pub fn info(&self) -> String {
        let list = self.table.iter().map(|l| format!("{} {}", l.pad_text(), l.scene)).collect::<Vec<_>>().join(", ");
        format!("launch: {list} (a hit of the pad brings the scene in on its bar line; with no controller the song plays as written)")
    }

    /// The `launches` of `check`'s report: the scene, what it is, the pad and the bars of its line.
    pub fn to_json(&self) -> djengine::json::JsonValue {
        use djengine::json::{JsonObject, JsonValue};
        JsonValue::Array(
            self.table
                .iter()
                .map(|l| {
                    JsonValue::Object(JsonObject::from_pairs([
                        ("scene", JsonValue::str(l.scene.clone())),
                        ("kind", JsonValue::str(l.kind.name())),
                        ("pad", JsonValue::int(i64::from(l.pad))),
                        ("quantum", JsonValue::int(i64::from(l.quantum))),
                    ]))
                })
                .collect(),
        )
    }
}

/// The scene names every `launch` of the program names, valid or not: what the checks that say "never plays" leave alone.
pub fn targets(program: &Program) -> BTreeSet<String> {
    program.stmts.iter().filter_map(|s| if let StmtKind::Launch(l) = &s.kind { Some(l.scene.text.clone()) } else { None }).collect()
}

impl Lower<'_> {
    /// Check every `launch` against the sections and the named arrangements the program has: each names one, each pad launches one
    /// scene, and there is a song to launch into. The checked launches, in the order written.
    pub(crate) fn launches(&mut self, program: &Program, sections: &BTreeMap<String, SectionInfo>, arrs: &[(&ArrDecl, Span)], compositions: &BTreeSet<String>) -> Vec<Launch> {
        let mut out: Vec<Launch> = Vec::new();
        let mut seen_pad: BTreeMap<u32, (String, Span)> = BTreeMap::new();
        for st in &program.stmts {
            let StmtKind::Launch(l) = &st.kind else { continue };
            if sections.is_empty() && arrs.is_empty() {
                self.err(Diagnostic::error(st.span, "`launch` needs a song to launch into").help("a scene is a section: write `section drop 8 bars { ... }` and `arrange verse drop`"));
                continue;
            }
            let name = &l.scene.text;
            let kind = if sections.contains_key(name) {
                Some(SceneKind::Section)
            } else if arrs.iter().any(|(d, _)| d.name.text == *name) {
                Some(SceneKind::Arrangement)
            } else {
                None
            };
            let Some(kind) = kind else {
                let mut all: Vec<String> = sections.keys().cloned().collect();
                all.extend(arrs.iter().map(|(d, _)| d.name.text.clone()));
                let mut d = Diagnostic::error(l.scene.span, format!("`launch {name}`: there is no section or arrangement called `{name}`")).found(format!("`{name}`"));
                if compositions.contains(name) {
                    d = d.help(format!("`{name}` is a composition (a `let`), not a section: launch one of its sections"));
                } else {
                    let near = crate::env::suggest(name, &all, 1);
                    d = d.help(match near.first() {
                        Some(n) => format!("a scene is a section or a named arrangement: did you mean `{n}`?"),
                        None => format!("a scene is a section or a named arrangement; this program has {}", all.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ")),
                    });
                }
                self.err(d);
                continue;
            };
            if let Some((other, at)) = seen_pad.get(&l.pad) {
                let line = crate::diag::line_col(self.src, at.start).0;
                self.err(Diagnostic::error(l.pad_span, format!("pad({}) already launches `{other}` (line {line})", l.pad)).help("one pad launches one scene: use another pad"));
                continue;
            }
            seen_pad.insert(l.pad, (name.clone(), st.span));
            out.push(Launch { scene: name.clone(), kind, pad: l.pad, quantum: l.quantum.bars(), span: st.span, scene_span: l.scene.span });
        }
        out
    }
}

/// The program as it is when the scene is all there is: every `launch` left out, and the song's own ending (`arrange`, `start`, `end`,
/// `loop`) replaced by the scene's. A section is the named arrangement `NAME: NAME loop`; an arrangement is `start NAME`, with the
/// program's arrangements as they were. The statements keep their spans, so the scene's compile points at the program's text.
pub fn scene_program(program: &mut Program, launch: &Launch) {
    program.stmts.retain(|s| match s.kind {
        StmtKind::Launch(_) | StmtKind::Arrange(_) | StmtKind::ArrangeExpr(_) | StmtKind::End(_) | StmtKind::Loop | StmtKind::Start(_) => false,
        StmtKind::ArrangeNamed(_) => launch.kind == SceneKind::Arrangement,
        _ => true,
    });
    let word = |text: &str| Word { text: text.to_string(), span: launch.scene_span };
    let kind = match launch.kind {
        SceneKind::Section => StmtKind::ArrangeNamed(ArrDecl { name: word(SCENE_NAME), items: vec![SExpr::Name(word(&launch.scene))], ends: ArrEnd::Loop }),
        SceneKind::Arrangement => StmtKind::Start(word(&launch.scene)),
    };
    program.stmts.push(Stmt { span: launch.span, kind });
}
