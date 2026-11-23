//! Go `compose/model.go`, `fake.go`: the text model compose talks to (Gemini, or the recorded-fixture fake), its
//! errors, and the redactor that keeps the key out of every error text and log line.

use serde_json::Value;

#[derive(Clone, Debug)]
pub struct Message {
    /// "user" | "model"
    pub role: &'static str,
    pub text: String,
}

/// What one call used. `out` is candidates + thinking.
#[derive(Clone, Copy, Debug, Default)]
pub struct Usage {
    pub tin: i64,
    pub tout: i64,
    pub thoughts: i64,
}

pub const KIND_UPSTREAM: &str = "upstream";
pub const KIND_RATE_LIMITED: &str = "rate_limited";
pub const KIND_AUTH: &str = "auth";
pub const KIND_TRANSPORT: &str = "transport";
pub const KIND_BLOCKED: &str = "blocked";
pub const KIND_MAX_TOKENS: &str = "max_tokens";
pub const KIND_BAD_RESPONSE: &str = "bad_response";

#[derive(Clone, Debug, Default)]
pub struct ModelError {
    pub kind: String,
    pub status: u16,
    pub code: String,
    pub message: String,
    pub retry_after: u64,
    pub aborted: bool,
}

impl ModelError {
    pub fn new(kind: &str, message: impl Into<String>) -> ModelError {
        ModelError { kind: kind.into(), message: message.into(), ..Default::default() }
    }
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "model: {}", self.kind)?;
        if self.status != 0 {
            write!(f, " (HTTP {}", self.status)?;
            if !self.code.is_empty() {
                write!(f, " {}", self.code)?;
            }
            write!(f, ")")?;
        }
        if !self.message.is_empty() {
            write!(f, ": {}", self.message)?;
        }
        Ok(())
    }
}

/// A model: the reply text, what it used, and an error (usage is filled in also on an error when known).
pub trait Model: Send + Sync {
    fn generate(&self, system: &str, messages: &[Message]) -> (String, Usage, Option<ModelError>);
}

// ---------- redaction ----------

pub struct Redactor {
    secrets: Vec<String>,
}

fn url_escape(s: &str) -> String {
    let mut b = String::new();
    for c in s.bytes() {
        if c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.' | b'~') {
            b.push(c as char);
        } else {
            b.push_str(&format!("%{c:02X}"));
        }
    }
    b
}

impl Redactor {
    pub fn new(secrets: &[&str]) -> Redactor {
        Redactor { secrets: secrets.iter().map(|s| s.trim().to_string()).filter(|s| s.len() >= 6).collect() }
    }

    /// `Redact`: the known secrets (and their URL escapes), any `key=` query parameter, any Google-key-shaped string.
    pub fn redact(&self, s: &str) -> String {
        let mut s = s.to_string();
        for k in &self.secrets {
            s = s.replace(k.as_str(), "[redacted]");
            let esc = url_escape(k);
            if esc != *k {
                s = s.replace(&esc, "[redacted]");
            }
        }
        google_key(&key_param(&s))
    }
}

/// `(?i)([?&]key=)[^&\s"']+` → `${1}[redacted]`
fn key_param(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        if (b[i] == b'?' || b[i] == b'&') && i + 4 < b.len() + 1 && b.len() >= i + 5 && b[i + 1..i + 5].eq_ignore_ascii_case(b"key=") {
            let mut j = i + 5;
            while j < b.len() && !matches!(b[j], b'&' | b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' | b'"' | b'\'') {
                j += 1;
            }
            if j > i + 5 {
                out.push_str(&s[i..i + 5]);
                out.push_str("[redacted]");
                i = j;
                continue;
            }
        }
        let c = s[i..].chars().next().unwrap();
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// `AIza[0-9A-Za-z_\-]{20,}` → `[redacted]`
fn google_key(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        if b[i..].starts_with(b"AIza") {
            let mut j = i + 4;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_' || b[j] == b'-') {
                j += 1;
            }
            if j - (i + 4) >= 20 {
                out.push_str("[redacted]");
                i = j;
                continue;
            }
        }
        let c = s[i..].chars().next().unwrap();
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// `clip`: whitespace runs to one space, at most `n` runes with an ellipsis.
pub fn clip(s: &str, n: usize) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let r: Vec<char> = s.chars().collect();
    if r.len() <= n {
        return s;
    }
    r[..n].iter().collect::<String>() + "…"
}

/// `sanitizeWord`: an enum-like word and nothing else, at most 40 bytes.
pub fn sanitize_word(s: &str) -> String {
    let w: String = s.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
    w.chars().take(40).collect()
}

// ---------- the recorded-fixture model (BTW_MODEL=fake) ----------

const FIXTURES: &[&str] = &[
    include_str!("fixtures/blocked.json"),
    include_str!("fixtures/good.json"),
    include_str!("fixtures/max_tokens.json"),
    include_str!("fixtures/model_error.json"),
    include_str!("fixtures/oversize.json"),
    include_str!("fixtures/refine.json"),
    include_str!("fixtures/refine_worse.json"),
    include_str!("fixtures/repair.json"),
    include_str!("fixtures/unrepairable.json"),
];

struct Fixture {
    name: String,
    matches: Vec<String>,
    default: bool,
    steps: Vec<Value>,
}

/// `FakeModel`: the scenario is picked by the first user message (the first fixture by name with a `match` word in
/// it, else the default); step i answers the call with i model turns already in the conversation, the last step every
/// later one. No state between requests.
pub struct FakeModel {
    fixtures: Vec<Fixture>,
}

impl FakeModel {
    pub fn embedded() -> FakeModel {
        Self::from_json(FIXTURES).expect("embedded fixtures")
    }

    pub fn from_json(files: &[&str]) -> Result<FakeModel, String> {
        let mut fx = Vec::new();
        for f in files {
            let v: Value = serde_json::from_str(f).map_err(|e| e.to_string())?;
            let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
            let steps = v["steps"].as_array().cloned().unwrap_or_default();
            if s("name").is_empty() || steps.is_empty() {
                return Err(format!("fake model: fixture {:?} needs a name and at least one step", s("name")));
            }
            fx.push(Fixture {
                name: s("name"),
                matches: v["match"].as_array().map(|a| a.iter().filter_map(|m| m.as_str().map(str::to_string)).collect()).unwrap_or_default(),
                default: v["default"] == true,
                steps,
            });
        }
        fx.sort_by(|a, b| a.name.cmp(&b.name));
        let defs = fx.iter().filter(|f| f.default).count();
        if defs != 1 {
            return Err(format!("fake model: exactly one fixture must be the default, found {defs}"));
        }
        Ok(FakeModel { fixtures: fx })
    }

    fn pick(&self, first: &str) -> &Fixture {
        let low = first.to_lowercase();
        let mut def = None;
        for f in &self.fixtures {
            if f.default {
                def = Some(f);
            }
            if f.matches.iter().any(|m| !m.is_empty() && low.contains(&m.to_lowercase())) {
                return f;
            }
        }
        def.unwrap()
    }
}

impl Model for FakeModel {
    fn generate(&self, system: &str, messages: &[Message]) -> (String, Usage, Option<ModelError>) {
        let (mut first, mut turns, mut in_chars) = ("", 0usize, system.len());
        for m in messages {
            in_chars += m.text.len();
            if m.role == "model" {
                turns += 1;
            } else if first.is_empty() {
                first = &m.text;
            }
        }
        let fx = self.pick(first);
        let st = fx.steps.get(turns).unwrap_or_else(|| fx.steps.last().unwrap());
        let mut text = st["text"].as_str().unwrap_or("").to_string();
        let pad = st["pad_to_bytes"].as_u64().unwrap_or(0) as usize;
        if pad > text.len() {
            if !text.ends_with('\n') {
                text.push('\n');
            }
            while text.len() < pad {
                text.push_str("# padding to make this reply longer than a program may be\n");
            }
        }
        let mut use_ = Usage { tin: (in_chars / 3) as i64, tout: (text.len() / 3) as i64, thoughts: 0 };
        if let Some(u) = st.get("usage").filter(|u| u.is_object()) {
            use_ = Usage { tin: u["in"].as_i64().unwrap_or(0), tout: u["out"].as_i64().unwrap_or(0), thoughts: u["thoughts"].as_i64().unwrap_or(0) };
        }
        if let Some(e) = st.get("error").filter(|e| e.is_object()) {
            let kind = e["kind"].as_str().filter(|k| !k.is_empty()).unwrap_or(KIND_UPSTREAM);
            return (
                String::new(),
                Usage::default(),
                Some(ModelError {
                    kind: kind.into(),
                    status: e["status"].as_u64().unwrap_or(0) as u16,
                    code: e["code"].as_str().unwrap_or("").into(),
                    message: e["message"].as_str().unwrap_or("").into(),
                    retry_after: e["retry_after"].as_u64().unwrap_or(0),
                    aborted: false,
                }),
            );
        }
        if let Some(b) = st["block"].as_str().filter(|b| !b.is_empty()) {
            return (String::new(), use_, Some(ModelError::new(KIND_BLOCKED, format!("the model declined the prompt (blockReason {})", sanitize_word(b)))));
        }
        if st["finish"] == "MAX_TOKENS" {
            return (text, use_, Some(ModelError::new(KIND_MAX_TOKENS, "the reply was cut off at the output limit")));
        }
        (text, use_, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts() {
        let r = Redactor::new(&["sekrit-key-123"]);
        assert_eq!(r.redact("x sekrit-key-123 y"), "x [redacted] y");
        assert_eq!(r.redact("GET /v1?key=abc&x=1"), "GET /v1?key=[redacted]&x=1");
        assert_eq!(r.redact("k AIzaSyA0123456789abcdefghijk end"), "k [redacted] end");
    }

    #[test]
    fn fake_picks_and_steps() {
        let f = FakeModel::embedded();
        let u = |t: &str| Message { role: "user", text: t.into() };
        let (t, us, e) = f.generate("sys", &[u("hello")]);
        assert!(e.is_none() && t.starts_with("# dark techno"));
        assert_eq!(us.tin, 2310);
        let (_, _, e) = f.generate("sys", &[u("please fake-error now")]);
        assert_eq!(e.unwrap().status, 503);
        let (t, _, e) = f.generate("sys", &[u("fake-oversize")]);
        assert!(e.is_none() && t.len() >= 20000);
    }
}
