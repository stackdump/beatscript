//! The running app's control channel: a Unix socket speaking newline-delimited JSON. Port of
//! `Control/ControlSocket.swift`.
//!
//! ```text
//! → {"id": 1, "method": "state", "params": {}}
//! ← {"id": 1, "result": {…}}            or   {"id": 1, "error": "no deck C"}
//! ```
//!
//! The socket is user-only (0600) and local; `djctl mcp` bridges it to MCP. The handler is where
//! the host decides what a method means; this file only moves lines. Unix only (the type
//! definitions that need no socket are available everywhere).

use std::fmt;

use djengine::json::{JsonObject, JsonValue};

/// `ControlSocket.defaultPath`: `$DJ_CONTROL_SOCKET`, else `DJApp/control.sock` under the
/// app-support folder (`~/Library/Application Support` on macOS, where the Swift app listens;
/// `$XDG_DATA_HOME` / `~/.local/share` elsewhere).
pub fn default_path() -> String {
    if let Some(p) = std::env::var("DJ_CONTROL_SOCKET")
        .ok()
        .filter(|p| !p.is_empty())
    {
        return p;
    }
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| ".".into());
    crate::platform::data_dir_in(&home)
        .join("DJApp/control.sock")
        .to_string_lossy()
        .into_owned()
}

/// `ControlError`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ControlError {
    /// The app answered with an error.
    Remote(String),
    /// Nothing listening, or the line could not be sent / read.
    Unavailable(String),
    BadReply(String),
}

impl fmt::Display for ControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ControlError::Remote(s) | ControlError::Unavailable(s) => f.write_str(s),
            ControlError::BadReply(s) => write!(f, "bad reply from DJApp: {s}"),
        }
    }
}

impl std::error::Error for ControlError {}

/// `ControlHandler`: `(method, params)` → the result, or the error text.
pub type Handler = dyn Fn(&str, &JsonValue) -> Result<JsonValue, String> + Send + Sync;

/// `ControlServer.respond`: one request line → one reply line. Never fails: errors become
/// `{"error": …}`.
pub fn respond(line: &str, handler: &Handler) -> String {
    // a JSON-RPC-style array of requests is one `batch` (docs/CONTROL.md, "Batches")
    if line.trim_start().starts_with('[') {
        return respond_array(line, handler);
    }
    let mut reply = JsonObject::new();
    let req = JsonValue::parse(line).ok();
    let Some((req, method)) = req.as_ref().and_then(|r| {
        let m = r.get("method")?.as_str()?;
        Some((r, m))
    }) else {
        reply.set("id", JsonValue::Null);
        reply.set("error", "expected {\"id\", \"method\", \"params\"}".into());
        return JsonValue::Object(reply).js_stringified();
    };
    reply.set("id", req.get("id").cloned().unwrap_or(JsonValue::Null));
    // `params` may be left out with the params beside `method` instead: {"method":"deck","action":"play"}
    let params = req.get("params").cloned().unwrap_or_else(|| {
        let mut o = JsonObject::new();
        if let Some(all) = req.as_object() {
            for (k, v) in all.iter().filter(|(k, _)| !matches!(*k, "id" | "method")) {
                o.set(k, v.clone());
            }
        }
        JsonValue::Object(o)
    });
    match handler(method, &params) {
        Ok(v) => reply.set("result", v),
        Err(e) => reply.set("error", e.into()),
    }
    JsonValue::Object(reply).js_stringified()
}

/// An array of requests: `[{"id":1,"method":"deck","params":{...}}, {"id":2,...}]` answered as
/// `[{"id":1,"result":...}, {"id":2,"error":"..."}]`, in order, each with its own id. It is one
/// `batch` call to the handler (so the window answers the whole array in one service pass, in
/// order) with `stop_on_error` off, as JSON-RPC's batches are independent calls: every request
/// is answered whether or not an earlier one failed. A request that is not an object with a
/// `method` is answered with an error in its place. A failure of the batch itself (an empty
/// array, a handler with no `batch`) is one `{"id":null,"error":...}` object, as JSON-RPC has it.
fn respond_array(line: &str, handler: &Handler) -> String {
    let whole = |e: &str| {
        JsonValue::Object(JsonObject::from_pairs([("id", JsonValue::Null), ("error", e.into())])).js_stringified()
    };
    let Ok(JsonValue::Array(items)) = JsonValue::parse(line) else {
        return whole("expected a JSON array of {\"id\", \"method\", \"params\"} requests");
    };
    if items.is_empty() {
        return whole("an empty batch: send at least one request");
    }
    let mut commands = Vec::new();
    // each request's id, and where its command sits in the batch (None: it was not a request)
    let mut slots: Vec<(JsonValue, Option<usize>)> = Vec::new();
    for item in &items {
        let id = item.get("id").cloned().unwrap_or(JsonValue::Null);
        if item.get("method").and_then(JsonValue::as_str).is_some() {
            slots.push((id, Some(commands.len())));
            commands.push(item.clone());
        } else {
            slots.push((id, None));
        }
    }
    let params = JsonValue::Object(JsonObject::from_pairs([
        ("commands", JsonValue::Array(commands)),
        ("stop_on_error", false.into()),
    ]));
    let answer = match handler("batch", &params) {
        Ok(v) => v,
        Err(e) => return whole(&e),
    };
    let rows = answer.get("results").and_then(JsonValue::as_array).cloned().unwrap_or_default();
    let replies = slots
        .into_iter()
        .map(|(id, slot)| {
            let mut r = JsonObject::new();
            r.set("id", id);
            match slot.and_then(|k| rows.get(k)) {
                None => r.set("error", "expected {\"id\", \"method\", \"params\"}".into()),
                Some(row) => match row.get("status").and_then(JsonValue::as_str) {
                    Some("ok") => r.set("result", row.get("result").cloned().unwrap_or(JsonValue::Null)),
                    Some("waiting") => r.set(
                        "result",
                        JsonValue::Object(JsonObject::from_pairs([
                            ("pending", true.into()),
                            ("batch", answer.get("id").cloned().unwrap_or(JsonValue::Null)),
                        ])),
                    ),
                    Some("skipped") => r.set("error", row.get("skipped").cloned().unwrap_or_else(|| "skipped".into())),
                    _ => r.set("error", row.get("error").cloned().unwrap_or_else(|| "failed".into())),
                },
            }
            JsonValue::Object(r)
        })
        .collect();
    JsonValue::Array(replies).js_stringified()
}

#[cfg(unix)]
pub use unix::{ControlClient, ControlServer};

#[cfg(unix)]
mod unix {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread::JoinHandle;
    use std::time::Duration;

    /// `ControlServer`: bind and accept on a thread; a connection is served on its own thread,
    /// one reply line per request line.
    pub struct ControlServer {
        pub path: PathBuf,
        stopped: Arc<AtomicBool>,
        accept: Option<JoinHandle<()>>,
    }

    impl ControlServer {
        /// Bind and start accepting. An existing socket file at `path` is replaced; its folder
        /// is created. The socket is 0600.
        pub fn bind(
            path: impl AsRef<Path>,
            handler: Arc<Handler>,
        ) -> Result<ControlServer, ControlError> {
            let path = path.as_ref().to_path_buf();
            if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
                std::fs::create_dir_all(dir).map_err(|e| {
                    ControlError::Unavailable(format!("create {}: {e}", dir.display()))
                })?;
            }
            let _ = std::fs::remove_file(&path);
            let listener = UnixListener::bind(&path)
                .map_err(|e| ControlError::Unavailable(format!("bind {}: {e}", path.display())))?;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| ControlError::Unavailable(format!("chmod {}: {e}", path.display())))?;
            let stopped = Arc::new(AtomicBool::new(false));
            let flag = stopped.clone();
            let accept = std::thread::Builder::new()
                .name("dj-control-accept".into())
                .spawn(move || {
                    for conn in listener.incoming() {
                        if flag.load(Ordering::SeqCst) {
                            return; // the wake-up connection from `stop`
                        }
                        let Ok(stream) = conn else { continue };
                        let handler = handler.clone();
                        let _ = std::thread::Builder::new()
                            .name("dj-control-conn".into())
                            .spawn(move || serve(stream, &*handler));
                    }
                })
                .map_err(|e| ControlError::Unavailable(format!("thread: {e}")))?;
            Ok(ControlServer {
                path,
                stopped,
                accept: Some(accept),
            })
        }

        /// Stop accepting and remove the socket file. Open connections finish their line.
        pub fn stop(&mut self) {
            if self.stopped.swap(true, Ordering::SeqCst) {
                return;
            }
            // wake the `accept`: it takes this connection, sees the flag and returns
            let _ = UnixStream::connect(&self.path);
            if let Some(t) = self.accept.take() {
                let _ = t.join();
            }
            let _ = std::fs::remove_file(&self.path);
        }
    }

    impl Drop for ControlServer {
        fn drop(&mut self) {
            self.stop();
        }
    }

    fn serve(stream: UnixStream, handler: &Handler) {
        let Ok(mut out) = stream.try_clone() else {
            return;
        };
        for line in BufReader::new(stream).lines() {
            let Ok(line) = line else { return };
            if line.is_empty() {
                continue;
            }
            let mut reply = respond(&line, handler);
            reply.push('\n');
            if out.write_all(reply.as_bytes()).is_err() {
                return;
            }
        }
    }

    /// `ControlClient`: one connection to the app. Synchronous: `djctl` is a plain CLI.
    pub struct ControlClient {
        reader: BufReader<UnixStream>,
        writer: UnixStream,
        next_id: i64,
    }

    impl ControlClient {
        /// `timeout`: how long one request may take (composing a track takes a few seconds).
        pub fn connect(
            path: impl AsRef<Path>,
            timeout: Duration,
        ) -> Result<ControlClient, ControlError> {
            let path = path.as_ref();
            let stream = UnixStream::connect(path).map_err(|_| {
                ControlError::Unavailable(format!(
                    "DJApp is not running (nothing listening on {}). Start the app, or `djctl serve`.",
                    path.display()
                ))
            })?;
            let _ = stream.set_read_timeout(Some(timeout));
            let writer = stream
                .try_clone()
                .map_err(|e| ControlError::Unavailable(format!("socket: {e}")))?;
            Ok(ControlClient {
                reader: BufReader::new(stream),
                writer,
                next_id: 1,
            })
        }

        /// One line out, one line back.
        fn exchange(&mut self, line: String) -> Result<JsonValue, ControlError> {
            let mut line = line;
            line.push('\n');
            self.writer
                .write_all(line.as_bytes())
                .map_err(|_| ControlError::Unavailable("DJApp closed the connection".into()))?;
            let mut reply = String::new();
            match self.reader.read_line(&mut reply) {
                Ok(0) => {
                    return Err(ControlError::Unavailable(
                        "DJApp closed the connection".into(),
                    ))
                }
                Ok(_) => {}
                Err(e) => {
                    return Err(ControlError::Unavailable(format!(
                        "no reply from DJApp ({e})"
                    )))
                }
            }
            let reply = reply.trim_end_matches(['\n', '\r']);
            JsonValue::parse(reply).map_err(|_| ControlError::BadReply(reply.to_string()))
        }

        pub fn request(
            &mut self,
            method: &str,
            params: &JsonValue,
        ) -> Result<JsonValue, ControlError> {
            let id = self.next_id;
            self.next_id += 1;
            let req = JsonValue::Object(JsonObject::from_pairs([
                ("id", JsonValue::int(id)),
                ("method", method.into()),
                ("params", params.clone()),
            ]));
            let v = self.exchange(req.js_stringified())?;
            if let Some(e) = v.get("error").and_then(JsonValue::as_str) {
                return Err(ControlError::Remote(e.to_string()));
            }
            Ok(v.get("result").cloned().unwrap_or(JsonValue::Null))
        }

        /// Several requests as one array line (`respond`'s array form): one answer per request,
        /// in order, run in order in one pass of the app. The batch itself failing is the `Err`.
        pub fn requests(
            &mut self,
            reqs: &[(&str, JsonValue)],
        ) -> Result<Vec<Result<JsonValue, String>>, ControlError> {
            let line = JsonValue::Array(
                reqs.iter()
                    .enumerate()
                    .map(|(i, (m, p))| {
                        JsonValue::Object(JsonObject::from_pairs([
                            ("id", JsonValue::int(i as i64 + 1)),
                            ("method", (*m).into()),
                            ("params", p.clone()),
                        ]))
                    })
                    .collect(),
            )
            .js_stringified();
            match self.exchange(line)? {
                JsonValue::Array(rows) => Ok(rows
                    .iter()
                    .map(|r| match r.get("error").and_then(JsonValue::as_str) {
                        Some(e) => Err(e.to_string()),
                        None => Ok(r.get("result").cloned().unwrap_or(JsonValue::Null)),
                    })
                    .collect()),
                v => Err(ControlError::Remote(
                    v.get("error").and_then(JsonValue::as_str).unwrap_or("not an array").to_string(),
                )),
            }
        }

        /// One request on a fresh connection (what `djctl mcp` does per tool call, so the app
        /// can restart under a live MCP session).
        pub fn once(
            path: impl AsRef<Path>,
            timeout: Duration,
            method: &str,
            params: &JsonValue,
        ) -> Result<JsonValue, ControlError> {
            ControlClient::connect(path, timeout)?.request(method, params)
        }
    }
}

/// Serve MCP over a line transport: each request line in `input` goes to `bridge`, each reply
/// is written to `output` with a newline (`djctl mcp`'s loop). Stops at end of input.
pub fn serve_mcp(
    input: impl std::io::BufRead,
    mut output: impl std::io::Write,
    bridge: &djengine::mcp::McpBridge<'_>,
    call: &mut djengine::mcp::Call<'_>,
) -> std::io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = bridge.handle(&line, call) {
            output.write_all(reply.as_bytes())?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
    Ok(())
}
