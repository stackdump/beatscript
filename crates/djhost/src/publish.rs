//! The CDN transports of `Session/SessionPublish.swift`: a directory (tests; a CDN tree
//! mounted locally) and ssh to the CDN origin host, the way the render farm stages. The bundle, the
//! staging order and the probe are `djengine::session::publish`.
//!
//! Nothing here runs unless asked: `publish_session` stages, reloads and, only when asked,
//! mirrors (the public step).

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use djengine::session::publish::{CdnTransport, PublishError, PublishStep, SessionBundle};

/// Writes straight into a blobs directory.
pub struct DirectoryTransport {
    pub blobs: PathBuf,
}

impl DirectoryTransport {
    pub fn new(blobs: impl Into<PathBuf>) -> Self {
        DirectoryTransport {
            blobs: blobs.into(),
        }
    }
}

impl CdnTransport for DirectoryTransport {
    fn stage(
        &self,
        bundle: &SessionBundle,
        progress: &mut dyn FnMut(PublishStep),
    ) -> Result<(), PublishError> {
        let dir = self.blobs.join(&bundle.cid);
        let fail = |what: &str, e: std::io::Error| {
            PublishError::CommandFailed(what.into(), 1, e.to_string())
        };
        std::fs::create_dir_all(&dir).map_err(|e| fail("mkdir", e))?;
        let n = bundle.files.len();
        for (i, f) in bundle.files.iter().enumerate() {
            progress(PublishStep::Staging {
                file: f.name.clone(),
                index: i + 1,
                of: n,
            });
            // atomically, so a reader never sees a partial file
            let tmp = dir.join(format!(".{}.part", f.name));
            std::fs::write(&tmp, &f.data).map_err(|e| fail(&format!("stage {}", f.name), e))?;
            std::fs::rename(&tmp, dir.join(&f.name))
                .map_err(|e| fail(&format!("stage {}", f.name), e))?;
        }
        Ok(())
    }
    fn reload(&self, _cid: &str) -> Result<bool, PublishError> {
        Ok(true)
    }
    fn mirror(&self) -> Result<(), PublishError> {
        Ok(())
    }
}

/// The process seam: command line and stdin in, exit status and output out.
pub type Runner =
    Box<dyn Fn(&[String], Option<&[u8]>) -> std::io::Result<(i32, String)> + Send + Sync>;

/// Stages onto the CDN origin host over ssh: nothing but the existing ssh key is needed. The host and its paths
/// are build-time settings (DJHOST_PUBLISH_HOST, DJHOST_PUBLISH_BLOBS, DJHOST_PUBLISH_MIRROR), so a booth sets
/// its own in .cargo/config.toml [env]; the defaults name no machine.
pub struct SshTransport {
    pub host: String,
    pub blobs_dir: String,
    pub reload_url: String,
    pub mirror_script: String,
    pub run: Runner,
}

impl Default for SshTransport {
    fn default() -> Self {
        SshTransport {
            host: option_env!("DJHOST_PUBLISH_HOST").unwrap_or("cdn-origin.local").into(),
            blobs_dir: option_env!("DJHOST_PUBLISH_BLOBS").unwrap_or("~/cdn-stackdump-com/data/blobs").into(),
            reload_url: "http://127.0.0.1:18091/admin/reload".into(),
            mirror_script: option_env!("DJHOST_PUBLISH_MIRROR").unwrap_or("~/cdn-stackdump-com/bin/mirror-push.sh").into(),
            run: Box::new(process),
        }
    }
}

/// `~/x` must expand on the remote side, so a leading `~/` is left outside the quotes.
pub fn remote_path(p: &str) -> String {
    match p.strip_prefix("~/") {
        Some(rest) => format!("~/{}", shell_quote(rest)),
        None => shell_quote(p),
    }
}

pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

impl SshTransport {
    fn ssh(&self, command: &str, stdin: Option<&[u8]>) -> Result<(i32, String), PublishError> {
        let argv: Vec<String> = [
            "ssh",
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=10",
            "-o",
            "ServerAliveInterval=15",
            &self.host,
            command,
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        (self.run)(&argv, stdin)
            .map_err(|e| PublishError::CommandFailed("ssh".into(), -1, e.to_string()))
    }
}

impl CdnTransport for SshTransport {
    fn stage(
        &self,
        bundle: &SessionBundle,
        progress: &mut dyn FnMut(PublishStep),
    ) -> Result<(), PublishError> {
        if !SessionBundle::is_safe_cid(&bundle.cid) {
            return Err(PublishError::UnsafeCid(bundle.cid.clone()));
        }
        let dir = format!("{}/{}", remote_path(&self.blobs_dir), bundle.cid);
        let n = bundle.files.len();
        for (i, f) in bundle.files.iter().enumerate() {
            progress(PublishStep::Staging {
                file: f.name.clone(),
                index: i + 1,
                of: n,
            });
            // write to a dotfile and rename, so a reader never sees a partial file
            let tmp = format!("{dir}/.{}.part", f.name);
            let cmd = format!("mkdir -p {dir} && cat > {tmp} && mv {tmp} {dir}/{}", f.name);
            let (status, out) = self.ssh(&cmd, Some(&f.data))?;
            if status != 0 {
                return Err(PublishError::CommandFailed(
                    format!("stage {}", f.name),
                    status,
                    out,
                ));
            }
        }
        Ok(())
    }

    fn reload(&self, cid: &str) -> Result<bool, PublishError> {
        if !SessionBundle::is_safe_cid(cid) {
            return Err(PublishError::UnsafeCid(cid.into()));
        }
        let url = shell_quote(&format!("{}?cid={cid}", self.reload_url));
        let (status, _) = self.ssh(&format!("curl -s -f -m 10 -X POST {url} >/dev/null"), None)?;
        Ok(status == 0)
    }

    fn mirror(&self) -> Result<(), PublishError> {
        let script = remote_path(&self.mirror_script);
        let (status, out) = self.ssh(
            &format!("test -x {script} && nohup {script} >/dev/null 2>&1 &"),
            None,
        )?;
        if status != 0 {
            return Err(PublishError::CommandFailed(
                "mirror-push".into(),
                status,
                out,
            ));
        }
        Ok(())
    }
}

/// `SSHTransport.process`: run `argv` with `stdin`, feeding it on its own thread so a large
/// WAV can't deadlock against the output pipe. A write to a process that already exited is
/// an error, not a crash.
pub fn process(argv: &[String], stdin: Option<&[u8]>) -> std::io::Result<(i32, String)> {
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut inp = child.stdin.take();
    let data = stdin.map(|d| d.to_vec());
    let writer = std::thread::spawn(move || {
        if let (Some(mut i), Some(d)) = (inp.take(), data) {
            let _ = i.write_all(&d);
        }
        // dropping `i` closes the pipe
    });
    let out = child.wait_with_output()?;
    let _ = writer.join();
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok((out.status.code().unwrap_or(-1), text))
}
