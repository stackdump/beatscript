//! Forms: arrange shapes from classical music (docs/btw/forms.md). The forms are templates (`djlive::template::TEMPLATES`,
//! each with its [`djlive::template::Form`]): this module is what the MCP tools (`genre_template` with `form`) and the docs
//! read of them, so that the page, the tools and the ontology say one thing.

use djlive::template::{forms, Form, Template};
use serde_json::{json, Value};

/// The canonical doc's path on the site.
pub const DOC_PATH: &str = "/docs/forms";

/// A form as a tool answers it: what it is, its sections and its mutation plan, the themes it is built from, a reference
/// to write (`arrange from template(...)`), and the hand-written track in it when there is one.
pub fn form_json(origin: &str, t: &Template, f: &Form) -> Value {
    let o = origin.trim_end_matches('/');
    let mut j = json!({
        "name": t.name, "label": f.label, "summary": t.summary, "shape": f.shape, "plan": f.plan,
        "mutations": f.mutations, "themes": f.themes, "example": t.example,
        "docs": format!("{o}{DOC_PATH}.md#{}", t.name),
    });
    if let Some((name, cid)) = f.reference {
        // pinned in the BeatScript repo (examples/btw/, with its linked data beside it); not yet one of the site's examples
        j["reference"] = json!({"name": name, "cid": cid, "file": format!("examples/btw/{name}.btw"), "jsonld": format!("examples/btw/{name}.jsonld")});
    }
    j
}

/// The form called `name` (its template's name, or its label: `theme and variations`). Err: no such form, with the names.
pub fn find(name: &str) -> Result<(&'static Template, &'static Form), String> {
    let n = name.trim().to_ascii_lowercase().replace([' ', '_'], "-");
    forms().find(|(t, f)| t.name == n || f.label.replace(' ', "-") == n).ok_or_else(|| format!("no form {name:?}; the forms are {}", names().join(", ")))
}

/// The forms' names, in the docs' order.
pub fn names() -> Vec<&'static str> {
    forms().map(|(t, _)| t.name).collect()
}

/// The phrase of a form's example (`@psalm42_deep_calleth`): what `genre_template` sings when no phrase is given.
pub fn default_phrase(t: &Template) -> &'static str {
    t.example.split_once(", ").map_or("", |(_, p)| p.trim_end_matches(')'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_form_is_found_by_name_or_label() {
        assert_eq!(find("quodlibet").unwrap().0.name, "quodlibet");
        assert_eq!(find("Theme and Variations").unwrap().0.name, "theme-and-variations");
        assert!(find("sonata").err().unwrap().contains("passacaglia"));
        assert_eq!(names().len(), 6);
        let (t, f) = find("rondo").unwrap();
        let j = form_json("https://beatscript.app/", t, f);
        assert_eq!(j["reference"]["cid"], "z4EBG9izUsbisU2JZtPHUo4hcQpYd8FLyZXLqKUmbtftjj3Lirw");
        assert_eq!(default_phrase(t), "@shanty_drunken_sailor");
        assert!(find("canon").map(|(t, f)| form_json("x", t, f)["reference"].is_null()).unwrap());
    }
}
