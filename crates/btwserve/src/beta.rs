//! The Android beta sign-up: a small list of people who asked to join the BeatScript Android beta.
//!
//! Off unless `-android-beta-url` (or `BTW_ANDROID_BETA_URL`) is set; the value is the Google Play opt-in page
//! (closed test, reached through a Google Group) and it is shown only to someone the owner has marked added. A
//! person gives the Google account email used on their Android phone, an optional name and a consent tick; the row
//! is `{email, name, created, status}` with status `pending`, `added` or `removed`, kept in `<data>/beta-signups.json`
//! (mode 0600, written whole through a temp file and a rename).
//!
//! - `POST /api/beta/signup {email, name, consent}`   stores or refreshes a request; the answer never says whether the
//!   address was already there. Per address 5 an hour, and the list holds at most `MAX_ROWS`.
//! - `POST /api/beta/status {email}`                  `{added, url?}`: the link only for an added address.
//! - `POST /api/beta/remove {email}`                  the same email takes itself off: status `removed`, the name
//!   cleared; the owner erases removed rows (`purge`) once the address is out of the Google Group.
//! - `GET|POST /api/admin/beta` (the admin token, in compose.rs) and the `beta_signups` MCP tool (owners only,
//!   `-beta-owners uid,uid`) list, mark added, remove, purge.
//!
//! No email is sent and no third party is called.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, Method, Response};
use axum::routing::any;
use axum::Router;
use serde_json::{json, Value};

use crate::mcp::limiter::Limiter;
use crate::{ApiError, AppState};

const MAX_ROWS: usize = 5000;
const MAX_BODY: usize = 2048;
const MAX_EMAIL: usize = 254;
const MAX_NAME: usize = 80;

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub email: String,
    pub name: String,
    pub created: String,
    pub status: String,
}

impl Row {
    pub fn json(&self) -> Value {
        json!({"email": self.email, "name": self.name, "created": self.created, "status": self.status})
    }
}

pub struct Beta {
    url: String,
    owners: Vec<String>,
    path: PathBuf,
    rows: Mutex<Vec<Row>>,
    sign_ip: Limiter,
    ask_ip: Limiter,
}

/// The one instance, made on first use from the flags.
pub fn get(app: &AppState) -> Arc<Beta> {
    app.beta.get_or_init(|| Arc::new(Beta::new(&app.cfg.flag("android-beta-url", ""), &app.cfg.flag("beta-owners", ""), app.cfg.data_dir().join("beta-signups.json")))).clone()
}

fn now_rfc3339() -> String {
    let s = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let (days, rem) = (s.div_euclid(86400), s.rem_euclid(86400));
    // civil from days (Howard Hinnant)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Lower-cased and trimmed, or None when it is not a plausible address (one `@`, a dotted domain, no spaces or controls).
pub fn clean_email(raw: &str) -> Option<String> {
    let e = raw.trim().to_lowercase();
    if e.is_empty() || e.len() > MAX_EMAIL || e.chars().any(|c| c.is_whitespace() || c.is_control() || c == ',' || c == ';' || c == '<' || c == '>' || c == '"') {
        return None;
    }
    let (local, domain) = e.split_once('@')?;
    if local.is_empty() || local.len() > 64 || domain.contains('@') || !domain.contains('.') || domain.starts_with('.') || domain.ends_with('.') || domain.contains("..") {
        return None;
    }
    Some(e)
}

fn clean_name(raw: &str) -> String {
    let s: String = raw.chars().filter(|c| !c.is_control()).collect();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    match s.char_indices().nth(MAX_NAME) {
        Some((i, _)) => s[..i].to_string(),
        None => s,
    }
}

impl Beta {
    pub fn new(url: &str, owners: &str, path: PathBuf) -> Beta {
        let rows = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| v["signups"].as_array().cloned())
            .unwrap_or_default()
            .iter()
            .map(|r| Row { email: r["email"].as_str().unwrap_or("").into(), name: r["name"].as_str().unwrap_or("").into(), created: r["created"].as_str().unwrap_or("").into(), status: r["status"].as_str().unwrap_or("pending").into() })
            .filter(|r| !r.email.is_empty())
            .collect();
        Beta {
            url: url.trim().to_string(),
            owners: owners.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
            path,
            rows: Mutex::new(rows),
            sign_ip: Limiter::new(5, Duration::from_secs(3600)),
            ask_ip: Limiter::new(30, Duration::from_secs(3600)),
        }
    }

    /// Sign-up is open only when the flag has a value.
    pub fn enabled(&self) -> bool {
        !self.url.is_empty()
    }

    pub fn is_owner(&self, uid: &str) -> bool {
        !uid.is_empty() && self.owners.iter().any(|o| o == uid)
    }

    pub fn has_owners(&self) -> bool {
        !self.owners.is_empty()
    }

    fn save(&self, rows: &[Row]) -> Result<(), String> {
        let v = json!({"signups": rows.iter().map(Row::json).collect::<Vec<_>>()});
        let tmp = self.path.with_extension("json.tmp");
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp).map_err(|e| e.to_string())?;
            f.write_all(serde_json::to_string_pretty(&v).unwrap().as_bytes()).map_err(|e| e.to_string())?;
            f.sync_all().map_err(|e| e.to_string())?;
        }
        std::fs::rename(&tmp, &self.path).map_err(|e| e.to_string())
    }

    /// A request: new, or an address already there (a removed one asks again and is pending again; an added one
    /// stays added). The caller is told the same thing either way.
    pub fn signup(&self, email: &str, name: &str) -> Result<(), String> {
        let mut rows = self.rows.lock().unwrap();
        if let Some(r) = rows.iter_mut().find(|r| r.email == email) {
            if r.status == "removed" {
                r.status = "pending".into();
                r.created = now_rfc3339();
            }
            if !name.is_empty() {
                r.name = name.into();
            }
        } else {
            if rows.len() >= MAX_ROWS {
                return Err("full".into());
            }
            rows.push(Row { email: email.into(), name: name.into(), created: now_rfc3339(), status: "pending".into() });
        }
        self.save(&rows)
    }

    pub fn is_added(&self, email: &str) -> bool {
        self.rows.lock().unwrap().iter().any(|r| r.email == email && r.status == "added")
    }

    /// The address takes itself off (or the owner removes it): status removed, the name cleared. False if unknown.
    pub fn remove(&self, email: &str) -> Result<bool, String> {
        let mut rows = self.rows.lock().unwrap();
        let Some(r) = rows.iter_mut().find(|r| r.email == email) else { return Ok(false) };
        r.status = "removed".into();
        r.name.clear();
        self.save(&rows)?;
        Ok(true)
    }

    pub fn mark_added(&self, email: &str) -> Result<bool, String> {
        let mut rows = self.rows.lock().unwrap();
        let Some(r) = rows.iter_mut().find(|r| r.email == email && r.status != "removed") else { return Ok(false) };
        r.status = "added".into();
        self.save(&rows)?;
        Ok(true)
    }

    /// Erase the removed rows for good; how many.
    pub fn purge(&self) -> Result<usize, String> {
        let mut rows = self.rows.lock().unwrap();
        let before = rows.len();
        rows.retain(|r| r.status != "removed");
        let n = before - rows.len();
        if n > 0 {
            self.save(&rows)?;
        }
        Ok(n)
    }

    pub fn list(&self, status: &str) -> Vec<Row> {
        self.rows.lock().unwrap().iter().filter(|r| status.is_empty() || r.status == status).cloned().collect()
    }

    pub fn csv(&self, status: &str) -> String {
        let q = |s: &str| format!("\"{}\"", s.replace('"', "\"\"").trim_start_matches(['=', '+', '-', '@']));
        let mut out = String::from("email,name,created,status\n");
        for r in self.list(status) {
            out += &format!("{},{},{},{}\n", q(&r.email), q(&r.name), r.created, r.status);
        }
        out
    }

    /// The owner's one entry point, shared by the admin endpoint and the MCP tool: `list`, `added` (email), `remove`
    /// (email), `purge`.
    pub fn act(&self, action: &str, email: &str, status: &str) -> Result<Value, ApiError> {
        let need = || clean_email(email).ok_or_else(|| ApiError::new(400, "bad_request", "give the email address"));
        let int = |e: String| ApiError::internal(e);
        match action {
            "" | "list" => {
                let rows = self.list(status);
                let (mut p, mut a, mut r) = (0, 0, 0);
                for x in self.list("") {
                    match x.status.as_str() {
                        "added" => a += 1,
                        "removed" => r += 1,
                        _ => p += 1,
                    }
                }
                Ok(json!({"signups": rows.iter().map(Row::json).collect::<Vec<_>>(), "count": rows.len(), "pending": p, "added": a, "removed": r}))
            }
            "added" | "mark_added" | "add" => {
                let e = need()?;
                if !self.mark_added(&e).map_err(int)? {
                    return Err(ApiError::new(404, "not_found", "no such request (or it was removed)"));
                }
                Ok(json!({"email": e, "status": "added"}))
            }
            "remove" => {
                let e = need()?;
                if !self.remove(&e).map_err(int)? {
                    return Err(ApiError::new(404, "not_found", "no such request"));
                }
                Ok(json!({"email": e, "status": "removed"}))
            }
            "purge" => Ok(json!({"purged": self.purge().map_err(int)?})),
            _ => Err(ApiError::new(400, "bad_request", "action is list, added, remove or purge")),
        }
    }
}

// ---------- routes ----------

fn resp(status: u16, v: Value) -> Response<Body> {
    Response::builder().status(status).header(header::CONTENT_TYPE, "application/json; charset=utf-8").header(header::CACHE_CONTROL, "no-store").body(Body::from(v.to_string() + "\n")).unwrap()
}

fn err(e: ApiError) -> Response<Body> {
    let mut r = resp(e.status, json!({"error": {"code": e.code, "message": e.message}}));
    if e.retry_after > 0 {
        r.headers_mut().insert(header::RETRY_AFTER, e.retry_after.to_string().parse().unwrap());
    }
    r
}

fn ip_of(req: &Request) -> String {
    let peer = req.extensions().get::<crate::Peer>().map_or_else(|| "127.0.0.1:0".parse().unwrap(), |p| p.0);
    crate::client_ip(peer, req.headers())
}

async fn body_json(req: Request) -> Result<Value, ApiError> {
    let b = axum::body::to_bytes(req.into_body(), MAX_BODY).await.map_err(|_| ApiError::new(413, "too_large", format!("request body is over {MAX_BODY} bytes")))?;
    serde_json::from_slice::<Value>(&b).ok().filter(Value::is_object).ok_or_else(|| ApiError::new(400, "bad_request", "the body must be a JSON object"))
}

async fn handle(State(app): State<AppState>, req: Request) -> Response<Body> {
    let b = get(&app);
    let path = req.uri().path().to_string();
    if req.method() != Method::POST {
        return app.site.serve(req.method(), &path, req.headers());
    }
    if !b.enabled() {
        return err(ApiError::new(404, "not_found", "not found"));
    }
    let ip = ip_of(&req);
    let key = format!("ip:{}", ip);
    let body = match body_json(req).await {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    let email = clean_email(body["email"].as_str().unwrap_or(""));
    let Some(email) = email else { return err(ApiError::new(400, "bad_request", "that does not look like an email address")) };
    let wait = |l: &Limiter| l.allow(&key).err();
    match path.as_str() {
        "/api/beta/signup" => {
            if let Some(s) = wait(&b.sign_ip) {
                return err(ApiError::new(429, "rate_limited", format!("too many requests from your address; try again in {s}s")).retry(s));
            }
            if body["consent"] != json!(true) {
                return err(ApiError::new(400, "consent_required", "tick the box to join the beta"));
            }
            let name = clean_name(body["name"].as_str().unwrap_or(""));
            match b.signup(&email, &name) {
                Ok(()) => resp(200, json!({"status": "received", "message": "Thanks. You are on the list: once you are added to the BeatScript Android beta this page will show you the link to join. Your email is used only for this and you can remove it any time."})),
                Err(e) if e == "full" => err(ApiError::new(503, "full", "the beta list is full for now; try again later")),
                Err(e) => err(ApiError::internal(e)),
            }
        }
        "/api/beta/status" => {
            if let Some(s) = wait(&b.ask_ip) {
                return err(ApiError::new(429, "rate_limited", format!("too many requests from your address; try again in {s}s")).retry(s));
            }
            if b.is_added(&email) {
                resp(200, json!({"added": true, "url": b.url}))
            } else {
                resp(200, json!({"added": false}))
            }
        }
        "/api/beta/remove" => {
            if let Some(s) = wait(&b.ask_ip) {
                return err(ApiError::new(429, "rate_limited", format!("too many requests from your address; try again in {s}s")).retry(s));
            }
            match b.remove(&email) {
                Ok(_) => resp(200, json!({"status": "removed", "message": "Done. If this email was on the list it has been taken off."})),
                Err(e) => err(ApiError::internal(e)),
            }
        }
        _ => err(ApiError::new(404, "not_found", "not found")),
    }
}

pub fn routes(app: &AppState) -> Router {
    let _ = get(app);
    Router::new().route("/api/beta/signup", any(handle)).route("/api/beta/status", any(handle)).route("/api/beta/remove", any(handle)).with_state(app.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn beta(dir: &std::path::Path) -> Beta {
        Beta::new("https://play.google.com/apps/testing/app.beatscript", "u1", dir.join("b.json"))
    }

    fn tmp(n: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("btwbeta-{n}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn emails() {
        assert_eq!(clean_email("  Me@Example.COM "), Some("me@example.com".into()));
        for bad in ["", "a", "a@b", "a@@b.c", "@b.c", "a b@c.d", "a@b..c", "a@.c.d", "a@b.c,d@e.f", "<a@b.c>"] {
            assert_eq!(clean_email(bad), None, "{bad}");
        }
    }

    #[test]
    fn lifecycle_and_persistence() {
        let d = tmp("life");
        let b = beta(&d);
        assert!(b.enabled() && b.is_owner("u1") && !b.is_owner("u2") && !b.is_owner(""));
        b.signup("a@x.io", "Ann").unwrap();
        b.signup("a@x.io", "").unwrap(); // dedupe: one row, name kept
        assert_eq!(b.list("").len(), 1);
        assert_eq!(b.list("")[0].name, "Ann");
        assert!(!b.is_added("a@x.io"));
        assert!(b.mark_added("a@x.io").unwrap());
        assert!(b.is_added("a@x.io"));
        // a new process reads the same file
        let b2 = beta(&d);
        assert!(b2.is_added("a@x.io"));
        assert!(b2.remove("a@x.io").unwrap());
        assert!(!b2.is_added("a@x.io") && b2.list("")[0].name.is_empty() && b2.list("")[0].status == "removed");
        assert!(!b2.mark_added("a@x.io").unwrap()); // removed cannot be added
        b2.signup("a@x.io", "Ann B").unwrap(); // asking again is pending again
        assert_eq!(b2.list("pending").len(), 1);
        assert!(b2.remove("a@x.io").unwrap());
        assert_eq!(b2.purge().unwrap(), 1);
        assert!(b2.list("").is_empty());
        assert!(!b2.remove("nobody@x.io").unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(d.join("b.json")).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn off_without_the_flag_and_csv_is_safe() {
        let d = tmp("off");
        assert!(!Beta::new("", "", d.join("b.json")).enabled());
        let b = beta(&d);
        b.signup("a@x.io", "=cmd|' /C calc'!A0").unwrap();
        let c = b.csv("");
        assert!(c.contains("\"cmd|' /C calc'!A0\"") && !c.contains("\"="), "{c}");
        assert_eq!(b.act("list", "", "").unwrap()["pending"], 1);
        assert!(b.act("nope", "", "").is_err());
        assert!(b.act("added", "", "").is_err());
    }

    #[test]
    fn names_are_clipped_and_clean() {
        assert_eq!(clean_name("  A \n\t B\u{7} "), "A B");
        assert_eq!(clean_name(&"é".repeat(200)).chars().count(), 80);
    }

    #[test]
    fn rfc3339_shape() {
        let t = now_rfc3339();
        assert_eq!(t.len(), 20);
        assert!(t.starts_with("20") && t.ends_with('Z'));
    }
}
