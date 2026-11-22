//! The query path (ontology-bitwrap-io `guard.go`, `limits.go`, `sparql.go`, `engine.go`): guard, gate, engine, caps.
//!
//! The Go server runs the open-ontologies engine (MIT) as a child process, and that engine is Oxigraph 0.5 over an
//! in-memory store loaded from the pack's N-Triples. Here Oxigraph (MIT OR Apache-2.0) runs in-process on the same
//! graph, so the same query gets the same evaluator. What differs is the timeout: the engine could not cancel a query,
//! so Go killed and restarted it; Oxigraph takes a cancellation token, so a query past its five seconds is cancelled
//! and nothing restarts. The client still earns the same strike, and two in ten minutes the same ban.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use oxigraph::io::{RdfFormat, RdfParser};
use oxigraph::model::Term as OxTerm;
use oxigraph::sparql::{CancellationToken, QueryResults, SparqlEvaluator};
use oxigraph::store::Store;
use regex::Regex;
use serde_json::{json, Value};
use std::sync::LazyLock;

pub const MAX_QUERY_BYTES: usize = 8 << 10;
pub const MAX_ROWS: usize = 1000;
pub const QUERY_TIMEOUT: Duration = Duration::from_secs(5);
const BAN_AFTER: usize = 2;
const BAN_WINDOW: Duration = Duration::from_secs(600);

static RE_LINE_COMMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^\s*#.*$").unwrap());
static RE_STRINGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'"#).unwrap());
static RE_IRIS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"<[^<>"\s]*>"#).unwrap());
static RE_VARS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[?$][A-Za-z0-9_]+").unwrap());
static RE_PNAMES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[A-Za-z_][A-Za-z0-9_.-]*:[A-Za-z0-9_.:%-]*|:[A-Za-z0-9_.:%-]*").unwrap());
static RE_FORBIDDEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(service|from|insert|delete|load|clear|drop|create|copy|move|add|with|using)\b").unwrap());
static RE_PROLOGUE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^\s*(?:prefix\s+[A-Za-z0-9_.-]*:\s*<[^>]*>|base\s*<[^>]*>)").unwrap());

/// `queryForm`: SELECT, ASK, CONSTRUCT or DESCRIBE, and nothing else; SERVICE and FROM are refused because they make
/// an engine fetch a URL the caller names.
pub fn query_form(q: &str) -> Result<String, String> {
    if q.len() > MAX_QUERY_BYTES {
        return Err(format!("query longer than {MAX_QUERY_BYTES} bytes"));
    }
    let mut body = RE_LINE_COMMENT.replace_all(q, "").into_owned();
    while let Some(m) = RE_PROLOGUE.find(&body) {
        body = body[m.end()..].to_string();
    }
    let Some(first) = body.split_whitespace().next() else { return Err("empty query".into()) };
    let form = first.to_uppercase();
    if !matches!(form.as_str(), "SELECT" | "ASK" | "CONSTRUCT" | "DESCRIBE") {
        return Err("only SELECT, ASK, CONSTRUCT and DESCRIBE are allowed".into());
    }
    let bare = RE_STRINGS.replace_all(&body, "\"\"");
    let bare = RE_IRIS.replace_all(&bare, "<>");
    let bare = RE_VARS.replace_all(&bare, "?v");
    let bare = RE_PNAMES.replace_all(&bare, "p:n");
    if let Some(m) = RE_FORBIDDEN.find(&bare) {
        return Err(format!("\"{}\" is not allowed in a public query", m.as_str().to_uppercase()));
    }
    Ok(form)
}

/// One evaluated query: every value is its N-Triples form, as the engine's `/api/query` returns it.
pub enum Answer {
    Select { vars: Vec<String>, rows: Vec<Vec<(String, String)>> },
    Ask(bool),
    Graph(Vec<[String; 3]>),
}

pub struct Result_ {
    pub answer: Answer,
    pub truncated: bool,
}

#[derive(Debug)]
pub enum QueryError {
    User(String),
    Busy,
    Unavailable,
}

impl QueryError {
    pub fn status(&self) -> u16 {
        match self {
            QueryError::User(_) => 400,
            _ => 503,
        }
    }
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueryError::User(m) => f.write_str(m),
            QueryError::Busy => f.write_str("too many queries in flight; retry shortly"),
            QueryError::Unavailable => f.write_str("query service unavailable"),
        }
    }
}

/// A per-client token bucket (`limiter`): `burst` tokens, one back every `every`.
struct Limiter {
    buckets: HashMap<String, (f64, Instant)>,
    burst: f64,
    every: Duration,
}

impl Limiter {
    fn allow(&mut self, key: &str, now: Instant) -> bool {
        if !self.buckets.contains_key(key) && self.buckets.len() > 10000 {
            self.buckets.clear(); // forget idle clients rather than grow forever
        }
        let b = self.buckets.entry(key.to_string()).or_insert((self.burst, now));
        b.0 = (b.0 + now.duration_since(b.1).as_secs_f64() / self.every.as_secs_f64()).min(self.burst);
        b.1 = now;
        if b.0 < 1.0 {
            return false;
        }
        b.0 -= 1.0;
        true
    }
}

#[derive(Default)]
struct Bans {
    strikes: HashMap<String, Vec<Instant>>,
    banned: HashMap<String, Instant>,
}

/// Where queries run. In-process is for tests; production runs [`Workers`]: Oxigraph checks its cancellation token
/// only when it reads the store, so a join over results it already holds (a cross product) runs on past any deadline.
/// The Go server met the same engine limit by killing the engine; so do we, with the query in a worker process.
enum Engine {
    InProcess(Store),
    Workers(Workers),
}

/// The query service (`sparqlService`): the engine, the gate, the in-flight bound.
pub struct Service {
    engine: Engine,
    triples: usize,
    lim: Mutex<Limiter>,
    bans: Mutex<Bans>,
    sem: Arc<tokio::sync::Semaphore>,
    pub timeout: Duration,
}

/// The engine's load: the whole document parsed first, then inserted (all or nothing).
fn load_store(nt: &[u8]) -> Result<Store, String> {
    let store = Store::new().map_err(|e| e.to_string())?;
    let quads = RdfParser::from_format(RdfFormat::NTriples).for_reader(nt).collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
    store.extend(quads).map_err(|e| e.to_string())?;
    Ok(store)
}

/// Queries in flight at once (Go: `sem: make(chan struct{}, 2)`), and so the number of workers.
const IN_FLIGHT: usize = 2;

impl Service {
    fn with(engine: Engine, triples: usize) -> Service {
        Service {
            engine,
            triples,
            lim: Mutex::new(Limiter { buckets: HashMap::new(), burst: 20.0, every: Duration::from_secs(3) }),
            bans: Mutex::new(Bans::default()),
            sem: Arc::new(tokio::sync::Semaphore::new(IN_FLIGHT)),
            timeout: QUERY_TIMEOUT,
        }
    }

    /// Queries evaluated in this process (tests; a runaway query is cut off only where Oxigraph checks its token).
    pub fn new(nt: &[u8]) -> Result<Service, String> {
        let store = load_store(nt)?;
        let n = store.len().unwrap_or(0);
        Ok(Service::with(Engine::InProcess(store), n))
    }

    /// Queries evaluated in worker processes (`cmd` started with [`WORKER_ENV`] set), killed and started again when one
    /// runs past its deadline. Both workers are started now, so a bad binary fails at startup, not on a query.
    pub fn with_workers(nt: &[u8], cmd: WorkerCmd) -> Result<Service, String> {
        let n = load_store(nt)?.len().unwrap_or(0);
        let w = Workers::new(cmd);
        for i in 0..IN_FLIGHT {
            let mut slot = w.slots[i].lock().unwrap();
            *slot = Some(w.spawn()?);
        }
        Ok(Service::with(Engine::Workers(w), n))
    }

    pub fn len(&self) -> usize {
        self.triples
    }

    /// `gate`: Err(seconds to wait) when the client is banned or over its rate.
    pub fn gate(&self, ip: &str) -> Result<(), Duration> {
        let now = Instant::now();
        {
            let mut b = self.bans.lock().unwrap();
            if let Some(&until) = b.banned.get(ip) {
                if now < until {
                    return Err(until - now);
                }
                b.banned.remove(ip);
            }
        }
        if !self.lim.lock().unwrap().allow(ip, now) {
            return Err(Duration::from_secs(3));
        }
        Ok(())
    }

    /// `strike`: a query that timed out; two within the window earn a ban of the window's length.
    fn strike(&self, ip: &str) {
        if ip.is_empty() {
            return;
        }
        let now = Instant::now();
        let mut b = self.bans.lock().unwrap();
        let kept: Vec<Instant> = b.strikes.get(ip).into_iter().flatten().copied().filter(|t| now.duration_since(*t) < BAN_WINDOW).chain([now]).collect();
        if kept.len() >= BAN_AFTER {
            b.banned.insert(ip.to_string(), now + BAN_WINDOW);
            b.strikes.remove(ip);
        } else {
            b.strikes.insert(ip.to_string(), kept);
        }
    }

    /// `run`: shared by /sparql and the MCP tools, so both get the same guard and limits.
    pub async fn run(self: &Arc<Self>, q: &str, ip: &str) -> Result<Result_, QueryError> {
        query_form(q).map_err(QueryError::User)?;
        let permit = match tokio::time::timeout(Duration::from_secs(2), self.sem.clone().acquire_owned()).await {
            Ok(Ok(p)) => p,
            _ => return Err(QueryError::Busy),
        };
        let token = CancellationToken::new();
        let watch = Arc::new(Watch::default());
        let (me, q2, tok, w2) = (self.clone(), q.to_string(), token.clone(), watch.clone());
        let deadline = Instant::now() + self.timeout;
        let job = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            match &me.engine {
                Engine::InProcess(store) => evaluate(store, &q2, tok, deadline),
                Engine::Workers(w) => w.run(&q2, me.timeout, &w2),
            }
        });
        let cancel = {
            let t = self.timeout;
            tokio::spawn(async move {
                tokio::time::sleep(t).await;
                token.cancel();
                // the worker's own deadline did not end it: kill it (Go: restart the engine)
                tokio::time::sleep(Duration::from_millis(250)).await;
                watch.kill();
            })
        };
        let out = job.await.map_err(|_| QueryError::Unavailable)?;
        cancel.abort();
        match out {
            Err(Eval::Timeout) => {
                self.strike(ip);
                Err(QueryError::User(format!("query exceeded the {} limit", go_duration(self.timeout))))
            }
            Err(Eval::Query(m)) => Err(QueryError::User(format!("query error: {m}"))),
            Err(Eval::Unavailable) => Err(QueryError::Unavailable),
            Ok(r) => Ok(r),
        }
    }
}

fn evaluate(store: &Store, q: &str, token: CancellationToken, deadline: Instant) -> Result<Result_, Eval> {
    let prepared = SparqlEvaluator::new().with_cancellation_token(token).parse_query(q).map_err(|e| Eval::Query(e.to_string()))?;
    let late = || Instant::now() >= deadline;
    let err = |e: &dyn std::fmt::Display| if late() { Eval::Timeout } else { Eval::Query(e.to_string()) };
    let results = prepared.on_store(store).execute().map_err(|e| err(&e))?;
    let mut truncated = false;
    let answer = match results {
        QueryResults::Boolean(b) => Answer::Ask(b),
        QueryResults::Solutions(sol) => {
            let vars: Vec<String> = sol.variables().iter().map(|v| v.as_str().to_string()).collect();
            let mut rows = Vec::new();
            for s in sol {
                let s = s.map_err(|e| err(&e))?;
                if rows.len() == MAX_ROWS {
                    truncated = true;
                    break;
                }
                let row: Vec<(String, String)> = vars.iter().filter_map(|v| s.get(v.as_str()).map(|t: &OxTerm| (v.clone(), t.to_string()))).collect();
                rows.push(row);
                if late() {
                    return Err(Eval::Timeout);
                }
            }
            Answer::Select { vars, rows }
        }
        QueryResults::Graph(ts) => {
            let mut out = Vec::new();
            for t in ts {
                let t = t.map_err(|e| err(&e))?;
                if out.len() == MAX_ROWS {
                    truncated = true;
                    break;
                }
                out.push([t.subject.to_string(), t.predicate.to_string(), t.object.to_string()]);
                if late() {
                    return Err(Eval::Timeout);
                }
            }
            Answer::Graph(out)
        }
    };
    Ok(Result_ { answer, truncated })
}

// ---------- the workers ----------

/// Set in a worker's environment: the process answers queries on stdin/stdout and does nothing else.
pub const WORKER_ENV: &str = "BTWSERVE_ONTOLOGY_WORKER";
/// A worker's answers follow this mark, so anything else a process prints (a test harness, which writes a test's name
/// with no newline before running it) is skipped.
const MARK: &str = "\u{1e}ow ";

/// How to start a worker: a program and its arguments (production: `/proc/self/exe`, which still names this binary
/// after a deploy has replaced the file on disk).
#[derive(Clone)]
pub struct WorkerCmd {
    pub program: std::path::PathBuf,
    pub args: Vec<String>,
}

impl WorkerCmd {
    pub fn this_binary() -> WorkerCmd {
        let program = if std::path::Path::new("/proc/self/exe").exists() { "/proc/self/exe".into() } else { std::env::current_exe().unwrap_or_default() };
        WorkerCmd { program, args: Vec::new() }
    }
}

struct Worker {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    stdout: std::io::BufReader<std::process::ChildStdout>,
}

/// One query's link to the worker running it, for the deadline timer.
#[derive(Default)]
struct Watch {
    pid: std::sync::atomic::AtomicU32,
    killed: std::sync::atomic::AtomicBool,
}

impl Watch {
    fn kill(&self) {
        use std::sync::atomic::Ordering::SeqCst;
        let pid = self.pid.load(SeqCst);
        if pid != 0 {
            self.killed.store(true, SeqCst);
            // SAFETY: kill(2) on a pid this process started and has not reaped (the slot holds its Child)
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGKILL);
            }
        }
    }
}

struct Workers {
    cmd: WorkerCmd,
    slots: Vec<Mutex<Option<Worker>>>,
    free: Mutex<Vec<usize>>,
}

impl Workers {
    fn new(cmd: WorkerCmd) -> Workers {
        Workers { cmd, slots: (0..IN_FLIGHT).map(|_| Mutex::new(None)).collect(), free: Mutex::new((0..IN_FLIGHT).collect()) }
    }

    /// Starts a worker on the spawner thread: PR_SET_PDEATHSIG fires when the THREAD that forked exits, so the fork
    /// must come from a thread that lives as long as the process (not a blocking-pool thread, which retires when idle).
    fn spawn(&self) -> Result<Worker, String> {
        type Job = (WorkerCmd, std::sync::mpsc::Sender<Result<Worker, String>>);
        static SPAWNER: std::sync::OnceLock<Mutex<std::sync::mpsc::Sender<Job>>> = std::sync::OnceLock::new();
        let tx = SPAWNER.get_or_init(|| {
            let (tx, rx) = std::sync::mpsc::channel::<Job>();
            std::thread::Builder::new()
                .name("ontology spawner".into())
                .spawn(move || {
                    for (cmd, back) in rx {
                        let _ = back.send(Workers::spawn_here(&cmd));
                    }
                })
                .expect("ontology spawner thread");
            Mutex::new(tx)
        });
        let (back, answer) = std::sync::mpsc::channel();
        tx.lock().unwrap().send((self.cmd.clone(), back)).map_err(|_| "the ontology spawner thread is gone".to_string())?;
        answer.recv().map_err(|_| "the ontology spawner thread is gone".to_string())?
    }

    fn spawn_here(cmd: &WorkerCmd) -> Result<Worker, String> {
        #[cfg(target_os = "linux")]
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};
        let mut c = Command::new(&cmd.program);
        c.args(&cmd.args).env(WORKER_ENV, "1").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());
        // SAFETY: prctl only, between fork and exec: the worker dies with the server. Linux only: PR_SET_PDEATHSIG
        // does not exist elsewhere, so on other platforms (a dev Mac) a worker outlives a server that crashes.
        #[cfg(target_os = "linux")]
        unsafe {
            c.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                Ok(())
            });
        }
        let mut child = c.spawn().map_err(|e| format!("starting the ontology worker {}: {e}", cmd.program.display()))?;
        let stdin = child.stdin.take().unwrap();
        let mut stdout = std::io::BufReader::new(child.stdout.take().unwrap());
        // wait until it has loaded the graph, so no query's deadline pays for the load
        let mut line = String::new();
        loop {
            line.clear();
            match std::io::BufRead::read_line(&mut stdout, &mut line) {
                Ok(0) | Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("the ontology worker {} exited before it was ready", cmd.program.display()));
                }
                Ok(_) if line.contains(MARK) => break,
                Ok(_) => {}
            }
        }
        Ok(Worker { child, stdin, stdout })
    }

    /// One query on a free worker (there is one per permit), started again if it died or was killed.
    fn run(&self, q: &str, timeout: Duration, watch: &Watch) -> Result<Result_, Eval> {
        use std::io::{BufRead, Write};
        use std::sync::atomic::Ordering::SeqCst;
        let i = self.free.lock().unwrap().pop().expect("a free worker for every permit");
        let out = (|| {
            let mut slot = self.slots[i].lock().unwrap();
            if slot.is_none() {
                *slot = Some(self.spawn().map_err(|e| {
                    eprintln!("btwserve: ontology: {e}");
                    Eval::Unavailable
                })?);
            }
            let w = slot.as_mut().unwrap();
            watch.pid.store(w.child.id(), SeqCst);
            let req = json!({"q": q, "timeout_ms": timeout.as_millis() as u64}).to_string() + "\n";
            let mut line = String::new();
            let ok = w.stdin.write_all(req.as_bytes()).and_then(|_| w.stdin.flush()).is_ok() && loop {
                line.clear();
                match w.stdout.read_line(&mut line) {
                    Ok(0) | Err(_) => break false,
                    Ok(_) if line.contains(MARK) => break true,
                    Ok(_) => continue,
                }
            };
            watch.pid.store(0, SeqCst);
            if !ok {
                // killed at the deadline, or it died: reap it and start its replacement now, outside any query's
                // deadline (loading the graph takes a moment), so the next query finds it ready
                if let Some(mut w) = slot.take() {
                    let _ = w.child.kill();
                    let _ = w.child.wait();
                }
                match self.spawn() {
                    Ok(w) => *slot = Some(w),
                    Err(e) => eprintln!("btwserve: ontology: {e}"),
                }
                return Err(if watch.killed.load(SeqCst) { Eval::Timeout } else { Eval::Unavailable });
            }
            decode(&line[line.find(MARK).unwrap() + MARK.len()..])
        })();
        self.free.lock().unwrap().push(i);
        out
    }
}

impl Drop for Workers {
    fn drop(&mut self) {
        for slot in &self.slots {
            if let Some(mut w) = slot.lock().unwrap().take() {
                let _ = w.child.kill();
                let _ = w.child.wait();
            }
        }
    }
}

fn decode(line: &str) -> Result<Result_, Eval> {
    let v: Value = serde_json::from_str(line).map_err(|_| Eval::Unavailable)?;
    if let Some(e) = v.get("err") {
        return Err(match e.as_str() {
            Some("timeout") => Eval::Timeout,
            _ => Eval::Query(v["msg"].as_str().unwrap_or("").to_string()),
        });
    }
    let strs = |x: &Value| x.as_array().into_iter().flatten().map(|s| s.as_str().unwrap_or("").to_string()).collect::<Vec<_>>();
    let answer = match v["kind"].as_str() {
        Some("ask") => Answer::Ask(v["boolean"].as_bool().unwrap_or(false)),
        Some("graph") => Answer::Graph(v["triples"].as_array().into_iter().flatten().map(|t| {
            let t = strs(t);
            [t[0].clone(), t[1].clone(), t[2].clone()]
        }).collect()),
        Some("select") => Answer::Select {
            vars: strs(&v["vars"]),
            rows: v["rows"].as_array().into_iter().flatten().map(|r| r.as_array().into_iter().flatten().map(|kv| {
                let kv = strs(kv);
                (kv[0].clone(), kv[1].clone())
            }).collect()).collect(),
        },
        _ => return Err(Eval::Unavailable),
    };
    Ok(Result_ { answer, truncated: v["truncated"].as_bool().unwrap_or(false) })
}

fn encode(r: Result<Result_, Eval>) -> Value {
    match r {
        Err(Eval::Timeout) => json!({"err": "timeout"}),
        Err(Eval::Query(m)) => json!({"err": "query", "msg": m}),
        Err(Eval::Unavailable) => json!({"err": "unavailable"}),
        Ok(r) => {
            let mut v = match r.answer {
                Answer::Ask(b) => json!({"kind": "ask", "boolean": b}),
                Answer::Graph(ts) => json!({"kind": "graph", "triples": ts}),
                Answer::Select { vars, rows } => json!({"kind": "select", "vars": vars, "rows": rows}),
            };
            v["truncated"] = json!(r.truncated);
            v
        }
    }
}

/// A worker's life: load the graph, then one answer line per query line until stdin closes.
pub fn worker_loop(nt: &[u8]) {
    use std::io::{BufRead, Write};
    let store = match load_store(nt) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("btwserve: ontology worker: {e}");
            std::process::exit(1);
        }
    };
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    if writeln!(out, "{MARK}{}", json!({"ready": true})).and_then(|_| out.flush()).is_err() {
        return;
    }
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(req) = serde_json::from_str::<Value>(&line) else { continue };
        let timeout = Duration::from_millis(req["timeout_ms"].as_u64().unwrap_or(5000));
        let token = CancellationToken::new();
        let t2 = token.clone();
        std::thread::spawn(move || {
            std::thread::sleep(timeout);
            t2.cancel();
        });
        let r = evaluate(&store, req["q"].as_str().unwrap_or(""), token, Instant::now() + timeout);
        if writeln!(out, "{MARK}{}", encode(r)).and_then(|_| out.flush()).is_err() {
            break;
        }
    }
}

enum Eval {
    Timeout,
    Query(String),
    Unavailable,
}

/// time.Duration's String for the whole seconds and milliseconds used here: `5s`, `400ms`.
fn go_duration(d: Duration) -> String {
    if d.subsec_millis() == 0 && d.as_secs() > 0 {
        format!("{}s", d.as_secs())
    } else {
        format!("{}ms", d.as_millis())
    }
}

/// `triplesToNT`: `s p o .` lines, sorted, newline-terminated.
pub fn triples_to_nt(ts: &[[String; 3]]) -> String {
    let mut lines: Vec<String> = ts.iter().map(|t| format!("{} {} {} .", t[0], t[1], t[2])).collect();
    lines.sort();
    lines.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard() {
        assert_eq!(query_form("PREFIX a: <x:> SELECT * WHERE { ?s ?p ?o }").unwrap(), "SELECT");
        assert_eq!(query_form("# hi\n  ask { ?s ?p ?o }").unwrap(), "ASK");
        assert!(query_form("INSERT DATA { <a> <b> <c> }").is_err());
        assert_eq!(query_form("SELECT * FROM <http://x> WHERE { ?s ?p ?o }").unwrap_err(), "\"FROM\" is not allowed in a public query");
        assert!(query_form("SELECT * WHERE { SERVICE <http://x> { ?s ?p ?o } }").is_err());
        // a forbidden word inside a string, an IRI, a variable or a prefixed name is not a keyword
        assert!(query_form("SELECT ?from WHERE { ?from <http://x/load> \"drop\" . ?s ex:service ?o }").is_ok());
        assert_eq!(query_form("").unwrap_err(), "empty query");
        assert!(query_form(&"x".repeat(MAX_QUERY_BYTES + 1)).is_err());
    }

    /// The test binary as a worker: `workers_kill_a_runaway_query` starts it with WORKER_ENV set.
    #[test]
    fn worker_entry() {
        if std::env::var_os(WORKER_ENV).is_some() {
            worker_loop(crate::ontology::pack_graph().as_bytes());
        }
    }

    fn test_workers(timeout: Duration) -> Arc<Service> {
        let cmd = WorkerCmd { program: std::env::current_exe().unwrap(), args: ["ontology::query::tests::worker_entry", "--exact", "--nocapture", "--test-threads=1"].map(String::from).to_vec() };
        let mut sp = Service::with_workers(crate::ontology::pack_graph().as_bytes(), cmd).unwrap();
        sp.timeout = timeout;
        Arc::new(sp)
    }

    /// A cross product Oxigraph does not cancel (it reads no store while it joins): the worker is killed at the
    /// deadline, the client strikes, and the next query gets a fresh worker. Two timeouts ban the client.
    #[tokio::test]
    async fn workers_kill_a_runaway_query() {
        let sp = test_workers(Duration::from_millis(400));
        let slow = "SELECT (COUNT(*) AS ?n) WHERE { ?a ?b ?c . ?d ?e ?f . ?g ?h ?i }";
        for _ in 0..2 {
            let t = Instant::now();
            let e = sp.run(slow, "203.0.113.9").await.err().expect("timed out");
            assert!(e.to_string().contains("exceeded"), "{e}");
            assert!(t.elapsed() < Duration::from_secs(5), "answered at the deadline, not when the query ended: {:?}", t.elapsed());
        }
        assert!(sp.gate("203.0.113.9").is_err(), "two timeouts ban the client");
        let r = sp.run("ASK { ?s ?p ?o }", "203.0.113.10").await.expect("a fresh worker answers");
        assert!(matches!(r.answer, Answer::Ask(true)));
        let r = sp.run("SELECT ?s WHERE { ?s ?p ?o } LIMIT 3", "").await.unwrap();
        assert!(matches!(r.answer, Answer::Select { ref rows, .. } if rows.len() == 3));
        assert!(matches!(sp.run("SELECT WHERE {", "").await, Err(QueryError::User(m)) if m.starts_with("query error:")));
    }

    #[test]
    fn limiter() {
        let mut l = Limiter { buckets: HashMap::new(), burst: 2.0, every: Duration::from_secs(3) };
        let t = Instant::now();
        assert!(l.allow("a", t) && l.allow("a", t) && !l.allow("a", t));
        assert!(l.allow("b", t));
        assert!(l.allow("a", t + Duration::from_secs(3)));
    }
}
