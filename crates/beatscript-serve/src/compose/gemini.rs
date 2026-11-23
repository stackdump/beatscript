//! Go `compose/gemini.go`: `generateContent` over HTTPS (ureq + rustls, blocking: a compose call runs on a blocking
//! thread). The key goes in the `x-goog-api-key` header only, never follows a redirect, and every error text is
//! redacted. One retry of a 500/502/503/504 (those are not billed) after 800 ms; 90 s per call.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use super::model::*;

pub const DEFAULT_MODEL: &str = "gemini-3.8-flash";
pub const DEFAULT_ENDPOINT: &str = "https://generativelanguage.googleapis.com";
pub const DEFAULT_MAX_OUTPUT_TOKENS: i64 = 16384;
pub const DEFAULT_THINKING: &str = "LOW";

/// `NormalizeThinking`.
pub fn normalize_thinking(s: &str) -> Result<String, String> {
    let s = s.trim().to_uppercase();
    if s.is_empty() {
        return Ok(DEFAULT_THINKING.into());
    }
    if !matches!(s.as_str(), "MINIMAL" | "LOW" | "MEDIUM" | "HIGH") {
        return Err(format!("thinking level {s:?} is not one of MINIMAL, LOW, MEDIUM, HIGH"));
    }
    Ok(s)
}

pub struct Gemini {
    key: String,
    model: String,
    endpoint: String,
    max_out: i64,
    think: String,
    timeout: Duration,
    retry: Duration,
    red: Arc<Redactor>,
    agent: ureq::Agent,
}

fn model_id_ok(m: &str) -> bool {
    let b = m.as_bytes();
    !b.is_empty() && b[0].is_ascii_alphanumeric() && b.iter().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
}

impl Gemini {
    pub fn new(key: &str, model: &str, endpoint: &str, max_out: i64, thinking: &str, red: Arc<Redactor>) -> Result<Gemini, String> {
        if key.trim().is_empty() {
            return Err("gemini: no API key".into());
        }
        let mut m = model.trim().trim_start_matches("models/").to_string();
        if let Some(x) = model.trim().strip_prefix("models/") {
            m = x.to_string();
        }
        if m.is_empty() {
            m = DEFAULT_MODEL.into();
        }
        if !model_id_ok(&m) {
            return Err(format!("gemini: model id {m:?} is not a plain id like {DEFAULT_MODEL}"));
        }
        let mut ep = endpoint.trim().trim_end_matches('/').to_string();
        if ep.is_empty() {
            ep = DEFAULT_ENDPOINT.into();
        }
        let host_ok = ep.strip_prefix("https://").or_else(|| ep.strip_prefix("http://")).is_some_and(|r| !r.is_empty() && !r.starts_with('/'));
        if !host_ok {
            return Err("gemini: endpoint must be an http(s) URL".into());
        }
        let think = normalize_thinking(thinking).map_err(|e| format!("gemini: {e}"))?;
        let timeout = Duration::from_secs(90);
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).max_redirects(0).timeout_global(Some(timeout)).build().into();
        Ok(Gemini {
            key: key.trim().into(),
            model: m,
            endpoint: ep,
            max_out: if max_out <= 0 { DEFAULT_MAX_OUTPUT_TOKENS } else { max_out },
            think,
            timeout,
            retry: Duration::from_millis(800),
            red,
            agent,
        })
    }

    pub fn model_name(&self) -> &str {
        &self.model
    }

    fn post(&self, url: &str, body: &[u8]) -> Result<(u16, Option<String>, Vec<u8>), ModelError> {
        let r = self
            .agent
            .post(url)
            .header("Content-Type", "application/json")
            .header("x-goog-api-key", &self.key)
            .header("Accept", "application/json")
            .send(body);
        match r {
            Err(e) => {
                let timed_out = matches!(e, ureq::Error::Timeout(_));
                let msg = if timed_out { "the model service timed out" } else { "the model service did not answer" };
                Err(ModelError { kind: KIND_TRANSPORT.into(), aborted: timed_out, message: format!("{msg}: {}", clip(&self.red.redact(&e.to_string()), 160)), ..Default::default() })
            }
            Ok(mut resp) => {
                let status = resp.status().as_u16();
                let ra = resp.headers().get("retry-after").and_then(|v| v.to_str().ok()).map(str::to_string);
                match resp.body_mut().with_config().limit(4 << 20).read_to_vec() {
                    Ok(b) => Ok((status, ra, b)),
                    Err(e) => Err(ModelError { kind: KIND_TRANSPORT.into(), aborted: true, message: format!("reading the reply failed: {}", clip(&self.red.redact(&e.to_string()), 160)), ..Default::default() }),
                }
            }
        }
    }

    fn http_error(&self, status: u16, retry_after: Option<&str>, raw: &[u8]) -> ModelError {
        let mut me = ModelError { kind: KIND_UPSTREAM.into(), status, ..Default::default() };
        if let Ok(v) = serde_json::from_slice::<Value>(raw) {
            if v.is_object() && (v["error"].is_object() || v["error"].is_null()) {
                me.code = sanitize_word(v["error"]["status"].as_str().unwrap_or(""));
                me.message = clip(&self.red.redact(v["error"]["message"].as_str().unwrap_or("")), 200);
            }
        }
        match status {
            429 => {
                me.kind = KIND_RATE_LIMITED.into();
                me.retry_after = 30;
                if let Some(n) = retry_after.and_then(|s| s.trim().parse::<u64>().ok()).filter(|n| (1..=3600).contains(n)) {
                    me.retry_after = n;
                }
            }
            401 | 403 => me.kind = KIND_AUTH.into(),
            _ => {}
        }
        me
    }
}

impl Model for Gemini {
    fn generate(&self, system: &str, messages: &[Message]) -> (String, Usage, Option<ModelError>) {
        let mut rq = serde_json::Map::new();
        if !system.is_empty() {
            rq.insert("systemInstruction".into(), json!({"parts": [{"text": system}]}));
        }
        let contents: Vec<Value> = messages.iter().map(|m| json!({"role": if m.role == "model" { "model" } else { "user" }, "parts": [{"text": m.text}]})).collect();
        rq.insert("contents".into(), Value::Array(contents));
        rq.insert("generationConfig".into(), json!({"maxOutputTokens": self.max_out, "thinkingConfig": {"thinkingLevel": self.think}}));
        let body = super::gojson::to_string(&Value::Object(rq));
        let url = format!("{}/v1beta/models/{}:generateContent", self.endpoint, self.model);
        let start = std::time::Instant::now();
        let mut try_ = 0;
        let (status, ra, raw) = loop {
            let r = match self.post(&url, body.as_bytes()) {
                Ok(r) => r,
                Err(e) => return (String::new(), Usage::default(), Some(e)),
            };
            if try_ == 0 && matches!(r.0, 500 | 502 | 503 | 504) && start.elapsed() + self.retry < self.timeout {
                try_ += 1;
                std::thread::sleep(self.retry);
                continue;
            }
            break r;
        };
        if status / 100 != 2 {
            return (String::new(), Usage::default(), Some(self.http_error(status, ra.as_deref(), &raw)));
        }
        let Ok(out) = serde_json::from_slice::<Value>(&raw).map_err(|_| ()).and_then(|v| if v.is_object() { Ok(v) } else { Err(()) }) else {
            return (String::new(), Usage::default(), Some(ModelError { kind: KIND_BAD_RESPONSE.into(), status, message: "the reply was not JSON of the expected shape".into(), ..Default::default() }));
        };
        let um = &out["usageMetadata"];
        let g = |k: &str| um[k].as_i64().unwrap_or(0);
        let mut use_ = Usage { tin: g("promptTokenCount"), thoughts: g("thoughtsTokenCount"), tout: g("candidatesTokenCount") + g("thoughtsTokenCount") };
        if g("totalTokenCount") - g("promptTokenCount") > use_.tout {
            use_.tout = g("totalTokenCount") - g("promptTokenCount");
        }
        let cands = out["candidates"].as_array().cloned().unwrap_or_default();
        if cands.is_empty() {
            if let Some(br) = out["promptFeedback"]["blockReason"].as_str().filter(|s| !s.is_empty()) {
                return (String::new(), use_, Some(ModelError { kind: KIND_BLOCKED.into(), status, message: format!("the model declined the prompt (blockReason {})", sanitize_word(br)), ..Default::default() }));
            }
            return (String::new(), use_, Some(ModelError { kind: KIND_BAD_RESPONSE.into(), status, message: "the reply had no candidate".into(), ..Default::default() }));
        }
        let c = &cands[0];
        let mut text = String::new();
        for p in c["content"]["parts"].as_array().into_iter().flatten() {
            if p["thought"] == true {
                continue;
            }
            text.push_str(p["text"].as_str().unwrap_or(""));
        }
        match c["finishReason"].as_str().unwrap_or("") {
            "" | "STOP" | "FINISH_REASON_UNSPECIFIED" => (text, use_, None),
            "MAX_TOKENS" => {
                let m = format!("the reply was cut off at the output limit ({} tokens, thinking included)", self.max_out);
                (text, use_, Some(ModelError { kind: KIND_MAX_TOKENS.into(), status, message: m, ..Default::default() }))
            }
            fr => (String::new(), use_, Some(ModelError { kind: KIND_BLOCKED.into(), status, message: format!("the model stopped without an answer (finishReason {})", sanitize_word(fr)), ..Default::default() })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// A one-shot HTTP server on loopback that answers `reply` and hands back what it was sent.
    fn serve(reply: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let ln = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = format!("http://{}", ln.local_addr().unwrap());
        let h = std::thread::spawn(move || {
            let (mut s, _) = ln.accept().unwrap();
            let mut buf = vec![0u8; 65536];
            let mut got = Vec::new();
            loop {
                let n = s.read(&mut buf).unwrap();
                got.extend_from_slice(&buf[..n]);
                let t = String::from_utf8_lossy(&got).to_string();
                if let Some(i) = t.find("\r\n\r\n") {
                    let cl: usize = t.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length: ").map(|v| v.trim().parse().unwrap())).unwrap_or(0);
                    if got.len() >= i + 4 + cl {
                        break;
                    }
                }
            }
            s.write_all(reply.as_bytes()).unwrap();
            String::from_utf8_lossy(&got).to_string()
        });
        (addr, h)
    }

    #[test]
    fn generate_reads_text_and_usage() {
        let body = r#"{"candidates":[{"content":{"parts":[{"text":"think","thought":true},{"text":"tempo 120\n"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":5,"thoughtsTokenCount":3,"totalTokenCount":20}}"#;
        let reply: &'static str = Box::leak(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).into_boxed_str());
        let (addr, h) = serve(reply);
        let g = Gemini::new("AIzaSyTESTKEY0123456789abcdef", "gemini-x", &addr, 100, "", Arc::new(Redactor::new(&[]))).unwrap();
        let (t, u, e) = g.generate("sys", &[Message { role: "user", text: "hi".into() }]);
        assert!(e.is_none(), "{e:?}");
        assert_eq!(t, "tempo 120\n");
        assert_eq!((u.tin, u.tout, u.thoughts), (10, 10, 3));
        let req = h.join().unwrap();
        assert!(req.starts_with("POST /v1beta/models/gemini-x:generateContent"));
        assert!(req.to_ascii_lowercase().contains("x-goog-api-key: aizasytestkey0123456789abcdef"));
        assert!(req.contains(r#""generationConfig":{"maxOutputTokens":100,"thinkingConfig":{"thinkingLevel":"LOW"}}"#));
    }

    #[test]
    fn rate_limited_and_redacted() {
        let body = r#"{"error":{"code":429,"message":"quota for key AIzaSyTESTKEY0123456789abcdef","status":"RESOURCE_EXHAUSTED"}}"#;
        let reply: &'static str = Box::leak(format!("HTTP/1.1 429 Too Many Requests\r\nRetry-After: 12\r\nContent-Length: {}\r\n\r\n{body}", body.len()).into_boxed_str());
        let (addr, _h) = serve(reply);
        let red = Arc::new(Redactor::new(&["AIzaSyTESTKEY0123456789abcdef"]));
        let g = Gemini::new("AIzaSyTESTKEY0123456789abcdef", "", &addr, 0, "high", red).unwrap();
        let (_, _, e) = g.generate("", &[]);
        let e = e.unwrap();
        assert_eq!((e.kind.as_str(), e.retry_after, e.code.as_str()), ("rate_limited", 12, "RESOURCE_EXHAUSTED"));
        assert_eq!(e.message, "quota for key [redacted]");
    }
}
