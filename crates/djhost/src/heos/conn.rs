//! One CLI connection to a player: a command line out, its answer back. Acknowledgements
//! (`command under process`) are waited past, events that arrive in between are kept for the
//! caller, and an answer to some other command (a late one, after a timeout) is skipped.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use super::proto::{command, Reply};

/// How long one answer may take (`under process` acks restart nothing: this is the whole wait).
pub const ANSWER: Duration = Duration::from_secs(8);

pub struct Conn {
    pub addr: SocketAddr,
    writer: TcpStream,
    reader: BufReader<TcpStream>,
    /// Events read while waiting for an answer.
    pub events: Vec<Reply>,
}

/// `host`, `host:port` or `ip`: the address to dial (port 1255 when none is given).
pub fn resolve(host: &str) -> Result<SocketAddr, String> {
    let with_port = if host.parse::<SocketAddr>().is_ok()
        || host
            .rsplit_once(':')
            .is_some_and(|(_, p)| p.parse::<u16>().is_ok() && !host.contains("::"))
    {
        host.to_string()
    } else {
        format!("{host}:{}", super::proto::PORT)
    };
    with_port
        .to_socket_addrs()
        .map_err(|e| format!("{host}: {e}"))?
        .next()
        .ok_or_else(|| format!("{host}: no address"))
}

impl Conn {
    pub fn open(addr: SocketAddr, timeout: Duration) -> Result<Conn, String> {
        let s = TcpStream::connect_timeout(&addr, timeout).map_err(|e| format!("{addr}: {e}"))?;
        let _ = s.set_nodelay(true);
        let r = s.try_clone().map_err(|e| e.to_string())?;
        Ok(Conn {
            addr,
            writer: s,
            reader: BufReader::new(r),
            events: Vec::new(),
        })
    }

    /// One line, waiting at most `wait`. None on a timeout; Err when the connection is gone.
    pub fn read_line(&mut self, wait: Duration) -> Result<Option<Reply>, String> {
        let _ = self
            .reader
            .get_ref()
            .set_read_timeout(Some(wait.max(Duration::from_millis(1))));
        let mut line = String::new();
        match self.reader.read_line(&mut line) {
            Ok(0) => Err(format!("{}: closed", self.addr)),
            Ok(_) if line.trim().is_empty() => Ok(None),
            Ok(_) => Reply::parse(&line).map(Some),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                // a line cut by the timeout is lost with it; the CLI's lines are short and whole
                Ok(None)
            }
            Err(e) => Err(format!("{}: {e}", self.addr)),
        }
    }

    pub fn send(&mut self, path: &str, args: &[(&str, String)]) -> Result<(), String> {
        self.writer
            .write_all(command(path, args).as_bytes())
            .map_err(|e| format!("{}: {e}", self.addr))
    }

    /// Send `path?args` and wait for its answer. A `fail` comes back as Err (its text).
    pub fn call(&mut self, path: &str, args: &[(&str, String)]) -> Result<Reply, String> {
        self.send(path, args)?;
        let until = Instant::now() + ANSWER;
        loop {
            let Some(left) = until.checked_duration_since(Instant::now()) else {
                return Err(format!("{path}: no answer in {}s", ANSWER.as_secs()));
            };
            let Some(r) = self.read_line(left)? else {
                continue;
            };
            if r.is_event() {
                self.events.push(r);
                continue;
            }
            if r.command != path || r.under_process() {
                continue;
            }
            return if r.ok {
                Ok(r)
            } else {
                Err(format!("{path}: {}", r.error_text()))
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_get_the_cli_port_unless_they_name_one() {
        assert_eq!(resolve("127.0.0.1").unwrap().port(), 1255);
        assert_eq!(resolve("127.0.0.1:4000").unwrap().port(), 4000);
        assert_eq!(resolve("localhost:4001").unwrap().port(), 4001);
    }
}
