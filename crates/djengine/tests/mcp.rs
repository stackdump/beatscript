//! `Control/MCPBridge.swift`, `Control/AgentStream.swift` and the catalog/console tie
//! (`ConsoleParserTests.everyMCPToolIsAConsoleCommandWithItsHelp`).
//!
//! No Swift on this box: the catalog text is extracted from the Swift source itself
//! (`tools/mcp-catalog/extract.py`), and every assert below is a literal from the matching Swift
//! test (named in the comment above it), run against the real catalog.

use std::cell::RefCell;

use djengine::agent::{self, AgentEvent, Launch};
use djengine::console::{ConsoleCommand, ConsoleParser};
use djengine::fx::master_macro::MasterFxKey;
use djengine::fx::unit::FxParam;
use djengine::json::{JsonObject, JsonValue};
use djengine::mcp::{self, Catalog, McpBridge};
use djengine::session::action::{DeckAction, DEFAULT_MIX_IN_BARS, JSON_OPS, MIX_OUT_DEFAULT_BARS};
use djengine::wander::WanderLock;
use sha2::{Digest, Sha256};

fn parse(s: &str) -> JsonValue {
    JsonValue::parse(s).unwrap()
}

type Calls = RefCell<Vec<(String, JsonValue)>>;

/// MCPBridgeTests.bridge: records calls; `perform` fails with "no deck C".
fn ask(seen: &Calls, line: &str) -> JsonValue {
    let bridge = McpBridge::new(mcp::DEV_VERSION);
    let mut call = |m: &str, p: &JsonValue| {
        seen.borrow_mut().push((m.to_string(), p.clone()));
        if m == "perform" {
            Err("no deck C".to_string())
        } else {
            Ok(parse(r#"{"ok":true}"#))
        }
    };
    parse(&bridge.handle(line, &mut call).expect("a reply"))
}

fn tools_list() -> Vec<JsonValue> {
    let r = ask(
        &Calls::default(),
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
    );
    r.get("result")
        .unwrap()
        .get("tools")
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}

fn description(tool: &str) -> String {
    tools_list()
        .iter()
        .find(|t| t.get("name").and_then(JsonValue::as_str) == Some(tool))
        .and_then(|t| t.get("description")?.as_str().map(String::from))
        .unwrap_or_default()
}

#[test]
fn the_catalog_file_is_the_swift_source_at_the_recorded_commit() {
    // Re-run tools/mcp-catalog/extract.py after the Swift catalog changes; a stale copy is a diff.
    let digest = Sha256::digest(mcp::CATALOG_JSON.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        hex,
        "2deec4e027546f4d38ac6983cc1c03f13508c42d5c62587a2c89eefc7fb88b8c"
    );
    let c = Catalog::shared();
    assert_eq!(c.commit, "c4946e49cfc5fdbb92cf974a17aba96e8ca9f03f");
    assert_eq!(c.source, "Sources/DJCore/Control/MCPBridge.swift");
    assert_eq!(c.tools.len(), 27);
    assert_eq!(c.methods.len(), 27);
}

// MCPBridgeTests.initializeEchoesTheProtocolAndOffersTools
#[test]
fn initialize_echoes_the_protocol_and_offers_tools() {
    let r = ask(
        &Calls::default(),
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#,
    );
    assert_eq!(r.get("id").unwrap().as_i64(), Some(1));
    let res = r.get("result").unwrap();
    assert_eq!(
        res.get("protocolVersion").unwrap().as_str(),
        Some("2025-03-26")
    );
    assert!(res.get("capabilities").unwrap().get("tools").is_some());
    let info = res.get("serverInfo").unwrap();
    assert_eq!(info.get("name").unwrap().as_str(), Some("dj-vevr-se"));
    assert_eq!(info.get("version").unwrap().as_str(), Some("dev"));
    assert!(res
        .get("instructions")
        .unwrap()
        .as_str()
        .unwrap()
        .starts_with("Drives the running DJApp (dj-vevr-se) on this Mac"));
    // no protocolVersion asked: the default
    let r = ask(
        &Calls::default(),
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
    );
    assert_eq!(
        r.get("result")
            .unwrap()
            .get("protocolVersion")
            .unwrap()
            .as_str(),
        Some("2025-06-18")
    );
}

// MCPBridgeTests.notificationsGetNoReply
#[test]
fn notifications_get_no_reply() {
    let bridge = McpBridge::new("dev");
    let mut call = |_: &str, _: &JsonValue| Ok(JsonValue::Null);
    assert!(bridge
        .handle(
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            &mut call
        )
        .is_none());
    // a null id is a notification too
    assert!(bridge
        .handle(r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#, &mut call)
        .is_none());
    // garbage is a parse error, with a null id
    let r = parse(&bridge.handle("not json", &mut call).unwrap());
    assert_eq!(
        r.get("error").unwrap().get("code").unwrap().as_i64(),
        Some(-32700)
    );
    assert_eq!(r.get("id"), Some(&JsonValue::Null));
}

// MCPBridgeTests.everyToolHasASchemaAndAMethod
#[test]
fn every_tool_has_a_schema_and_a_method() {
    let tools = tools_list();
    let cat = Catalog::shared();
    assert_eq!(tools.len(), cat.methods.len());
    for t in &tools {
        let name = t.get("name").unwrap().as_str().unwrap();
        assert!(cat.method(name).is_some(), "{name} routes somewhere");
        assert_eq!(
            t.get("inputSchema").unwrap().get("type").unwrap().as_str(),
            Some("object")
        );
        assert!(!t.get("description").unwrap().as_str().unwrap().is_empty());
    }
}

// MCPBridgeTests.aToolCallBecomesOneControlRequest
#[test]
fn a_tool_call_becomes_one_control_request() {
    let seen = Calls::default();
    let r = ask(
        &seen,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"dj_generate","arguments":{"deck":"A","genre":"techno","seed":7}}}"#,
    );
    let seen = seen.borrow();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, "generate");
    assert_eq!(seen[0].1.get("genre").unwrap().as_str(), Some("techno"));
    let res = r.get("result").unwrap();
    assert_eq!(res.get("isError").unwrap().as_bool(), Some(false));
    assert_eq!(
        res.get("content")
            .unwrap()
            .at(0)
            .unwrap()
            .get("text")
            .unwrap()
            .as_str(),
        Some(r#"{"ok":true}"#)
    );
}

// MCPBridgeTests.loadShareIsOneControlRequest
#[test]
fn load_share_is_one_control_request() {
    let seen = Calls::default();
    let link = "https://beats.bitwrap.io/?g=house&s=7";
    let r = ask(
        &seen,
        &format!(
            r#"{{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{{"name":"dj_load_share","arguments":{{"deck":"B","link":"{link}"}}}}}}"#
        ),
    );
    {
        let seen = seen.borrow();
        assert!(seen.len() == 1 && seen[0].0 == "loadShare");
        assert_eq!(seen[0].1.get("link").unwrap().as_str(), Some(link));
        assert_eq!(seen[0].1.get("deck").unwrap().as_str(), Some("B"));
    }
    assert_eq!(
        r.get("result").unwrap().get("isError").unwrap().as_bool(),
        Some(false)
    );
    let tool = tools_list()
        .into_iter()
        .find(|t| t.get("name").unwrap().as_str() == Some("dj_load_share"))
        .unwrap();
    assert_eq!(
        tool.get("inputSchema").unwrap().get("required").unwrap(),
        &JsonValue::Array(vec!["deck".into(), "link".into()])
    );
    assert!(tool
        .get("description")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("raw-nets"));
}

// MCPBridgeTests.appErrorsAreToolErrorsNotProtocolErrors
#[test]
fn app_errors_are_tool_errors_not_protocol_errors() {
    let r = ask(
        &Calls::default(),
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"dj_perform","arguments":{}}}"#,
    );
    assert!(r.get("error").is_none());
    let res = r.get("result").unwrap();
    assert_eq!(res.get("isError").unwrap().as_bool(), Some(true));
    assert_eq!(
        res.get("content")
            .unwrap()
            .at(0)
            .unwrap()
            .get("text")
            .unwrap()
            .as_str(),
        Some("no deck C")
    );
}

// MCPBridgeTests.unknownThingsAreProtocolErrors (+ ping)
#[test]
fn unknown_things_are_protocol_errors() {
    let code = |line: &str| {
        ask(&Calls::default(), line)
            .get("error")
            .and_then(|e| e.get("code")?.as_i64())
    };
    assert_eq!(
        code(r#"{"jsonrpc":"2.0","id":5,"method":"resources/list"}"#),
        Some(-32601)
    );
    assert_eq!(
        code(r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"dj_nope"}}"#),
        Some(-32602)
    );
    let ping = ask(
        &Calls::default(),
        r#"{"jsonrpc":"2.0","id":9,"method":"ping"}"#,
    );
    assert_eq!(ping.get("result").unwrap(), &parse("{}"));
}

// A screenshot comes back as the image itself (the hook reads the file the app wrote).
#[test]
fn a_screenshot_comes_back_as_the_image() {
    let read = |p: &str| (p == "/tmp/shot.png").then(|| b"\x89PNG\r\n".to_vec());
    let mut bridge = McpBridge::new("dev");
    bridge.read_file = Some(&read);
    let mut call = |_: &str, _: &JsonValue| Ok(parse(r#"{"path":"/tmp/shot.png"}"#));
    let r = parse(
        &bridge
            .handle(
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"dj_screenshot"}}"#,
                &mut call,
            )
            .unwrap(),
    );
    let content = r.get("result").unwrap().get("content").unwrap();
    assert_eq!(
        content.at(0).unwrap().get("type").unwrap().as_str(),
        Some("image")
    );
    assert_eq!(
        content.at(0).unwrap().get("data").unwrap().as_str(),
        Some("iVBORw0K")
    );
    assert_eq!(
        content.at(0).unwrap().get("mimeType").unwrap().as_str(),
        Some("image/png")
    );
    assert_eq!(
        content.at(1).unwrap().get("type").unwrap().as_str(),
        Some("text")
    );
    // base64 padding
    assert_eq!(mcp::base64_std(b"a"), "YQ==");
    assert_eq!(mcp::base64_std(b"ab"), "YWI=");
    assert_eq!(mcp::base64_std(b"abc"), "YWJj");
    assert_eq!(mcp::base64_std(b""), "");
}

// MCPBridgeTests.everyPerformOpIsDocumented
#[test]
fn every_perform_op_is_documented() {
    let text = description("dj_perform");
    let needs = |op: &str| -> &'static str {
        match op {
            "queueNext" => r#","share":{"genre":"techno","seed":1}"#,
            "voiceTweak" | "voiceSynth" => r#","channel":0"#,
            "clipMove" | "clipFx" | "clipImport" => r#","id":"x""#,
            "lock" => r#","key":"A.play""#,
            "wanderPace" => r#","bars":8"#,
            "wanderLock" => r#","dimension":"tempo""#,
            "setRoleVoice" => r#","role":"bass""#,
            "masterFx" => r#","key":"lp-freq""#,
            "fx" => r#","param":"filter","value":0.5"#,
            "autoMixDefer" => r#","handoff":0"#,
            _ => "",
        }
    };
    let mut seen = std::collections::BTreeSet::new();
    for op in JSON_OPS {
        assert!(seen.insert(op), "{op} listed twice");
    }
    for op in JSON_OPS.iter().filter(|o| **o != "load") {
        let v = parse(&format!(r#"{{"op":"{op}","deck":"A"{}}}"#, needs(op)));
        let a = DeckAction::from_json(&v);
        assert!(a.is_some(), "{op} decodes");
        assert_eq!(
            a.unwrap().json().get("op").and_then(JsonValue::as_str),
            Some(*op),
            "{op} encodes back as itself"
        );
        assert!(
            text.contains(&format!("{op} {{"))
                || text.contains(&format!("{op}/"))
                || text.contains(&format!("/{op} {{")),
            "dj_perform's description documents {op}"
        );
    }
    let dims = WanderLock::DIMENSIONS.join("|");
    assert!(
        text.contains(&format!("dimension: {dims}|keep:<role>")),
        "wanderLock's dimensions are listed"
    );
}

// MCPBridgeTests.clipRetuneTakesVoicesByChannel
#[test]
fn clip_retune_takes_voices_by_channel() {
    let v =
        parse(r#"{"op":"clipRetune","id":"x","voices":{"3":"saw","bass":"piano","1":"organ"}}"#);
    let a = DeckAction::from_json(&v).expect("decodes");
    let j = a.json();
    let voices = j.get("voices").unwrap().as_object().unwrap();
    assert_eq!(voices.len(), 2, "non-numeric channels are dropped");
    assert_eq!(voices.get("3").unwrap().as_str(), Some("saw"));
    assert_eq!(voices.get("1").unwrap().as_str(), Some("organ"));
    assert!(description("dj_perform")
        .contains(r#"clipRetune {id, voices: {"<MIDI channel>": instrument}}"#));
}

// MCPBridgeTests.mixOutAndMixInAreDocumented
#[test]
fn mix_out_and_mix_in_are_documented() {
    let text = description("dj_perform");
    assert!(text.contains(&format!(
        "mixOut {{deck, bars? 1-64, default {MIX_OUT_DEFAULT_BARS}}}"
    )));
    assert!(text.contains(&format!(
        "mixIn {{deck, bars? 1-64, default {DEFAULT_MIX_IN_BARS}}}"
    )));
    assert!(text.contains("stinger, mixOut, mixIn) need a deck in net mode"));
    assert!(text.contains("master-chain macros and panic also work on a sample deck"));
    let mix = JSON_OPS.iter().position(|o| *o == "mixOut").unwrap();
    assert_eq!(JSON_OPS[mix + 1], "mixIn");
}

// MCPBridgeTests.theControllerOnlyOpsAreDocumented (the master FX keys and the FX unit's
// params are the one interpolated bit of text: the extractor reads Swift's FxParam, this
// checks the Rust enum agrees)
#[test]
fn the_controller_only_ops_are_documented() {
    let text = description("dj_perform");
    assert!(text.contains("rewind {deck}"));
    assert!(text.contains("focus {deck: A|B|L}"));
    assert!(text.contains("stemMute {deck, stem: music|mic, on}"));
    let keys: Vec<&str> = MasterFxKey::ALL.iter().map(|k| k.id()).collect();
    assert!(text.contains(&format!(
        "masterFx {{deck, key: {}, value 0…1}}",
        keys.join("|")
    )));
    let params: Vec<&str> = FxParam::ALL.iter().map(|p| p.id()).collect();
    assert!(
        text.contains(&format!("fx {{deck, param: {}, value}}", params.join("|"))),
        "FxParam ids"
    );
}

// MCPBridgeTests.importingAStockLoopOntoTheRowIsDocumented + dJPerformDocumentsAClipsOwnFollow
#[test]
fn stock_loops_and_clip_follow_are_documented() {
    let perform = description("dj_perform");
    assert!(perform.contains("clipImport {id, title, bpm, beats, provenance?}"));
    assert!(perform.contains("clipFollow {id, mode: louder|source or null}"));
    assert!(JSON_OPS.contains(&"clipFollow"));
    let stock = description("dj_stock_loops");
    assert!(stock.contains("addToRow {path}") && stock.contains("never published"));
    let tool = tools_list()
        .into_iter()
        .find(|t| t.get("name").unwrap().as_str() == Some("dj_stock_loops"))
        .unwrap();
    let action = tool.get("inputSchema").and_then(|s| {
        s.get("properties")?
            .get("action")?
            .get("description")?
            .as_str()
            .map(String::from)
    });
    assert_eq!(action.as_deref(), Some("list | load | addToRow"));
}

// dj_generate's properties are sorted by name (Swift sorts the dictionary), required kept.
#[test]
fn tool_schemas_sort_properties_and_keep_required() {
    let t = tools_list()
        .into_iter()
        .find(|t| t.get("name").unwrap().as_str() == Some("dj_generate"))
        .unwrap();
    let schema = t.get("inputSchema").unwrap();
    let keys: Vec<&str> = schema
        .get("properties")
        .unwrap()
        .as_object()
        .unwrap()
        .keys()
        .collect();
    assert_eq!(
        keys,
        ["deck", "genre", "options", "palette", "seed", "structure"]
    );
    assert_eq!(
        schema.get("required").unwrap(),
        &JsonValue::Array(vec!["deck".into(), "genre".into(), "seed".into()])
    );
    // a tool with no properties still lists an (empty) properties object
    let state = tools_list()
        .into_iter()
        .find(|t| t.get("name").unwrap().as_str() == Some("dj_state"))
        .unwrap();
    assert_eq!(
        state.get("inputSchema").unwrap(),
        &parse(r#"{"type":"object","properties":{}}"#)
    );
}

// ConsoleParserTests.everyMCPToolIsAConsoleCommandWithItsHelp, against the real catalog
#[test]
fn every_mcp_tool_is_a_console_command_with_its_help() {
    let cat = mcp::catalog();
    let c = Catalog::shared();
    assert!(cat.len() == c.methods.len() && cat.len() >= 25);
    let p = ConsoleParser::new(&cat);
    for t in &cat {
        assert_eq!(c.method(&t.name), Some(t.method.as_str()));
        assert!(!t.description.is_empty() && !t.summary().is_empty());
        assert_eq!(
            p.resolve(t.short_name())
                .unwrap_or_else(|e| panic!("{}: {}", t.short_name(), e.0))
                .name,
            t.name
        );
        assert!(p.overview(None).contains(t.short_name()));
        assert!(p.detail(&t.name).contains(&t.name));
    }
    // the same lines the console tests use, now on the real tool list
    match p.parse("set show first-night") {
        ConsoleCommand::Tool {
            name,
            method,
            params,
        } => {
            assert_eq!((name.as_str(), method.as_str()), ("dj_set", "set"));
            assert_eq!(params.get("set").unwrap().as_str(), Some("first-night"));
        }
        other => panic!("{other:?}"),
    }
    match p.parse("inspired inspiredBy=\"Daft Punk\"") {
        ConsoleCommand::Tool { method, .. } => assert_eq!(method, "composeRoll"),
        other => panic!("{other:?}"),
    }
    // completion offers real argument names
    let args = p.complete("set ");
    assert!(args.contains(&"action=".to_string()) && args.contains(&"leadBars=".to_string()));
    assert_eq!(p.complete("set lead"), ["leadBars="]);
}

// ---- AgentStreamTests -------------------------------------------------------------------

// theInitLineIsTheAgentBeingReady
#[test]
fn the_init_line_is_the_agent_being_ready() {
    let e = agent::parse(
        r#"{"type":"system","subtype":"init","session_id":"abc-123","model":"claude-x","tools":[]}"#,
    );
    assert_eq!(
        e,
        [AgentEvent::Ready {
            session: "abc-123".into(),
            model: Some("claude-x".into())
        }]
    );
    assert!(agent::parse(r#"{"type":"system","subtype":"status"}"#).is_empty());
}

// assistantTextAndToolCallsBecomeConsoleEvents
#[test]
fn assistant_text_and_tool_calls_become_console_events() {
    let line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Checking the set."},{"type":"tool_use","name":"mcp__dj-vevr-se__dj_set","input":{"action":"show","set":"first-night","wander":true}},{"type":"text","text":"   "}]}}"#;
    assert_eq!(
        agent::parse(line),
        [
            AgentEvent::Text("Checking the set.".into()),
            AgentEvent::ToolUse {
                name: "dj_set".into(),
                summary: "action=show set=first-night wander=true".into()
            },
        ]
    );
    // other servers' tools keep their full name; long arguments are cut
    let long = "x".repeat(300);
    let e = agent::parse(&format!(
        r#"{{"type":"assistant","message":{{"content":[{{"type":"tool_use","name":"Read","input":{{"file_path":"{long}"}}}}]}}}}"#
    ));
    match e.first() {
        Some(AgentEvent::ToolUse { name, summary }) => {
            assert!(name == "Read" && summary.chars().count() <= 200 && summary.ends_with('…'));
        }
        other => panic!("no tool use: {other:?}"),
    }
}

// toolResultsAreOneLineAndErrorsAreFlagged
#[test]
fn tool_results_are_one_line_and_errors_are_flagged() {
    let a = agent::parse(
        r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"line one\nline two"}]}}"#,
    );
    assert_eq!(
        a,
        [AgentEvent::ToolResult {
            text: "line one line two".into(),
            is_error: false
        }]
    );
    let b = agent::parse(
        r#"{"type":"user","message":{"content":[{"type":"tool_result","is_error":true,"content":[{"type":"text","text":"no set called x"}]}]}}"#,
    );
    assert_eq!(
        b,
        [AgentEvent::ToolResult {
            text: "no set called x".into(),
            is_error: true
        }]
    );
    assert!(agent::parse(r#"{"type":"user","message":{"content":"hello"}}"#).is_empty());
}

// aTurnEndsWithItsCostOrItsError
#[test]
fn a_turn_ends_with_its_cost_or_its_error() {
    assert_eq!(
        agent::parse(
            r#"{"type":"result","subtype":"success","is_error":false,"total_cost_usd":0.452,"num_turns":2}"#
        ),
        [AgentEvent::TurnDone {
            ok: true,
            cost: Some(0.452),
            turns: Some(2),
            message: None
        }]
    );
    assert_eq!(
        agent::parse(
            r#"{"type":"result","subtype":"error_max_budget_usd","total_cost_usd":5.1,"num_turns":9}"#
        ),
        [AgentEvent::TurnDone {
            ok: false,
            cost: Some(5.1),
            turns: Some(9),
            message: Some("error_max_budget_usd".into())
        }]
    );
    assert!(
        agent::parse("not json").is_empty()
            && agent::parse(r#"{"type":"rate_limit_event"}"#).is_empty()
    );
}

// aMessageIsOneStreamJSONUserLine
#[test]
fn a_message_is_one_stream_json_user_line() {
    let line = agent::user_message("more energy, \"darker\"\nplease");
    assert!(!line.contains('\n'));
    let v = parse(&line);
    assert_eq!(v.get("type").unwrap().as_str(), Some("user"));
    assert_eq!(
        v.get("message").unwrap().get("role").unwrap().as_str(),
        Some("user")
    );
    assert_eq!(
        v.get("message").unwrap().get("content").unwrap().as_str(),
        Some("more energy, \"darker\"\nplease")
    );
}

// theLaunchArgumentsPrimeTheDJAndLockItToTheApp
#[test]
fn the_launch_arguments_prime_the_dj_and_lock_it_to_the_app() {
    let mut l = Launch::default();
    let base = agent::arguments(&l);
    assert_eq!(
        base[..7],
        [
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--agent"
        ]
    );
    let has = |a: &[String], s: &str| a.iter().any(|x| x == s);
    assert!(has(&base, "dj") && has(&base, "--permission-mode") && has(&base, "dontAsk"));
    assert!(has(&base, "--max-budget-usd") && has(&base, "5") && !has(&base, "--resume"));
    assert!(!has(&base, "--strict-mcp-config") && !has(&base, "--model"));
    l.mcp_config_path = Some("/x/mcp.json".into());
    l.model = Some("opus".into());
    l.resume = Some("sess-1".into());
    l.budget_usd = Some(2.5);
    let a = agent::arguments(&l);
    assert!(has(&a, "--strict-mcp-config") && has(&a, "--mcp-config") && has(&a, "/x/mcp.json"));
    assert!(
        has(&a, "--model")
            && has(&a, "opus")
            && has(&a, "--resume")
            && has(&a, "sess-1")
            && has(&a, "2.5")
    );
    let i = a.iter().position(|x| x == "--allowedTools").unwrap();
    assert_eq!(a[i + 1], "mcp__dj-vevr-se,Skill,Read,Glob,Grep");
    l.budget_usd = None;
    assert!(!has(&agent::arguments(&l), "--max-budget-usd"));
    // %g
    for (x, s) in [
        (5.0, "5"),
        (2.5, "2.5"),
        (0.125, "0.125"),
        (12.0, "12"),
        (100000.0, "100000"),
        (0.0001, "0.0001"),
    ] {
        assert_eq!(agent::format_g(x), s);
    }
    assert_eq!(agent::format_g(1500000.0), "1.5e+06");
}

// theMCPConfigNamesOnlyTheAppsServer
#[test]
fn the_mcp_config_names_only_the_apps_server() {
    let v = parse(&agent::mcp_config("/a b/djctl"));
    let servers = v.get("mcpServers").unwrap().as_object().unwrap();
    assert_eq!(servers.keys().collect::<Vec<_>>(), ["dj-vevr-se"]);
    let s = servers.get("dj-vevr-se").unwrap();
    assert_eq!(s.get("command").unwrap().as_str(), Some("/a b/djctl"));
    assert_eq!(
        s.get("args").unwrap(),
        &JsonValue::Array(vec!["mcp".into()])
    );
}

// claudeAndDjctlAreFoundWhereTheyUsuallyAre
#[test]
fn claude_and_djctl_are_found_where_they_usually_are() {
    let have = [
        "/opt/bin/claude",
        "/Users/m/.local/bin/claude",
        "/p/build/mcp/djctl",
        "/custom/djctl",
    ];
    let exists = |p: &str| have.contains(&p);
    let env = |pairs: &'static [(&'static str, &'static str)]| {
        move |k: &str| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        }
    };
    assert_eq!(
        agent::find_claude(&env(&[("PATH", "/usr/bin:/opt/bin")]), "/Users/m", &exists).as_deref(),
        Some("/opt/bin/claude")
    );
    assert_eq!(
        agent::find_claude(&env(&[("PATH", "/usr/bin")]), "/Users/m", &exists).as_deref(),
        Some("/Users/m/.local/bin/claude")
    );
    assert_eq!(
        agent::find_claude(
            &env(&[("DJ_CLAUDE", "/opt/bin/claude"), ("PATH", "")]),
            "/h",
            &exists
        )
        .as_deref(),
        Some("/opt/bin/claude")
    );
    assert_eq!(agent::find_claude(&env(&[]), "/nowhere", &|_| false), None);
    assert_eq!(
        agent::find_djctl(&env(&[]), "/p", &exists).as_deref(),
        Some("/p/build/mcp/djctl")
    );
    assert_eq!(
        agent::find_djctl(&env(&[("DJ_DJCTL", "/custom/djctl")]), "/q", &exists).as_deref(),
        Some("/custom/djctl")
    );
    assert_eq!(agent::find_djctl(&env(&[]), "/q", &exists), None);
}

#[allow(dead_code)]
fn _unused(_: JsonObject) {}
