//! Forms: arrange shapes from classical music (docs/beatscript/forms.md). The forms are templates (`djlive::template::TEMPLATES`,
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
        // pinned in the BeatScript repo (examples/beatscript/, with its linked data beside it); not yet one of the site's examples
        j["reference"] = json!({"name": name, "cid": cid, "file": format!("examples/beatscript/{name}.beatscript"), "jsonld": format!("examples/beatscript/{name}.jsonld")});
    }
    j["roles"] = roles_json(t);
    j["params"] = params_json(t);
    j
}

/// A form's roles as a tool lists them (docs/beatscript/ext-roles.md, forms.md "Extensions"): each role's name, what it is, what fills
/// it unbound, the let it is written as, and for a seed role its parts, each bound alone as `ROLE.PART`.
pub fn roles_json(t: &Template) -> Value {
    Value::Array(
        djlive::roles::specs(t.name)
            .iter()
            .map(|r| {
                let mut o = json!({"name": r.name, "what": r.what, "default": r.default.replace('`', ""), "part": r.part});
                if r.parts.is_empty() {
                    o["written_as"] = json!(r.let_name);
                } else {
                    o["parts"] = json!(r.parts.iter().map(|p| json!({"bind": format!("{}.{}", r.name, p.0), "part": p.0, "written_as": p.2})).collect::<Vec<_>>());
                }
                if let Some(s) = r.section {
                    o["sections"] = json!(format!("bound to sections (a seed, a stored program or set, another form or one section of it), they stand in for `{s}`"));
                }
                o
            })
            .collect(),
    )
}

/// The numbers a form takes after its roles (the canon's `delay=` and `interval=`): name, what, default, range, an example.
pub fn params_json(t: &Template) -> Value {
    Value::Array(djlive::roles::params(t.name).iter().map(|p| json!({"name": p.name, "what": p.what, "default": p.default, "min": p.min, "max": p.max, "example": p.example})).collect())
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
        // the six classical forms and cross (6673)
        assert_eq!(names().len(), 7);
        let (t, f) = find("rondo").unwrap();
        let j = form_json("https://beatscript.app/", t, f);
        assert_eq!(j["reference"]["cid"], "z4EBG9izzm3asNWWapjXtPFSZoPzgyj4cKjPELrw4jvrhdjPxrW");
        assert_eq!(default_phrase(t), "@shanty_drunken_sailor");
        assert!(find("canon").map(|(t, f)| form_json("x", t, f)["reference"].is_null()).unwrap());
    }

    /// The listing names every role (the quodlibet's too), a seed role's parts as `a.bass`, and the canon's numbers.
    #[test]
    fn a_form_lists_its_roles_and_numbers() {
        let names = |form: &str| -> Vec<String> {
            let (t, f) = find(form).unwrap();
            form_json("x", t, f)["roles"].as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap().to_string()).collect()
        };
        assert_eq!(names("quodlibet"), ["a", "b", "ground", "third"]);
        assert_eq!(names("canon"), ["subject", "ground"]);
        let (t, f) = find("cross").unwrap();
        let j = form_json("x", t, f);
        assert_eq!(j["roles"][0]["parts"][1]["bind"], "a.bass");
        assert_eq!(j["params"], json!([]));
        let (t, f) = find("canon").unwrap();
        let p = &form_json("x", t, f)["params"];
        assert_eq!((p[0]["name"].as_str(), p[1]["name"].as_str()), (Some("delay"), Some("interval")));
        assert_eq!((p[1]["min"].as_i64(), p[1]["max"].as_i64()), (Some(-24), Some(24)));
        let (t, f) = find("rondo").unwrap();
        assert!(form_json("x", t, f)["roles"][1]["sections"].as_str().unwrap().contains("stand in for `episode`"));
    }
}
