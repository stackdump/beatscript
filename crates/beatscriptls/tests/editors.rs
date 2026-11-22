//! The VS Code grammar (editors/vscode) names every role, control word and statement word the language has: a word the
//! language adds and the grammar lacks would be left uncoloured there (and in IntelliJ, which reads the same grammar).

use djengine::json::JsonValue;

#[test]
fn the_grammar_knows_every_word() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../editors/vscode/syntaxes/btw.tmLanguage.json");
    let g = JsonValue::parse(&std::fs::read_to_string(path).unwrap()).unwrap();
    let pat = |k: &str| g.get("repository").and_then(|r| r.get(k)).and_then(|p| p.get("match")).and_then(JsonValue::as_str).unwrap().to_string();
    let words = |k: &str| -> Vec<String> {
        let m = pat(k);
        let inner = &m[m.find("])(").map_or(m.find('(').unwrap(), |i| i + 2) + 1..];
        inner[..inner.find(')').unwrap()].split('|').map(|w| w.replace("\\-", "-")).collect()
    };
    let (roles, actions, kw) = (words("role"), words("action"), words("keyword"));
    for r in djlive::parse::ROLES {
        assert!(roles.iter().any(|w| w == r), "role {r}");
    }
    for a in djlive::parse::ACTION_WORDS {
        assert!(actions.iter().any(|w| w == a) || kw.iter().any(|w| w == a), "control word {a}");
    }
    // the arrange mutation operators, on a `~` / `@` line
    let m = g.get("repository").and_then(|r| r.get("mutation")).and_then(|m| m.get("patterns")).and_then(JsonValue::as_array).and_then(|p| p.iter().find_map(|x| x.get("match").and_then(JsonValue::as_str))).expect("a mutation rule").to_string();
    let inner = &m[m.find('(').unwrap() + 1..];
    let ops: Vec<&str> = inner[inner.find('(').unwrap() + 1..].split(')').next().unwrap().split('|').collect();
    let mut want = djlive::mutate::operator_names();
    want.sort();
    let mut got = ops.clone();
    got.sort();
    assert_eq!(got, want, "the grammar's mutation operators are djlive's");
    assert!(kw.iter().any(|w| w == "start"), "`start` (named arrangements)");
    for s in djlive::parse::STATEMENT_WORDS {
        assert!(kw.iter().any(|w| w == s) || ["A", "B"].contains(&s), "statement word {s}");
    }
}
