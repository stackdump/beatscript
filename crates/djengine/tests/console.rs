//! `Control/ConsoleCommand.swift`: the console grammar, with `ConsoleParserTests`' literals.
//! Swift builds the tool list from `MCPBridge.catalog`; this uses a small stand-in catalog
//! with the tools and arguments those tests touch.

use djengine::console::{ConsoleCommand, ConsoleParser, Property, ToolInfo};
use djengine::json::{JsonObject, JsonValue};

fn tool(name: &str, method: &str, desc: &str, props: &[&str], required: &[&str]) -> ToolInfo {
    ToolInfo {
        name: name.into(),
        method: method.into(),
        description: desc.into(),
        properties: props
            .iter()
            .map(|p| Property {
                name: p.to_string(),
                type_: "string".into(),
                description: format!("the {p}"),
            })
            .collect(),
        required: required.iter().map(|s| s.to_string()).collect(),
    }
}

fn catalog() -> Vec<ToolInfo> {
    vec![
        tool("dj_state", "state", "Everything on the decks right now. Read before you act.", &["deck"], &[]),
        tool("dj_set", "set", "Plan, pre-roll, start and run a set of saved compositions: one tool for the whole DJ night.", &["action", "set", "count", "shape", "bpm", "keepItems", "leadBars", "mixOutBar"], &["action"]),
        tool("dj_settings", "settings", "Headphones, awake and the Inpulse output.", &["action", "pfl"], &[]),
        tool("dj_compose", "composeRoll", "Play a piano roll as a raw-nets deck.", &["deck", "tempo", "parts", "name"], &["parts"]),
        tool("dj_compositions", "compositions", "List and reload saved compositions.", &["action", "id"], &[]),
        tool("dj_inspired", "composeRoll", "An original piece in an artist's style: inspiredBy plus a brief.", &["inspiredBy", "name", "parts"], &["parts"]),
        tool("dj_generate", "generate", "Compose a track from a genre and a seed: the generator's coordinates.", &["deck", "genre", "seed", "structure", "palette"], &["deck"]),
        tool("dj_load_share", "loadShare", "Load a beats share link onto a deck.", &["deck", "link"], &["link"]),
        tool("dj_perform", "perform", "Do one thing a performer can do: a recorded action.", &["action"], &["action"]),
        tool("dj_dubstep", "dubstep", "Short.", &[], &[]),
    ]
}

fn run(line: &str) -> ConsoleCommand {
    let cat = catalog();
    ConsoleParser::new(&cat).parse(line)
}

fn tool_of(line: &str) -> Option<(String, String, JsonValue)> {
    match run(line) {
        ConsoleCommand::Tool {
            name,
            method,
            params,
        } => Some((name, method, params)),
        _ => None,
    }
}

fn error(line: &str) -> String {
    match run(line) {
        ConsoleCommand::Error(m) => m,
        _ => String::new(),
    }
}

// ConsoleParserTests.everyMCPToolIsAConsoleCommandWithItsHelp (against the stand-in catalog)
#[test]
fn every_tool_is_a_console_command_with_its_help() {
    let cat = catalog();
    let p = ConsoleParser::new(&cat);
    for t in &cat {
        assert!(!t.description.is_empty() && !t.summary().is_empty());
        assert_eq!(p.resolve(t.short_name()).unwrap().name, t.name);
        assert!(p.overview(None).contains(t.short_name()));
        assert!(p.detail(&t.name).contains(&t.name));
    }
}

// ConsoleParserTests.toolsTakeShortOrFullNamesAndNoArguments
#[test]
fn tools_take_short_or_full_names_and_no_arguments() {
    for line in ["state", "dj_state", "STATE", "  state  "] {
        let (n, m, p) = tool_of(line).unwrap();
        assert!(
            n == "dj_state" && m == "state" && p == JsonValue::Object(JsonObject::new()),
            "{line}"
        );
    }
}

// ConsoleParserTests.bareWordsFillTheUsualFirstArgumentsAndKeyValuesReadAsJSON
#[test]
fn bare_words_fill_the_usual_first_arguments_and_key_values_read_as_json() {
    let a = tool_of("set show first-night").unwrap().2;
    assert_eq!(a.get("action").unwrap().as_str(), Some("show"));
    assert_eq!(a.get("set").unwrap().as_str(), Some("first-night"));
    let b = tool_of("set action=plan set=fri count=6 shape=arc bpm=[124,150] keepItems=true")
        .unwrap()
        .2;
    assert_eq!(b.get("count").unwrap().as_f64(), Some(6.0));
    assert_eq!(b.get("shape").unwrap().as_str(), Some("arc"));
    assert_eq!(
        b.get("bpm")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .filter_map(JsonValue::as_f64)
            .collect::<Vec<_>>(),
        [124.0, 150.0]
    );
    assert_eq!(b.get("keepItems").unwrap().as_bool(), Some(true));
    let c = tool_of("generate A techno 42 structure=standard")
        .unwrap()
        .2;
    assert_eq!(c.get("deck").unwrap().as_str(), Some("A"));
    assert_eq!(c.get("genre").unwrap().as_str(), Some("techno"));
    assert_eq!(c.get("seed").unwrap().as_f64(), Some(42.0));
    assert_eq!(c.get("structure").unwrap().as_str(), Some("standard"));
    // an explicit key wins over a bare word that would have gone to the same slot
    let d = tool_of("set action=mix first-night").unwrap().2;
    assert_eq!(d.get("action").unwrap().as_str(), Some("mix"));
    assert_eq!(d.get("set").unwrap().as_str(), Some("first-night"));
}

// ConsoleParserTests.performTakesItsFieldsLooseAndWrapsThemInTheAction
#[test]
fn perform_takes_its_fields_loose_and_wraps_them_in_the_action() {
    let a = tool_of("perform op=crossfader value=0").unwrap().2;
    assert_eq!(
        a.get("action").unwrap(),
        &JsonValue::Object(JsonObject::from_pairs([
            ("op", JsonValue::str("crossfader")),
            ("value", JsonValue::Number(0.0))
        ]))
    );
    let b = tool_of("perform play A on=true").unwrap().2;
    let act = b.get("action").unwrap();
    assert_eq!(act.get("op").unwrap().as_str(), Some("play"));
    assert_eq!(act.get("deck").unwrap().as_str(), Some("A"));
    assert_eq!(act.get("on").unwrap().as_bool(), Some(true));
}

// ConsoleParserTests.aJSONObjectMergesAndQuotesGroup
#[test]
fn a_json_object_merges_and_quotes_group() {
    let a = tool_of(r#"compose {"deck":"A","tempo":126,"parts":{"kick":{"notes":[[0],[4]]}}}"#)
        .unwrap();
    assert_eq!(a.1, "composeRoll");
    assert_eq!(a.2.get("deck").unwrap().as_str(), Some("A"));
    assert_eq!(
        a.2.get("parts")
            .unwrap()
            .get("kick")
            .unwrap()
            .get("notes")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let b = tool_of(r#"load_share A "https://beats.bitwrap.io/?cid=x&z=y""#)
        .unwrap()
        .2;
    assert_eq!(
        b.get("link").unwrap().as_str(),
        Some("https://beats.bitwrap.io/?cid=x&z=y")
    );
    let c = tool_of(r#"inspired inspiredBy="Daft Punk" name='A B' parts={"kick":{"notes":[[0]]}}"#)
        .unwrap()
        .2;
    assert_eq!(c.get("inspiredBy").unwrap().as_str(), Some("Daft Punk"));
    assert_eq!(c.get("name").unwrap().as_str(), Some("A B"));
    assert!(c.get("parts").unwrap().get("kick").is_some());
}

// ConsoleParserTests.rawBuiltinsAndTheAgentCommand
#[test]
fn raw_builtins_and_the_agent_command() {
    assert_eq!(
        run("raw loadShare deck=A"),
        ConsoleCommand::Raw {
            method: "loadShare".into(),
            params: JsonValue::Object(JsonObject::from_pairs([("deck", JsonValue::str("A"))]))
        }
    );
    assert_eq!(run("help"), ConsoleCommand::Help(None));
    assert_eq!(run("help set"), ConsoleCommand::Help(Some("set".into())));
    assert_eq!(run("tools"), ConsoleCommand::Tools(None));
    assert_eq!(run("ls dub"), ConsoleCommand::Tools(Some("dub".into())));
    assert_eq!(run("clear"), ConsoleCommand::Clear);
    assert_eq!(run("history"), ConsoleCommand::History);
    assert_eq!(run(""), ConsoleCommand::Blank);
    assert_eq!(run("# note"), ConsoleCommand::Blank);
    assert_eq!(
        run("agent"),
        ConsoleCommand::Agent("status".into(), String::new())
    );
    assert_eq!(
        run("agent say more energy, darker"),
        ConsoleCommand::Agent("say".into(), "more energy, darker".into())
    );
    assert_eq!(
        run("agent budget 5"),
        ConsoleCommand::Agent("budget".into(), "5".into())
    );
}

// ConsoleParserTests.mistakesSayWhatToDo
#[test]
fn mistakes_say_what_to_do() {
    assert!(error("frobnicate").contains("unknown command"));
    assert!(error("compos").contains("ambiguous"));
    assert!(error("state now").contains("no argument"));
    assert!(error("set show {oops").contains("unbalanced"));
    assert!(error(r#"set "open"#).contains("unterminated"));
    assert!(error("raw").contains("method"));
    // "did you mean": a word that is inside names
    assert!(error("ubste").contains("did you mean dubstep"));
}

// ConsoleParserTests.completionOffersToolsThenTheToolsArguments
#[test]
fn completion_offers_tools_then_the_tools_arguments() {
    let cat = catalog();
    let p = ConsoleParser::new(&cat);
    let se = p.complete("se");
    assert!(se.contains(&"set".to_string()) && se.contains(&"settings".to_string()));
    let all = p.complete("");
    assert!(all.contains(&"help".to_string()) && all.contains(&"state".to_string()));
    let args = p.complete("set ");
    assert!(
        args.contains(&"action=".to_string())
            && args.contains(&"set=".to_string())
            && args.contains(&"leadBars=".to_string())
    );
    assert_eq!(p.complete("set lead"), ["leadBars="]);
    assert!(!p
        .complete("set action=show ")
        .contains(&"action=".to_string()));
    assert!(p.complete("set action=sh").is_empty());
}

#[test]
fn summaries_and_help_text() {
    let cat = catalog();
    let p = ConsoleParser::new(&cat);
    assert_eq!(cat[0].summary(), "Everything on the decks right now");
    assert_eq!(
        cat[9].summary(),
        "Short.",
        "a first sentence under 16 characters keeps the whole text"
    );
    let long = ToolInfo {
        description: "x".repeat(200),
        ..cat[0].clone()
    };
    assert_eq!(long.summary().chars().count(), 108);
    assert!(p.overview(Some("piano")).contains("compose"));
    let d = p.detail("set");
    assert!(
        d.contains("dj_set  (control method `set`)")
            && d.contains("(required)")
            && d.contains("bare words fill: action, set")
    );
    assert!(p.detail("nope").contains("unknown command"));
    assert!(djengine::console::USAGE.contains("perform op=crossfader value=0"));
}
