//! The HEOS worker: a thread that finds a player, holds two CLI connections to it (commands, and
//! change events, as the spec advises) and keeps a [`Snapshot`] of every zone that the window
//! and the control socket read without waiting on the network.
//!
//! Commands go in through [`Heos::send`] and are carried out in order. Volume moves are
//! coalesced (the latest per zone wins), so a fader swept across its travel sends a handful of
//! `set_volume`s, not hundreds. Events update the snapshot as they come; a full refresh runs on
//! connect, when the zones or groups change and every [`REFRESH`] besides. A lost connection is
//! retried with a backoff, by discovery again when no host was given.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::conn::{resolve, Conn};
use super::discover::{discover, Found};
use super::proto::{
    level, on_off, parse_groups, parse_players, Group, NowPlaying, PlayState, Player, Reply,
};

/// A full re-read of every zone, besides the events.
pub const REFRESH: Duration = Duration::from_secs(20);
/// How long discovery listens.
pub const DISCOVER: Duration = Duration::from_millis(2500);
const DIAL: Duration = Duration::from_secs(3);
const LOG_CAP: usize = 80;

/// Where to find a player.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// SSDP + mDNS.
    Auto,
    /// `host[:port]`.
    Host(String),
}

impl Target {
    pub fn parse(s: &str) -> Target {
        match s.trim() {
            "" | "auto" => Target::Auto,
            h => Target::Host(h.to_string()),
        }
    }
    pub fn describe(&self) -> String {
        match self {
            Target::Auto => "auto (SSDP + mDNS)".into(),
            Target::Host(h) => h.clone(),
        }
    }
}

/// What the worker can be asked to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    /// Drop the connection and find the players again.
    Discover,
    /// Talk to this host from now on.
    Connect(Target),
    Refresh,
    Volume {
        pid: i64,
        level: u8,
    },
    /// ± steps of 1-10 (`volume_up` / `volume_down`).
    VolumeStep {
        pid: i64,
        delta: i32,
    },
    GroupVolume {
        gid: i64,
        level: u8,
    },
    /// None toggles.
    Mute {
        pid: i64,
        on: Option<bool>,
    },
    GroupMute {
        gid: i64,
        on: Option<bool>,
    },
    PlayState {
        pid: i64,
        state: PlayState,
    },
    /// The leader first, then the members; one pid alone ungroups that leader's group.
    SetGroup {
        pids: Vec<i64>,
    },
    /// `browse/play_stream`: the player (a group's leader: the group follows) plays `url`.
    PlayUrl {
        pid: i64,
        url: String,
    },
}

enum Msg {
    Cmd(Cmd),
    Event(Reply),
    EventsGone(String),
}

/// One line of the worker's log.
#[derive(Clone, Debug, PartialEq)]
pub struct LogLine {
    pub seq: u64,
    pub ok: bool,
    pub text: String,
}

/// Everything known now.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub connected: bool,
    /// The player the CLI is connected to.
    pub host: Option<String>,
    /// In words: connecting, connected to …, why not.
    pub status: String,
    pub players: Vec<Player>,
    pub groups: Vec<Group>,
    /// What the last discovery found.
    pub found: Vec<Found>,
    /// Commands sent and what came of them, newest last.
    pub log: Vec<LogLine>,
    /// Bumped on every change.
    pub generation: u64,
    /// Events received.
    pub events: u64,
}

impl Snapshot {
    pub fn player(&self, pid: i64) -> Option<&Player> {
        self.players.iter().find(|p| p.pid == pid)
    }
    pub fn group_of(&self, pid: i64) -> Option<&Group> {
        self.groups.iter().find(|g| g.has(pid))
    }
}

pub struct Heos {
    tx: Sender<Msg>,
    state: Arc<Mutex<Snapshot>>,
    stop: Arc<AtomicBool>,
    pub target: Target,
}

impl Heos {
    pub fn start(target: Target) -> Heos {
        let (tx, rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(Snapshot {
            status: format!("starting: {}", target.describe()),
            ..Default::default()
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let mut w = Worker {
            target: target.clone(),
            rx,
            tx: tx.clone(),
            state: state.clone(),
            stop: stop.clone(),
            conn: None,
            events_stop: None,
            next_try: Instant::now(),
            backoff: Duration::from_secs(2),
            refreshed: Instant::now(),
            seq: 0,
        };
        std::thread::Builder::new()
            .name("heos".into())
            .spawn(move || w.run())
            .expect("spawn the heos thread");
        Heos {
            tx,
            state,
            stop,
            target,
        }
    }

    pub fn send(&self, c: Cmd) {
        if let Cmd::Connect(t) = &c {
            // the handle reports the target asked for at once
            if let Ok(mut s) = self.state.lock() {
                s.status = format!("connecting: {}", t.describe());
            }
        }
        let _ = self.tx.send(Msg::Cmd(c));
    }

    pub fn snapshot(&self) -> Snapshot {
        self.state.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Only the generation: has anything changed?
    pub fn generation(&self) -> u64 {
        self.state.lock().map_or(0, |s| s.generation)
    }
}

impl Drop for Heos {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.tx.send(Msg::Cmd(Cmd::Refresh)); // wake it
    }
}

struct Worker {
    target: Target,
    rx: Receiver<Msg>,
    tx: Sender<Msg>,
    state: Arc<Mutex<Snapshot>>,
    stop: Arc<AtomicBool>,
    conn: Option<Conn>,
    events_stop: Option<Arc<AtomicBool>>,
    next_try: Instant,
    backoff: Duration,
    refreshed: Instant,
    seq: u64,
}

impl Worker {
    fn edit(&self, f: impl FnOnce(&mut Snapshot)) {
        if let Ok(mut s) = self.state.lock() {
            f(&mut s);
            s.generation += 1;
        }
    }

    fn log(&mut self, ok: bool, text: String) {
        self.seq += 1;
        let seq = self.seq;
        self.edit(|s| {
            s.log.push(LogLine { seq, ok, text });
            let over = s.log.len().saturating_sub(LOG_CAP);
            s.log.drain(..over);
        });
    }

    fn run(&mut self) {
        while !self.stop.load(Ordering::SeqCst) {
            if self.conn.is_none() && Instant::now() >= self.next_try {
                self.connect();
            }
            let first = match self.rx.recv_timeout(Duration::from_millis(200)) {
                Ok(m) => Some(m),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            };
            let mut msgs: Vec<Msg> = first.into_iter().collect();
            msgs.extend(self.rx.try_iter());
            for m in coalesce(msgs) {
                if self.stop.load(Ordering::SeqCst) {
                    break;
                }
                match m {
                    Msg::Cmd(c) => self.command(c),
                    Msg::Event(e) => self.event(e),
                    Msg::EventsGone(why) => {
                        if self.conn.is_some() {
                            self.log(false, format!("event connection lost: {why}"));
                            self.drop_conn(format!("lost: {why}"));
                        }
                    }
                }
            }
            if self.conn.is_some() && self.refreshed.elapsed() >= REFRESH {
                self.refresh();
            }
        }
        if let Some(s) = &self.events_stop {
            s.store(true, Ordering::SeqCst);
        }
    }

    fn drop_conn(&mut self, why: String) {
        self.conn = None;
        if let Some(s) = self.events_stop.take() {
            s.store(true, Ordering::SeqCst);
        }
        self.next_try = Instant::now() + self.backoff;
        self.backoff = (self.backoff * 2).min(Duration::from_secs(30));
        self.edit(|s| {
            s.connected = false;
            s.status = why;
        });
    }

    fn connect(&mut self) {
        let candidates: Vec<(String, std::net::SocketAddr)> = match &self.target {
            Target::Host(h) => match resolve(h) {
                Ok(a) => vec![(h.clone(), a)],
                Err(e) => {
                    self.drop_conn(e);
                    return;
                }
            },
            Target::Auto => {
                self.edit(|s| s.status = "discovering (SSDP + mDNS)…".into());
                let found = discover(DISCOVER);
                let n = found.len();
                let c = found
                    .iter()
                    .map(|f| {
                        let label = f.name.clone().unwrap_or_else(|| f.ip.to_string());
                        (label, std::net::SocketAddr::new(f.ip, super::proto::PORT))
                    })
                    .collect();
                self.edit(|s| s.found = found);
                if n == 0 {
                    self.drop_conn("no HEOS players answered SSDP or mDNS".into());
                    return;
                }
                c
            }
        };
        let mut why = String::new();
        for (label, addr) in candidates {
            match Conn::open(addr, DIAL)
                .and_then(|mut c| c.call("player/get_players", &[]).map(|_| c))
            {
                Ok(c) => {
                    self.conn = Some(c);
                    self.backoff = Duration::from_secs(2);
                    self.start_events(addr);
                    let host = format!("{label} ({addr})");
                    self.edit(|s| {
                        s.connected = true;
                        s.host = Some(addr.to_string());
                        s.status = format!("connected to {host}");
                    });
                    self.log(true, format!("connected to {host}"));
                    self.refresh();
                    return;
                }
                Err(e) => why = e,
            }
        }
        self.drop_conn(format!("could not connect: {why}"));
    }

    fn start_events(&mut self, addr: std::net::SocketAddr) {
        let stop = Arc::new(AtomicBool::new(false));
        self.events_stop = Some(stop.clone());
        let tx = self.tx.clone();
        std::thread::Builder::new()
            .name("heos-events".into())
            .spawn(move || {
                let mut c = match Conn::open(addr, DIAL) {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = tx.send(Msg::EventsGone(e));
                        return;
                    }
                };
                if let Err(e) = c.call(
                    "system/register_for_change_events",
                    &[("enable", "on".into())],
                ) {
                    let _ = tx.send(Msg::EventsGone(e));
                    return;
                }
                while !stop.load(Ordering::SeqCst) {
                    for e in std::mem::take(&mut c.events) {
                        let _ = tx.send(Msg::Event(e));
                    }
                    match c.read_line(Duration::from_millis(500)) {
                        Ok(Some(r)) if r.is_event() => {
                            if tx.send(Msg::Event(r)).is_err() {
                                return;
                            }
                        }
                        Ok(_) => {}
                        Err(e) => {
                            if !stop.load(Ordering::SeqCst) {
                                let _ = tx.send(Msg::EventsGone(e));
                            }
                            return;
                        }
                    }
                }
            })
            .ok();
    }

    /// One call on the command connection; a broken connection is dropped (and retried).
    fn call(&mut self, path: &str, args: &[(&str, String)]) -> Result<Reply, String> {
        let Some(c) = self.conn.as_mut() else {
            return Err("not connected to a HEOS player".into());
        };
        let r = c.call(path, args);
        let events = std::mem::take(&mut c.events);
        for e in events {
            self.event(e);
        }
        if let Err(e) = &r {
            if e.contains("closed") || e.contains("Broken pipe") || e.contains("reset") {
                self.drop_conn(format!("lost: {e}"));
            }
        }
        r
    }

    /// A command that changes something, logged either way.
    fn act(&mut self, path: &str, args: &[(&str, String)]) -> bool {
        let words = format!(
            "{path} {}",
            args.iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
        match self.call(path, args) {
            Ok(_) => {
                self.log(true, words);
                true
            }
            Err(e) => {
                self.log(false, format!("{words}: {e}"));
                false
            }
        }
    }

    fn command(&mut self, c: Cmd) {
        match c {
            Cmd::Discover => {
                self.target = Target::Auto;
                self.drop_conn("rediscovering".into());
                self.next_try = Instant::now();
            }
            Cmd::Connect(t) => {
                self.target = t;
                self.drop_conn("connecting".into());
                self.backoff = Duration::from_secs(2);
                self.next_try = Instant::now();
            }
            Cmd::Refresh => {
                if self.conn.is_some() {
                    self.refresh();
                } else {
                    self.next_try = Instant::now();
                }
            }
            Cmd::Volume { pid, level: l } => {
                if self.act(
                    "player/set_volume",
                    &[("pid", pid.to_string()), ("level", l.to_string())],
                ) {
                    self.edit(|s| {
                        if let Some(p) = s.players.iter_mut().find(|p| p.pid == pid) {
                            p.volume = Some(l);
                        }
                    });
                }
            }
            Cmd::VolumeStep { pid, delta } => {
                let path = if delta >= 0 {
                    "player/volume_up"
                } else {
                    "player/volume_down"
                };
                let step = delta.unsigned_abs().clamp(1, 10);
                if self.act(
                    path,
                    &[("pid", pid.to_string()), ("step", step.to_string())],
                ) {
                    self.read_volume(pid);
                }
            }
            Cmd::GroupVolume { gid, level: l } => {
                if self.act(
                    "group/set_volume",
                    &[("gid", gid.to_string()), ("level", l.to_string())],
                ) {
                    self.edit(|s| {
                        if let Some(g) = s.groups.iter_mut().find(|g| g.gid == gid) {
                            g.volume = Some(l);
                        }
                    });
                }
            }
            Cmd::Mute { pid, on } => {
                let ok = match on {
                    Some(v) => self.act(
                        "player/set_mute",
                        &[
                            ("pid", pid.to_string()),
                            ("state", if v { "on" } else { "off" }.into()),
                        ],
                    ),
                    None => self.act("player/toggle_mute", &[("pid", pid.to_string())]),
                };
                if ok {
                    self.read_volume(pid);
                }
            }
            Cmd::GroupMute { gid, on } => {
                let ok = match on {
                    Some(v) => self.act(
                        "group/set_mute",
                        &[
                            ("gid", gid.to_string()),
                            ("state", if v { "on" } else { "off" }.into()),
                        ],
                    ),
                    None => self.act("group/toggle_mute", &[("gid", gid.to_string())]),
                };
                if ok {
                    self.read_group_volume(gid);
                }
            }
            Cmd::PlayState { pid, state } => {
                if self.act(
                    "player/set_play_state",
                    &[("pid", pid.to_string()), ("state", state.as_str().into())],
                ) {
                    self.edit(|s| {
                        if let Some(p) = s.players.iter_mut().find(|p| p.pid == pid) {
                            p.state = Some(state);
                        }
                    });
                }
            }
            Cmd::SetGroup { pids } => {
                let list = pids
                    .iter()
                    .map(i64::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                if self.act("group/set_group", &[("pid", list)]) {
                    self.read_groups();
                }
            }
            Cmd::PlayUrl { pid, url } => {
                if self.act(
                    "browse/play_stream",
                    &[("pid", pid.to_string()), ("url", url)],
                ) {
                    self.read_now_playing(pid);
                    self.read_state(pid);
                }
            }
        }
    }

    fn event(&mut self, e: Reply) {
        self.edit(|s| s.events += 1);
        let pid = e.field_i64("pid");
        match e.command.as_str() {
            "event/player_state_changed" => {
                let st = e.field("state").and_then(PlayState::parse);
                if let Some(pid) = pid {
                    self.edit(|s| {
                        if let Some(p) = s.players.iter_mut().find(|p| p.pid == pid) {
                            p.state = st;
                        }
                    });
                }
            }
            "event/player_volume_changed" => {
                if let Some(pid) = pid {
                    let l = e.field_i64("level").map(|v| level(v as f64));
                    let m = e.field("mute").and_then(on_off);
                    self.edit(|s| {
                        if let Some(p) = s.players.iter_mut().find(|p| p.pid == pid) {
                            p.volume = l.or(p.volume);
                            p.mute = m.or(p.mute);
                        }
                    });
                }
            }
            "event/group_volume_changed" => {
                if let Some(gid) = e.field_i64("gid") {
                    let l = e.field_i64("level").map(|v| level(v as f64));
                    let m = e.field("mute").and_then(on_off);
                    self.edit(|s| {
                        if let Some(g) = s.groups.iter_mut().find(|g| g.gid == gid) {
                            g.volume = l.or(g.volume);
                            g.mute = m.or(g.mute);
                        }
                    });
                }
            }
            "event/player_now_playing_changed" => {
                if let Some(pid) = pid {
                    self.read_now_playing(pid);
                }
            }
            "event/groups_changed" => self.read_groups(),
            "event/players_changed" => self.refresh(),
            _ => {}
        }
    }

    fn read_volume(&mut self, pid: i64) {
        let args = [("pid", pid.to_string())];
        let v = self
            .call("player/get_volume", &args)
            .ok()
            .and_then(|r| r.field_i64("level"));
        let m = self
            .call("player/get_mute", &args)
            .ok()
            .and_then(|r| r.field("state").and_then(on_off));
        self.edit(|s| {
            if let Some(p) = s.players.iter_mut().find(|p| p.pid == pid) {
                p.volume = v.map(|x| level(x as f64)).or(p.volume);
                p.mute = m.or(p.mute);
            }
        });
    }

    fn read_state(&mut self, pid: i64) {
        let st = self
            .call("player/get_play_state", &[("pid", pid.to_string())])
            .ok()
            .and_then(|r| r.field("state").and_then(PlayState::parse));
        self.edit(|s| {
            if let Some(p) = s.players.iter_mut().find(|p| p.pid == pid) {
                p.state = st.or(p.state);
            }
        });
    }

    fn read_now_playing(&mut self, pid: i64) {
        if let Ok(r) = self.call("player/get_now_playing_media", &[("pid", pid.to_string())]) {
            let np = r
                .payload
                .as_ref()
                .map(NowPlaying::parse)
                .unwrap_or_default();
            self.edit(|s| {
                if let Some(p) = s.players.iter_mut().find(|p| p.pid == pid) {
                    p.now = np;
                }
            });
        }
    }

    fn read_group_volume(&mut self, gid: i64) {
        let args = [("gid", gid.to_string())];
        let v = self
            .call("group/get_volume", &args)
            .ok()
            .and_then(|r| r.field_i64("level"));
        let m = self
            .call("group/get_mute", &args)
            .ok()
            .and_then(|r| r.field("state").and_then(on_off));
        self.edit(|s| {
            if let Some(g) = s.groups.iter_mut().find(|g| g.gid == gid) {
                g.volume = v.map(|x| level(x as f64)).or(g.volume);
                g.mute = m.or(g.mute);
            }
        });
    }

    fn read_groups(&mut self) {
        let Ok(r) = self.call("group/get_groups", &[]) else {
            return;
        };
        let groups = r.payload.as_ref().map(parse_groups).unwrap_or_default();
        let gids: Vec<i64> = groups.iter().map(|g| g.gid).collect();
        self.edit(|s| {
            for p in s.players.iter_mut() {
                p.gid = groups.iter().find(|g| g.has(p.pid)).map(|g| g.gid);
            }
            // keep the volumes already read for groups that stay
            let old = std::mem::take(&mut s.groups);
            s.groups = groups
                .into_iter()
                .map(|mut g| {
                    if let Some(o) = old.iter().find(|o| o.gid == g.gid) {
                        g.volume = o.volume;
                        g.mute = o.mute;
                    }
                    g
                })
                .collect();
        });
        for gid in gids {
            self.read_group_volume(gid);
        }
    }

    fn refresh(&mut self) {
        self.refreshed = Instant::now();
        let players = match self.call("player/get_players", &[]) {
            Ok(r) => r.payload.as_ref().map(parse_players).unwrap_or_default(),
            Err(e) => {
                self.log(false, format!("player/get_players: {e}"));
                return;
            }
        };
        let pids: Vec<i64> = players.iter().map(|p| p.pid).collect();
        self.edit(|s| {
            // keep what is known of zones that stay until it is read again
            let old = std::mem::take(&mut s.players);
            s.players = players
                .into_iter()
                .map(|mut p| {
                    if let Some(o) = old.iter().find(|o| o.pid == p.pid) {
                        p.volume = o.volume;
                        p.mute = o.mute;
                        p.state = o.state;
                        p.now = o.now.clone();
                    }
                    p
                })
                .collect();
        });
        for pid in pids {
            if self.conn.is_none() {
                return;
            }
            self.read_volume(pid);
            self.read_state(pid);
            self.read_now_playing(pid);
        }
        self.read_groups();
    }
}

/// Only the latest volume per zone (and per group) of a batch: a fader's sweep is one move.
fn coalesce(msgs: Vec<Msg>) -> Vec<Msg> {
    let last_of = |want: &dyn Fn(&Msg) -> Option<(u8, i64)>| -> Vec<Option<usize>> {
        let mut last: Vec<(u8, i64, usize)> = Vec::new();
        for (i, m) in msgs.iter().enumerate() {
            if let Some((k, id)) = want(m) {
                match last.iter_mut().find(|(a, b, _)| *a == k && *b == id) {
                    Some(e) => e.2 = i,
                    None => last.push((k, id, i)),
                }
            }
        }
        msgs.iter()
            .enumerate()
            .map(|(i, m)| {
                want(m).map(|(k, id)| {
                    last.iter()
                        .find(|e| e.0 == k && e.1 == id)
                        .map_or(i, |e| e.2)
                })
            })
            .collect()
    };
    let key = |m: &Msg| match m {
        Msg::Cmd(Cmd::Volume { pid, .. }) => Some((0u8, *pid)),
        Msg::Cmd(Cmd::GroupVolume { gid, .. }) => Some((1u8, *gid)),
        _ => None,
    };
    let keep = last_of(&key);
    msgs.into_iter()
        .enumerate()
        .filter(|(i, _)| keep[*i].is_none_or(|last| last == *i))
        .map(|(_, m)| m)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fader_sweep_is_one_set_volume_per_zone() {
        let msgs = vec![
            Msg::Cmd(Cmd::Volume { pid: 1, level: 10 }),
            Msg::Cmd(Cmd::Volume { pid: 2, level: 5 }),
            Msg::Cmd(Cmd::Mute { pid: 1, on: None }),
            Msg::Cmd(Cmd::Volume { pid: 1, level: 30 }),
            Msg::Cmd(Cmd::GroupVolume { gid: 1, level: 7 }),
            Msg::Cmd(Cmd::Volume { pid: 1, level: 40 }),
        ];
        let out: Vec<Cmd> = coalesce(msgs)
            .into_iter()
            .filter_map(|m| match m {
                Msg::Cmd(c) => Some(c),
                _ => None,
            })
            .collect();
        assert_eq!(
            out,
            vec![
                Cmd::Volume { pid: 2, level: 5 },
                Cmd::Mute { pid: 1, on: None },
                Cmd::GroupVolume { gid: 1, level: 7 },
                Cmd::Volume { pid: 1, level: 40 },
            ]
        );
    }

    #[test]
    fn targets_parse() {
        assert_eq!(Target::parse("auto"), Target::Auto);
        assert_eq!(Target::parse(""), Target::Auto);
        assert_eq!(Target::parse("10.0.0.5"), Target::Host("10.0.0.5".into()));
    }
}
