//! Go `mcpsrv/limiter.go`: a sliding-window counter per key, at most n events in any window.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct Limiter {
    n: usize,
    window: Duration,
    hits: Mutex<HashMap<String, Vec<Instant>>>,
}

impl Limiter {
    pub fn new(n: usize, window: Duration) -> Limiter {
        Limiter { n, window, hits: Mutex::new(HashMap::new()) }
    }

    /// Record an event for `key` if it fits; else the seconds until the oldest leaves the window (at least 1).
    pub fn allow(&self, key: &str) -> Result<(), u64> {
        if self.n == 0 {
            return Ok(());
        }
        let now = Instant::now();
        let mut hits = self.hits.lock().unwrap();
        if hits.len() > 20000 {
            hits.retain(|_, v| v.last().is_some_and(|t| now.duration_since(*t) < self.window));
        }
        let h = hits.entry(key.to_string()).or_default();
        h.retain(|t| now.duration_since(*t) < self.window);
        if h.len() >= self.n {
            let d = self.window.saturating_sub(now.duration_since(h[0]));
            return Err(d.as_secs_f64().ceil().max(1.0) as u64);
        }
        h.push(now);
        Ok(())
    }
}
