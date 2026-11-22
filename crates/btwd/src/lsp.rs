//! `btwd lsp`: the btw language server over stdio (the LSP base protocol: `Content-Length` headers, JSON-RPC bodies). All of the
//! language is btwls's [`btwls::Server`]; this is only the framing.

use std::io::{BufRead, Write};

use djengine::json::JsonValue;

/// Read one message's body: headers to the blank line, then `Content-Length` bytes. None at the end of the input.
pub fn read_message(r: &mut impl BufRead) -> std::io::Result<Option<String>> {
    let mut len: Option<usize> = None;
    loop {
        let mut line = String::new();
        if r.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            if len.is_some() {
                break;
            }
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                len = v.trim().parse().ok();
            }
        }
    }
    let mut body = vec![0u8; len.unwrap_or(0)];
    r.read_exact(&mut body)?;
    Ok(Some(String::from_utf8_lossy(&body).into_owned()))
}

pub fn write_message(w: &mut impl Write, v: &JsonValue) -> std::io::Result<()> {
    let body = v.js_stringified();
    write!(w, "Content-Length: {}\r\n\r\n{body}", body.len())?;
    w.flush()
}

/// What the main loop waits on: a message from the client, the end of the input, or a worker's check.
enum Event {
    Message(String),
    End,
    Checked(String, i64, String, djlive::compile::Check),
}

/// How long the text must rest before it is checked (the compile; the parser's diagnostics are published at once).
const DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(150);

/// Serve until `exit` or the end of the input. Exit code 0 after a `shutdown`, 1 without one (the LSP's rule).
///
/// Three threads: one reads the input, one checks (the compile can take seconds on a long evolving song, and a request must
/// not wait for it), and this one answers. A change is answered with the parser's diagnostics at once; once the text has
/// rested [`DEBOUNCE`], its check runs on the worker and its diagnostics replace them (a check of a version the document
/// has moved past is dropped). At the end of the input the checks still owed are made before returning.
pub fn serve<R: BufRead + Send + 'static>(r: R, w: &mut impl Write) -> std::io::Result<i32> {
    use std::sync::mpsc;
    let (tx, rx) = mpsc::channel::<Event>();
    let reader_tx = tx.clone();
    std::thread::spawn(move || {
        let mut r = r;
        loop {
            match read_message(&mut r) {
                Ok(Some(body)) => {
                    if reader_tx.send(Event::Message(body)).is_err() {
                        return;
                    }
                }
                _ => {
                    let _ = reader_tx.send(Event::End);
                    return;
                }
            }
        }
    });
    let (job_tx, job_rx) = mpsc::channel::<(String, i64, String)>();
    let worker_tx = tx;
    std::thread::spawn(move || {
        while let Ok(first) = job_rx.recv() {
            // the latest job of each document: an older version is not worth checking
            let mut jobs = vec![first];
            while let Ok(j) = job_rx.try_recv() {
                jobs.retain(|(u, _, _)| *u != j.0);
                jobs.push(j);
            }
            for (uri, version, text) in jobs {
                let c = btwls::analysis::full_check(&text);
                if worker_tx.send(Event::Checked(uri, version, text, c)).is_err() {
                    return;
                }
            }
        }
    });
    let mut server = btwls::Server::new();
    server.deferred = true;
    // the host's registry: its params carry their docs (hover, completion); checks still use the site's snapshot
    server.registry = Some(&djhost::btw_env::RegistryView);
    // (uri, version) sent to the worker
    let mut sent: std::collections::BTreeSet<(String, i64)> = std::collections::BTreeSet::new();
    let mut ended = false;
    loop {
        let ev = if ended {
            match rx.recv() {
                Ok(e) => e,
                Err(_) => break,
            }
        } else if server.stale().iter().any(|(u, v, _)| !sent.contains(&(u.clone(), *v))) {
            match rx.recv_timeout(DEBOUNCE) {
                Ok(e) => e,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    for (u, v, t) in server.stale() {
                        if sent.insert((u.clone(), v)) {
                            let _ = job_tx.send((u, v, t));
                        }
                    }
                    continue;
                }
                Err(_) => break,
            }
        } else {
            match rx.recv() {
                Ok(e) => e,
                Err(_) => break,
            }
        };
        match ev {
            Event::Message(body) => {
                let msg = match JsonValue::parse(&body) {
                    Ok(m) => m,
                    Err(e) => {
                        let err = btwls::json::obj([
                            ("jsonrpc", JsonValue::str("2.0")),
                            ("id", JsonValue::Null),
                            ("error", btwls::json::obj([("code", JsonValue::int(-32700)), ("message", JsonValue::str(format!("parse error: {e}")))])),
                        ]);
                        write_message(w, &err)?;
                        continue;
                    }
                };
                for out in server.handle(&msg) {
                    write_message(w, &out)?;
                }
                if server.exited {
                    return Ok(if server.is_shut_down() { 0 } else { 1 });
                }
            }
            Event::Checked(uri, version, text, check) => {
                sent.remove(&(uri.clone(), version));
                if let Some(publish) = server.install(&uri, version, &text, check) {
                    write_message(w, &publish)?;
                }
                if ended && sent.is_empty() {
                    break;
                }
            }
            Event::End => {
                ended = true;
                // the checks still owed
                for (u, v, t) in server.stale() {
                    if sent.insert((u.clone(), v)) {
                        let _ = job_tx.send((u, v, t));
                    }
                }
                if sent.is_empty() {
                    break;
                }
            }
        }
    }
    Ok(if server.is_shut_down() { 0 } else { 1 })
}

pub fn main() -> i32 {
    let r = std::io::BufReader::new(std::io::stdin());
    let stdout = std::io::stdout();
    let mut w = stdout.lock();
    match serve(r, &mut w) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("btwd lsp: {e}");
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(body: &str) -> String {
        format!("Content-Length: {}\r\n\r\n{body}", body.len())
    }

    #[test]
    fn a_session_over_the_framing() {
        let input = [
            frame(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#),
            frame(r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#),
            frame(r#"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///t.btw","languageId":"btw","version":1,"text":"tempo 12x\n"}}}"#),
            frame(r#"{"jsonrpc":"2.0","id":2,"method":"shutdown"}"#),
            frame(r#"{"jsonrpc":"2.0","method":"exit"}"#),
        ]
        .concat();
        let mut out = Vec::new();
        let code = serve(std::io::Cursor::new(input.into_bytes()), &mut out).unwrap();
        assert_eq!(code, 0);
        let out = String::from_utf8(out).unwrap();
        let mut r = std::io::Cursor::new(out.into_bytes());
        let init = JsonValue::parse(&read_message(&mut r).unwrap().unwrap()).unwrap();
        assert_eq!(init.get("id").and_then(JsonValue::as_f64), Some(1.0));
        assert!(init.get("result").and_then(|r| r.get("capabilities")).is_some());
        let diags = JsonValue::parse(&read_message(&mut r).unwrap().unwrap()).unwrap();
        assert_eq!(diags.get("method").and_then(JsonValue::as_str), Some("textDocument/publishDiagnostics"));
        let list = diags.get("params").and_then(|p| p.get("diagnostics")).and_then(JsonValue::as_array).unwrap();
        assert!(!list.is_empty(), "{}", diags.js_stringified());
        let shut = JsonValue::parse(&read_message(&mut r).unwrap().unwrap()).unwrap();
        assert_eq!(shut.get("result"), Some(&JsonValue::Null));
        assert!(read_message(&mut r).unwrap().is_none());
    }

    #[test]
    fn the_check_is_published_after_the_parse() {
        // `inst=nope` is no voice: only the compile says so; the parser finds nothing
        let input = [
            frame(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#),
            frame(r#"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///t.btw","languageId":"btw","version":1,"text":"deck A: bass a1 inst=nopenope\n"}}}"#),
        ]
        .concat();
        let mut out = Vec::new();
        serve(std::io::Cursor::new(input.into_bytes()), &mut out).unwrap();
        let mut r = std::io::Cursor::new(out);
        let mut published = Vec::new();
        while let Some(m) = read_message(&mut r).unwrap() {
            let v = JsonValue::parse(&m).unwrap();
            if v.get("method").and_then(JsonValue::as_str) == Some("textDocument/publishDiagnostics") {
                published.push(v.get("params").and_then(|p| p.get("diagnostics")).and_then(JsonValue::as_array).map_or(0, Vec::len));
            }
        }
        assert_eq!(published.len(), 2, "the parser's, then the check's");
        assert_eq!(published[0], 0);
        assert!(published[1] > 0);
    }
}
