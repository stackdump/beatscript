//! The Claude agent process the console spawns (`DJViews/AgentController.swift`'s process half):
//! one long-lived `claude -p --input-format stream-json …`, a user line per turn, its output
//! parsed by `djengine::agent` and handed to a callback. The pure protocol (arguments, parsing)
//! is `djengine::agent`; this only moves bytes.

use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::thread::JoinHandle;

use djengine::agent::{arguments, user_message, AgentEvent, Launch};

pub struct AgentProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    reader: Option<JoinHandle<()>>,
    /// The session id from the init line (`--resume` it next time), once seen.
    pub session: std::sync::Arc<std::sync::Mutex<Option<String>>>,
}

impl AgentProcess {
    /// Spawn `claude` (a path, or a fake script in tests) in `cwd` with `launch`'s arguments.
    /// Every parsed output line's events go to `on_event` on the reader thread.
    pub fn spawn(
        claude: &str,
        launch: &Launch,
        cwd: &Path,
        env: &[(String, String)],
        mut on_event: impl FnMut(AgentEvent) + Send + 'static,
    ) -> io::Result<AgentProcess> {
        let mut cmd = Command::new(claude);
        cmd.args(arguments(launch))
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take();
        let out = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("no stdout"))?;
        let session = std::sync::Arc::new(std::sync::Mutex::new(None));
        let seen = session.clone();
        let reader = std::thread::Builder::new()
            .name("dj-agent-reader".into())
            .spawn(move || {
                for line in BufReader::new(out).lines() {
                    let Ok(line) = line else { return };
                    for e in djengine::agent::parse(&line) {
                        if let AgentEvent::Ready { session, .. } = &e {
                            if let Ok(mut g) = seen.lock() {
                                *g = Some(session.clone());
                            }
                        }
                        on_event(e);
                    }
                }
            })?;
        Ok(AgentProcess {
            child,
            stdin,
            reader: Some(reader),
            session,
        })
    }

    /// One user turn.
    pub fn say(&mut self, text: &str) -> io::Result<()> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "the agent is stopped"))?;
        stdin.write_all(user_message(text).as_bytes())?;
        stdin.write_all(b"\n")?;
        stdin.flush()
    }

    /// Whether the process is still running.
    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// End the process (closing its input first, then killing it if it lingers) and wait for
    /// the reader to drain.
    pub fn stop(&mut self) {
        self.stdin = None;
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
        while self.is_running() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if self.is_running() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        if let Some(r) = self.reader.take() {
            let _ = r.join();
        }
    }
}

impl Drop for AgentProcess {
    fn drop(&mut self) {
        self.stop();
    }
}
