//! The agent process against a fake `claude` (a shell script): launch arguments, the init line,
//! a turn per user line, a clean stop. No network, no money.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::sync::{mpsc, Mutex, MutexGuard};
use std::time::Duration;

use djengine::agent::{AgentEvent, Launch};
use djhost::agent_process::AgentProcess;

/// Held across writing the fake `claude` and spawning. A child forked by the other test while the
/// script is open for writing inherits that descriptor until it execs, and Linux then refuses to
/// exec the script: "Text file busy".
static SPAWNING: Mutex<()> = Mutex::new(());

fn spawning() -> MutexGuard<'static, ()> {
    SPAWNING.lock().unwrap_or_else(|e| e.into_inner())
}

fn fake_claude(dir: &std::path::Path) -> std::path::PathBuf {
    let script = dir.join("claude");
    std::fs::write(
        &script,
        r#"#!/bin/sh
echo "$@" > "$(dirname "$0")/args.txt"
echo '{"type":"system","subtype":"init","session_id":"sess-9","model":"fake"}'
while read -r line; do
  echo "$line" >> "$(dirname "$0")/said.txt"
  echo '{"type":"assistant","message":{"content":[{"type":"text","text":"heard you"},{"type":"tool_use","name":"mcp__dj-vevr-se__dj_state","input":{}}]}}'
  echo '{"type":"user","message":{"content":[{"type":"tool_result","content":"ok"}]}}'
  echo '{"type":"result","subtype":"success","is_error":false,"total_cost_usd":0.01,"num_turns":1}'
done
"#,
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script
}

#[test]
fn a_turn_goes_in_and_its_events_come_out() {
    let dir = std::env::temp_dir().join(format!("djagent-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let guard = spawning();
    let claude = fake_claude(&dir);
    let (tx, rx) = mpsc::channel();
    let launch = Launch {
        mcp_config_path: Some("/x/mcp.json".into()),
        resume: Some("old-session".into()),
        ..Launch::default()
    };
    let mut agent = AgentProcess::spawn(claude.to_str().unwrap(), &launch, &dir, &[], move |e| {
        let _ = tx.send(e);
    })
    .unwrap();
    drop(guard);
    let next = || rx.recv_timeout(Duration::from_secs(10)).expect("an event");
    assert_eq!(
        next(),
        AgentEvent::Ready {
            session: "sess-9".into(),
            model: Some("fake".into())
        }
    );
    agent.say("more energy, \"darker\"").unwrap();
    assert_eq!(next(), AgentEvent::Text("heard you".into()));
    assert_eq!(
        next(),
        AgentEvent::ToolUse {
            name: "dj_state".into(),
            summary: String::new()
        }
    );
    assert_eq!(
        next(),
        AgentEvent::ToolResult {
            text: "ok".into(),
            is_error: false
        }
    );
    assert_eq!(
        next(),
        AgentEvent::TurnDone {
            ok: true,
            cost: Some(0.01),
            turns: Some(1),
            message: None
        }
    );
    assert_eq!(agent.session.lock().unwrap().as_deref(), Some("sess-9"));
    assert!(agent.is_running());
    agent.stop();
    assert!(!agent.is_running());
    assert!(
        agent.say("late").is_err(),
        "a stopped agent takes no more turns"
    );

    let args = std::fs::read_to_string(dir.join("args.txt")).unwrap();
    for want in [
        "-p",
        "--agent dj",
        "--permission-mode dontAsk",
        "--strict-mcp-config",
        "--mcp-config /x/mcp.json",
        "--resume old-session",
        "--max-budget-usd 5",
    ] {
        assert!(args.contains(want), "{want} in {args}");
    }
    // the user turn went in as one stream-json line
    let said = std::fs::read_to_string(dir.join("said.txt")).unwrap();
    let line = djengine::json::JsonValue::parse(said.lines().next().unwrap()).unwrap();
    assert_eq!(
        line.get("message")
            .unwrap()
            .get("content")
            .unwrap()
            .as_str(),
        Some("more energy, \"darker\"")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_claude_is_an_error_not_a_hang() {
    let _guard = spawning();
    let r = AgentProcess::spawn(
        "/no/such/claude",
        &Launch::default(),
        std::path::Path::new("."),
        &[],
        |_| {},
    );
    assert!(r.is_err());
}
