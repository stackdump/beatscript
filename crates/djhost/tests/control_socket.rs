//! `ControlTests.swift` (ControlSocketTests): a request round-trips through the socket, the
//! socket is user-only and goes away on stop, no app is a clear error, garbage gets an error
//! line. Plus the MCP loop through a pipe and the default path rules.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::time::Duration;

use djengine::json::{JsonObject, JsonValue};
use djengine::mcp::McpBridge;
use djhost::control_socket::{
    default_path, respond, serve_mcp, ControlClient, ControlError, ControlServer, Handler,
};

/// A fresh socket path. sun_path is 104 bytes, so not under a long temp dir (Swift's note).
fn temp_socket() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    format!(
        "/tmp/djc-{}-{}.sock",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

fn obj(pairs: Vec<(&str, JsonValue)>) -> JsonValue {
    JsonValue::Object(JsonObject::from_pairs(pairs))
}

// aRequestRoundTripsThroughTheSocket
#[test]
fn a_request_round_trips_through_the_socket() {
    let path = temp_socket();
    let handler: Arc<Handler> = Arc::new(|method, params| {
        if method == "boom" {
            return Err("no deck C".to_string());
        }
        Ok(obj(vec![
            ("method", method.into()),
            ("echo", params.clone()),
        ]))
    });
    let mut server = ControlServer::bind(&path, handler).unwrap();
    let t = Duration::from_secs(60);
    let mut c = ControlClient::connect(&path, t).unwrap();
    let first = c
        .request("state", &obj(vec![("x", JsonValue::int(1))]))
        .unwrap();
    let second = c.request("catalog", &obj(vec![])).unwrap(); // same connection, next line
    assert_eq!(first.get("method").unwrap().as_str(), Some("state"));
    assert_eq!(
        first.get("echo").unwrap().get("x").unwrap().as_i64(),
        Some(1)
    );
    assert_eq!(second.get("method").unwrap().as_str(), Some("catalog"));
    let err = ControlClient::connect(&path, t)
        .unwrap()
        .request("boom", &obj(vec![]))
        .unwrap_err();
    assert_eq!(err, ControlError::Remote("no deck C".into()));
    server.stop();
}

// theSocketIsUserOnlyAndGoesAwayOnStop
#[test]
fn the_socket_is_user_only_and_goes_away_on_stop() {
    let path = temp_socket();
    let mut server = ControlServer::bind(&path, Arc::new(|_, _| Ok(JsonValue::Null))).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    server.stop();
    assert!(!std::path::Path::new(&path).exists());
    // a stopped server's path is free again, and stop twice is fine
    server.stop();
    assert!(ControlClient::connect(&path, Duration::from_secs(1)).is_err());
}

// noAppMeansAClearError
#[test]
fn no_app_means_a_clear_error() {
    let path = temp_socket();
    match ControlClient::connect(&path, Duration::from_secs(1)) {
        Err(ControlError::Unavailable(m)) => {
            assert!(m.contains("not running") && m.contains(&path), "{m}")
        }
        Ok(_) => panic!("connected to nothing"),
        Err(e) => panic!("{e:?}"),
    }
}

// garbageGetsAnErrorLineNotADroppedConnection
#[test]
fn garbage_gets_an_error_line_not_a_dropped_connection() {
    let h: &Handler = &|_, _| Ok(JsonValue::Null);
    let reply = JsonValue::parse(&respond("not json", h)).unwrap();
    assert!(reply.get("error").and_then(JsonValue::as_str).is_some());
    assert_eq!(reply.get("id"), Some(&JsonValue::Null));
    // ids, results and missing params
    let ok = JsonValue::parse(&respond(r#"{"id":7,"method":"x"}"#, &|m, p| {
        Ok(obj(vec![("m", m.into()), ("p", p.clone())]))
    }))
    .unwrap();
    assert_eq!(ok.get("id").unwrap().as_i64(), Some(7));
    assert_eq!(ok.get("result").unwrap().get("p").unwrap(), &obj(vec![]));
}

// a second client keeps working while the first is open, and a restart replaces the file
#[test]
fn clients_are_independent_and_a_restart_replaces_the_socket() {
    let path = temp_socket();
    let h: Arc<Handler> = Arc::new(|m, _| Ok(m.into()));
    let mut s1 = ControlServer::bind(&path, h.clone()).unwrap();
    let t = Duration::from_secs(30);
    let mut a = ControlClient::connect(&path, t).unwrap();
    let mut b = ControlClient::connect(&path, t).unwrap();
    assert_eq!(
        a.request("one", &obj(vec![])).unwrap().as_str(),
        Some("one")
    );
    assert_eq!(
        b.request("two", &obj(vec![])).unwrap().as_str(),
        Some("two")
    );
    s1.stop();
    let mut s2 = ControlServer::bind(&path, h).unwrap();
    assert_eq!(
        ControlClient::once(&path, t, "three", &obj(vec![]))
            .unwrap()
            .as_str(),
        Some("three")
    );
    s2.stop();
}

// the MCP loop: a handshake and a tool call through a pipe, one control request per call
#[test]
fn the_mcp_loop_runs_a_session_through_a_pipe() {
    let path = temp_socket();
    let mut server = ControlServer::bind(
        &path,
        Arc::new(|m, p| Ok(obj(vec![("method", m.into()), ("args", p.clone())]))),
    )
    .unwrap();
    let input = concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        "\n",
        "\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"dj_state","arguments":{}}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"dj_generate","arguments":{"deck":"A","genre":"techno","seed":7}}}"#,
        "\n",
    );
    let mut out = Vec::new();
    let bridge = McpBridge::new("dev");
    let p2 = path.clone();
    let mut call = move |m: &str, p: &JsonValue| {
        ControlClient::once(&p2, Duration::from_secs(30), m, p).map_err(|e| e.to_string())
    };
    serve_mcp(input.as_bytes(), &mut out, &bridge, &mut call).unwrap();
    server.stop();
    let lines: Vec<JsonValue> = String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| JsonValue::parse(l).unwrap())
        .collect();
    assert_eq!(
        lines.len(),
        3,
        "the notification and the blank line get no reply"
    );
    assert_eq!(lines[0].get("id").unwrap().as_i64(), Some(1));
    let text = |i: usize| {
        lines[i]
            .get("result")
            .unwrap()
            .get("content")
            .unwrap()
            .at(0)
            .unwrap()
            .get("text")
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    };
    assert_eq!(text(1), r#"{"method":"state","args":{}}"#);
    assert_eq!(
        text(2),
        r#"{"method":"generate","args":{"deck":"A","genre":"techno","seed":7}}"#
    );
    // with the app gone, a call is an isError tool result, not a dropped session
    let mut call = |m: &str, p: &JsonValue| {
        ControlClient::once(&path, Duration::from_secs(1), m, p).map_err(|e| e.to_string())
    };
    let mut out = Vec::new();
    serve_mcp(
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"dj_state"}}"#.as_bytes(),
        &mut out,
        &bridge,
        &mut call,
    )
    .unwrap();
    let r = JsonValue::parse(String::from_utf8(out).unwrap().trim()).unwrap();
    let res = r.get("result").unwrap();
    assert_eq!(res.get("isError").unwrap().as_bool(), Some(true));
    assert!(res
        .get("content")
        .unwrap()
        .at(0)
        .unwrap()
        .get("text")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("not running"));
}

#[test]
fn the_default_path_follows_the_environment() {
    // all env mutation in one test (the tests in this file run on threads)
    std::env::set_var("DJ_CONTROL_SOCKET", "/x/y.sock");
    assert_eq!(default_path(), "/x/y.sock");
    std::env::remove_var("DJ_CONTROL_SOCKET");
    std::env::set_var("XDG_DATA_HOME", "/data");
    std::env::set_var("HOME", "/home/m");
    let p = default_path();
    if djhost::platform::IS_MACOS {
        assert_eq!(p, "/home/m/Library/Application Support/DJApp/control.sock");
    } else {
        assert_eq!(p, "/data/DJApp/control.sock");
    }
}

/// A handler that answers `batch` the way djapp does, from a script: a `deck` command succeeds
/// with its own params, a `mixer` one fails, and what it was given is kept.
fn batch_handler(seen: Arc<std::sync::Mutex<Vec<JsonValue>>>) -> Arc<Handler> {
    Arc::new(move |method, params| {
        if method != "batch" {
            return Ok(JsonValue::Null);
        }
        seen.lock().unwrap().push(params.clone());
        let plan = djhost::batch::Plan::parse(params)?;
        let rows = plan
            .commands
            .iter()
            .map(|c| {
                let mut r = JsonObject::new();
                r.set("index", JsonValue::int(c.index as i64));
                match (&c.error, c.noun.as_str()) {
                    (Some(e), _) => {
                        r.set("status", "error".into());
                        r.set("error", JsonValue::str(e));
                    }
                    (None, "mixer") => {
                        r.set("status", "error".into());
                        r.set("error", "mixer: no deck Q".into());
                    }
                    (None, _) => {
                        r.set("status", "ok".into());
                        r.set("result", c.params.clone());
                    }
                }
                JsonValue::Object(r)
            })
            .collect();
        Ok(obj(vec![("id", "b1".into()), ("results", JsonValue::Array(rows))]))
    })
}

// an array of requests is one batch, answered as an array in order with each request's own id
#[test]
fn an_array_of_requests_is_one_batch_answered_as_an_array() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let h = batch_handler(seen.clone());
    let line = r#"[
        {"id":"a","method":"deck","params":{"action":"play","deck":"A"}},
        {"id":2,"method":"mixer","params":{"deck":"Q","mute":true}},
        {"id":"c","method":"deck.pause","deck":"B"},
        {"id":4,"method":"nope"},
        "not a request",
        {"params":{}}
    ]"#
    .replace('\n', " ");
    let reply = JsonValue::parse(&respond(&line, &*h)).unwrap();
    let rows = reply.as_array().expect("an array in, an array out");
    assert_eq!(rows.len(), 6, "one answer per request, in order");
    let id = |i: usize| rows[i].get("id").cloned().unwrap();
    assert_eq!((id(0), id(1), id(2), id(3)), ("a".into(), JsonValue::int(2), "c".into(), JsonValue::int(4)));
    assert_eq!(rows[0].get("result").and_then(|r| r.get("deck")).and_then(JsonValue::as_str), Some("A"));
    assert_eq!(rows[1].get("error").and_then(JsonValue::as_str), Some("mixer: no deck Q"), "a failure does not stop the rest");
    assert_eq!(rows[2].get("result").and_then(|r| r.get("action")).and_then(JsonValue::as_str), Some("pause"), "dotted and inline, as ever");
    assert!(rows[3].get("error").and_then(JsonValue::as_str).unwrap().contains("unknown method"));
    for r in &rows[4..] {
        assert_eq!(r.get("id"), Some(&JsonValue::Null));
        assert!(r.get("error").and_then(JsonValue::as_str).unwrap().contains("expected"));
    }
    // one call to the handler, with the whole array, not stopping on an error
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].get("commands").and_then(JsonValue::as_array).map(Vec::len), Some(4), "only the requests");
    assert_eq!(seen[0].get("stop_on_error").and_then(JsonValue::as_bool), Some(false));
}

#[test]
fn a_batch_that_cannot_be_made_is_one_error_object() {
    let h = batch_handler(Arc::new(std::sync::Mutex::new(Vec::new())));
    let r = JsonValue::parse(&respond("[]", &*h)).unwrap();
    assert_eq!(r.get("id"), Some(&JsonValue::Null));
    assert!(r.get("error").and_then(JsonValue::as_str).unwrap().contains("empty"));
    let r = JsonValue::parse(&respond("[1,", &*h)).unwrap();
    assert!(r.get("error").and_then(JsonValue::as_str).unwrap().contains("array"));
    // a handler that has no batch (or refuses it) says so once
    let refuse: Arc<Handler> = Arc::new(|_, _| Err("unknown method \"batch\"".to_string()));
    let r = JsonValue::parse(&respond(r#"[{"id":1,"method":"deck"}]"#, &*refuse)).unwrap();
    assert!(r.get("error").and_then(JsonValue::as_str).unwrap().contains("batch"));
    // an object line is still the single-request form
    let r = JsonValue::parse(&respond(r#"{"id":9,"method":"deck","params":{"action":"play"}}"#, &*h)).unwrap();
    assert_eq!(r.get("id").and_then(JsonValue::as_i64), Some(9));
}

// over a real socket: the client's `requests` is the array form
#[test]
fn the_client_sends_an_array_over_the_socket() {
    let path = temp_socket();
    let mut server = ControlServer::bind(&path, batch_handler(Arc::new(std::sync::Mutex::new(Vec::new())))).unwrap();
    let mut c = ControlClient::connect(&path, Duration::from_secs(30)).unwrap();
    let r = c
        .requests(&[
            ("deck", obj(vec![("action", "play".into())])),
            ("mixer", obj(vec![])),
        ])
        .unwrap();
    assert_eq!(r.len(), 2);
    assert!(r[0].is_ok());
    assert_eq!(r[1], Err("mixer: no deck Q".to_string()));
    // the same connection still takes a single request after it
    assert!(c.request("deck", &obj(vec![])).is_ok());
    server.stop();
}
