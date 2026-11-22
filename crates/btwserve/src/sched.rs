//! The render CPU pool: at most `workers` engine threads compute at once, and the next one to run is the one whose
//! listener needs its audio soonest (earliest deadline first).
//!
//! Every render is still its own thread (it owns its engine, and `render_to` is a plain loop), but between two PCM
//! chunks (60 ms first, then 250 ms of audio, a few ms of CPU each) the thread hands its permit back and asks again
//! with the deadline of its next chunk: the instant a listener who started playing at the first byte reaches it,
//! `started + seconds already rendered`. So
//!
//! - a new render (deadline: now) gets a core at once and its first audio leaves in milliseconds, even when every
//!   core is busy;
//! - a render that is far ahead of its listener (a lightly loaded box renders ~10x real time) yields to the ones that
//!   are close to running dry;
//! - with no contention nothing waits: every render runs flat out and the cache fills as fast as before.
//!
//! When the box is overloaded (more real-time demand than cores), every stream falls behind together; `lag()` says by
//! how much, and the service can refuse new renders on it (`-render-admit-lag-ms`).
//!
//! Waiters park on their own thread handle (no thundering herd); a release hands the permit straight to the earliest
//! waiter. A waiter checks its cancel flag every 100 ms.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::Thread;
use std::time::{Duration, Instant};

struct Waiter {
    thread: Thread,
    granted: AtomicBool,
}

struct St {
    free: usize,
    next: u64,
    waiting: BTreeMap<(Instant, u64), Arc<Waiter>>,
}

pub struct Sched {
    workers: usize,
    st: Mutex<St>,
}

/// A held CPU slot; dropping it hands the slot on.
pub struct Permit {
    s: Arc<Sched>,
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.s.release();
    }
}

impl Sched {
    pub fn new(workers: usize) -> Sched {
        let workers = workers.max(1);
        Sched { workers, st: Mutex::new(St { free: workers, next: 0, waiting: BTreeMap::new() }) }
    }

    pub fn workers(&self) -> usize {
        self.workers
    }

    fn release(&self) {
        let mut st = self.st.lock().unwrap();
        match st.waiting.pop_first() {
            Some((_, w)) => {
                w.granted.store(true, Ordering::Release);
                w.thread.unpark();
            }
            None => st.free += 1,
        }
    }

    /// A slot, when every waiter with an earlier deadline has had one. None if `cancel` was set while waiting.
    pub fn acquire(self: &Arc<Self>, deadline: Instant, cancel: &AtomicBool) -> Option<Permit> {
        let w = {
            let mut st = self.st.lock().unwrap();
            if st.free > 0 {
                st.free -= 1;
                return Some(Permit { s: self.clone() });
            }
            let id = st.next;
            st.next += 1;
            let w = Arc::new(Waiter { thread: std::thread::current(), granted: AtomicBool::new(false) });
            st.waiting.insert((deadline, id), w.clone());
            (w, (deadline, id))
        };
        let (w, key) = w;
        loop {
            if w.granted.load(Ordering::Acquire) {
                return Some(Permit { s: self.clone() });
            }
            if cancel.load(Ordering::Relaxed) {
                let mut st = self.st.lock().unwrap();
                if st.waiting.remove(&key).is_some() {
                    return None;
                }
                drop(st);
                // granted between the check and the lock: give it straight back
                if w.granted.load(Ordering::Acquire) {
                    self.release();
                }
                return None;
            }
            std::thread::park_timeout(Duration::from_millis(100));
        }
    }

    /// How far the most overdue waiting render is behind its listener (zero when none is).
    pub fn lag(&self) -> Duration {
        let st = self.st.lock().unwrap();
        st.waiting.keys().next().map_or(Duration::ZERO, |(d, _)| Instant::now().saturating_duration_since(*d))
    }

    /// Renders waiting for a slot right now.
    #[cfg(test)]
    pub fn queued(&self) -> usize {
        self.st.lock().unwrap().waiting.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn earliest_deadline_runs_first() {
        let s = Arc::new(Sched::new(1));
        let never = AtomicBool::new(false);
        let p = s.acquire(Instant::now(), &never).unwrap();
        let order = Arc::new(Mutex::new(Vec::new()));
        let base = Instant::now();
        let mut hs = Vec::new();
        for (i, ms) in [(0u32, 300u64), (1, 100), (2, 200)] {
            let (s, order) = (s.clone(), order.clone());
            hs.push(std::thread::spawn(move || {
                let never = AtomicBool::new(false);
                let _p = s.acquire(base + Duration::from_millis(ms), &never).unwrap();
                order.lock().unwrap().push(i);
            }));
            std::thread::sleep(Duration::from_millis(30));
        }
        drop(p);
        for h in hs {
            h.join().unwrap();
        }
        assert_eq!(*order.lock().unwrap(), vec![1, 2, 0]);
    }

    #[test]
    fn never_more_than_workers_and_cancel_leaves() {
        let s = Arc::new(Sched::new(3));
        let (now, peak) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let hs: Vec<_> = (0..16)
            .map(|i| {
                let (s, now, peak) = (s.clone(), now.clone(), peak.clone());
                std::thread::spawn(move || {
                    let never = AtomicBool::new(false);
                    for k in 0..20 {
                        let _p = s.acquire(Instant::now() + Duration::from_millis((i * 7 + k) % 13), &never).unwrap();
                        let n = now.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(n, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_micros(200));
                        now.fetch_sub(1, Ordering::SeqCst);
                    }
                })
            })
            .collect();
        for h in hs {
            h.join().unwrap();
        }
        assert!(peak.load(Ordering::SeqCst) <= 3);
        // a cancelled waiter gives up its place and the slot count stays whole
        let a = s.acquire(Instant::now(), &AtomicBool::new(false)).unwrap();
        let b = s.acquire(Instant::now(), &AtomicBool::new(false)).unwrap();
        let c = s.acquire(Instant::now(), &AtomicBool::new(false)).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let (s2, c2) = (s.clone(), cancel.clone());
        let h = std::thread::spawn(move || s2.acquire(Instant::now(), &c2).is_none());
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(s.queued(), 1);
        cancel.store(true, Ordering::Relaxed);
        assert!(h.join().unwrap());
        assert_eq!(s.queued(), 0);
        drop((a, b, c));
        let all: Vec<_> = (0..3).map(|_| s.acquire(Instant::now(), &AtomicBool::new(false)).unwrap()).collect();
        assert_eq!(all.len(), 3);
    }
}
