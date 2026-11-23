//! The nested parts of a recipe's expansion (roadmap 6680 item 2, docs/beatscript/ext-template.md "Nested parts").
//!
//! `template::expand` writes a template reference out as one text. Inside it, every role the reference binds was written in by
//! a binding (roles.rs): a gen call's part, a stored program's sections, a theme, notes, or another form, whose own roles are
//! bindings in turn. Each of those is a **part** of the expansion, and each part has a CID of its own:
//!
//! ```text
//! arrange from template(rondo, @shanty_drunken_sailor, episodes=template(passacaglia, ground=gen@1 dubstep seed=54876))
//!
//! episodes          template  template(passacaglia, ground=gen@1 dubstep seed=54876)   z4EBG9...  (its sections and lets)
//! episodes/ground   gen       gen@1 dubstep seed=54876                                  z4EBG9...  resolves z4EBG9... seeded@1
//! ```
//!
//! A part's text is what its binding wrote, written out all the way (its gen calls fitted and written out, the sections a stored
//! program lends written in, as the whole program's are) under the `tempo` and `key` of the program it was written into, printed
//! canonical: a program of its own, so its CID names exactly what plays there. `resolves` is the pin of its source (the program a
//! gen call prints, a stored program's CID). The same reference always gives the same parts with the same CIDs.
//!
//! Pure: no I/O and no clock (a gen call is the installed generator's, a stored program the host's text).

use crate::env::Env;
use crate::roles::{Bound, Source};

/// One part of an expansion.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    /// Where it stands: the role, after the roles of the forms it is inside (`episodes/ground`).
    pub path: String,
    /// The role it fills (the last step of the path).
    pub role: String,
    /// What it was bound to: `gen`, `stored`, `theme`, `notes` or `template` (another form).
    pub kind: &'static str,
    /// The source as the reference binds it, canonical (`gen@1 dubstep seed=54876`, `ode_to_joy`, `template(passacaglia, ...)`).
    pub call: String,
    /// What it is written out as, all the way, under the key and tempo it plays in: canonical BeatScript.
    pub text: String,
    /// The CID of [`Part::text`].
    pub cid: String,
    /// The CID its source is pinned to: the program a gen call prints, a stored program's CID. None for a theme, notes or a form.
    pub resolves: Option<String>,
    /// A gen call's generator and version (`seeded@1`).
    pub generator: Option<String>,
    /// The parts of a form bound to this role.
    pub parts: Vec<Part>,
}

/// The parts of the template reference of `src` (a program has at most one: `arrange from template(...)`), in the order the
/// form names its roles, each with its own. Empty for a program with no template, a template with no roles bound, or one whose
/// references do not expand (check says why).
pub fn parts(src: &str, env: &Env) -> Vec<Part> {
    let (r, bound) = crate::roles::recording(|| crate::template::expand(src, env));
    if !matches!(r, Some(Ok(_))) {
        return Vec::new();
    }
    let flat: Vec<Part> = bound.iter().map(|b| part(b, env)).collect();
    tree(&flat, "")
}

/// Every part of `parts` and of theirs, depth first.
pub fn flatten(parts: &[Part]) -> Vec<&Part> {
    let mut out = Vec::new();
    for p in parts {
        out.push(p);
        out.extend(flatten(&p.parts));
    }
    out
}

fn parent_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(a, _)| a)
}

/// The parts whose parent is `parent`, each with its own, in the order they were recorded.
fn tree(flat: &[Part], parent: &str) -> Vec<Part> {
    flat.iter().filter(|p| parent_of(&p.path) == parent).map(|p| Part { parts: tree(flat, &p.path), ..p.clone() }).collect()
}

/// One binding as a part: its lines under the program's key and tempo, written out and printed canonical.
fn part(b: &Bound, env: &Env) -> Part {
    let mut src = String::new();
    if let Some(t) = b.host.tempo {
        src.push_str(&format!("tempo {}\n", crate::print::num(t)));
    }
    if let Some(k) = &b.host.key {
        src.push_str(&format!("key {k}\n"));
    }
    for l in &b.lines {
        src.push_str(l);
        src.push('\n');
    }
    // its gen calls fitted to that key and tempo and written out, and the sections they and a stored program lend written in,
    // as the stages write the whole program's (compile.rs: template, seed, gen, stored)
    let body = match crate::stored::expand_text(&src, env) {
        Some(Ok((t, _, _))) => t,
        _ => src,
    };
    let text = crate::print::canonical(&crate::parse::parse_with(&body, env).program);
    let cid = crate::compile::program_cid(&text);
    let (kind, resolves, generator) = match &b.source {
        Source::Gen(_) => {
            // the program the call prints, as the binding wrote it (fitted to the key it plays in): its pin
            let lines = b.lines.join("\n");
            let toks = crate::lex::lex(&lines).tokens;
            let pin = crate::gen::runs(&toks).into_iter().find_map(|r| r.r.ok()).and_then(|c| crate::gen::resolve(&c, &b.host).ok());
            ("gen", pin.as_ref().map(|r| r.cid.clone()), pin.map(|r| r.generator))
        }
        Source::Stored { cid, .. } => ("stored", Some(cid.clone()), None),
        Source::Theme(_) => ("theme", None, None),
        Source::Notes(_) => ("notes", None, None),
        Source::Form(_) => ("template", None, None),
    };
    Part { path: b.path.clone(), role: b.role.clone(), kind, call: b.source.text(), text, cid, resolves, generator, parts: Vec::new() }
}
