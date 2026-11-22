//! The HEOS CLI protocol, from Denon's public "HEOS CLI Protocol Specification": plain text
//! commands over TCP port 1255, one per line, `heos://group/command?key=value&key=value\r\n`,
//! answered by one JSON object per line:
//!
//! ```text
//! {"heos": {"command": "player/get_volume", "result": "success", "message": "pid=-15&level=21"}}
//! {"heos": {"command": "player/get_players", "result": "success", "message": ""}, "payload": [...]}
//! {"heos": {"command": "player/get_volume", "result": "fail", "message": "eid=2&text=ID Not Valid&pid=1"}}
//! {"heos": {"command": "player/get_players", "result": "success", "message": "command under process"}}
//! {"heos": {"command": "event/player_volume_changed", "message": "pid=-15&level=30&mute=off"}}
//! ```
//!
//! A reply whose message says `command under process` is only an acknowledgement: the real
//! answer follows. Events (`event/...`, after `system/register_for_change_events?enable=on`) come
//! unasked, interleaved with answers. In values, `&`, `=` and `%` are percent-encoded. Pids and
//! gids are signed 32-bit numbers (often negative).
//!
//! Pure: no sockets here. [`super::conn`] does the I/O.

use djengine::json::JsonValue;

/// The CLI's TCP port.
pub const PORT: u16 = 1255;

/// A value as the CLI wants it: `&`, `=`, `%` (and anything not printable ASCII, and space)
/// percent-encoded; everything else as it is, so a URL keeps its `:` and `/`.
pub fn encode_value(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for b in v.bytes() {
        if b == b'&' || b == b'=' || b == b'%' || b == b' ' || !(0x21..0x7f).contains(&b) {
            out.push_str(&format!("%{b:02X}"));
        } else {
            out.push(b as char);
        }
    }
    out
}

/// Undo [`encode_value`] (and any other `%XX`); a broken escape is kept as written.
pub fn decode_value(v: &str) -> String {
    let b = v.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok();
            if let Some(n) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(n);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// One command line, CRLF included: `command("player/set_volume", &[("pid", "-15"), ("level",
/// "20")])` → `heos://player/set_volume?pid=-15&level=20\r\n`.
pub fn command(path: &str, args: &[(&str, String)]) -> String {
    let mut s = format!("heos://{path}");
    for (k, (name, v)) in args.iter().enumerate() {
        s.push(if k == 0 { '?' } else { '&' });
        s.push_str(name);
        s.push('=');
        s.push_str(&encode_value(v));
    }
    s.push_str("\r\n");
    s
}

/// A message's fields, `pid=1&level=20&under process` → [(pid, 1), (level, 20), (under
/// process, "")], values decoded.
pub fn parse_message(msg: &str) -> Vec<(String, String)> {
    msg.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) => (decode_value(k), decode_value(v)),
            None => (decode_value(p), String::new()),
        })
        .collect()
}

/// One line from a player, parsed.
#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    /// `player/get_volume`, or `event/player_volume_changed` for an event.
    pub command: String,
    /// `result: success` (events, which carry no result, count as ok).
    pub ok: bool,
    pub message: String,
    pub fields: Vec<(String, String)>,
    pub payload: Option<JsonValue>,
}

impl Reply {
    pub fn parse(line: &str) -> Result<Reply, String> {
        let j = JsonValue::parse(line.trim()).map_err(|e| format!("not JSON ({e:?}): {line}"))?;
        let h = j
            .get("heos")
            .ok_or_else(|| format!("no \"heos\" object: {line}"))?;
        let command = h
            .get("command")
            .and_then(JsonValue::as_str)
            .unwrap_or_default()
            .to_string();
        let ok = match h.get("result").and_then(JsonValue::as_str) {
            Some(r) => r == "success",
            None => true,
        };
        let message = h
            .get("message")
            .and_then(JsonValue::as_str)
            .unwrap_or_default()
            .to_string();
        Ok(Reply {
            fields: parse_message(&message),
            command,
            ok,
            message,
            payload: j.get("payload").cloned(),
        })
    }

    pub fn is_event(&self) -> bool {
        self.command.starts_with("event/")
    }

    /// Only an acknowledgement: the answer follows.
    pub fn under_process(&self) -> bool {
        self.message.contains("command under process")
    }

    pub fn field(&self, key: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    pub fn field_i64(&self, key: &str) -> Option<i64> {
        self.field(key).and_then(|v| v.trim().parse().ok())
    }

    /// A failure in words: `ID Not Valid (eid 2)`.
    pub fn error_text(&self) -> String {
        match (self.field("text"), self.field("eid")) {
            (Some(t), Some(e)) => format!("{t} (eid {e})"),
            (Some(t), None) => t.to_string(),
            _ if self.message.is_empty() => "failed".into(),
            _ => self.message.clone(),
        }
    }
}

/// A player's transport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayState {
    Play,
    Pause,
    Stop,
}

impl PlayState {
    pub fn parse(s: &str) -> Option<PlayState> {
        match s.trim().to_ascii_lowercase().as_str() {
            "play" => Some(PlayState::Play),
            "pause" => Some(PlayState::Pause),
            "stop" => Some(PlayState::Stop),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            PlayState::Play => "play",
            PlayState::Pause => "pause",
            PlayState::Stop => "stop",
        }
    }
}

/// What a player is playing (`player/get_now_playing_media`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NowPlaying {
    /// `song` or `station`.
    pub kind: String,
    pub song: String,
    pub artist: String,
    pub album: String,
    pub station: String,
    pub mid: String,
    /// The music source's id (`sid`): 1024 is a URL / local stream.
    pub sid: Option<i64>,
}

impl NowPlaying {
    pub fn parse(payload: &JsonValue) -> NowPlaying {
        let s = |k: &str| {
            payload
                .get(k)
                .map(|v| match v {
                    JsonValue::String(s) => s.clone(),
                    JsonValue::Null => String::new(),
                    other => other.js_stringified(),
                })
                .unwrap_or_default()
        };
        NowPlaying {
            kind: s("type"),
            song: s("song"),
            artist: s("artist"),
            album: s("album"),
            station: s("station"),
            mid: s("mid"),
            sid: payload
                .get("sid")
                .and_then(JsonValue::as_f64)
                .map(|v| v as i64),
        }
    }

    /// One line for people: `Song — Artist`, the station, or empty.
    pub fn line(&self) -> String {
        match (self.song.is_empty(), self.artist.is_empty()) {
            (false, false) if self.song != self.artist => {
                format!("{} — {}", self.song, self.artist)
            }
            (false, _) => self.song.clone(),
            _ if !self.station.is_empty() => self.station.clone(),
            _ => String::new(),
        }
    }
}

/// A player (a zone) and what is known of it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Player {
    pub pid: i64,
    pub name: String,
    pub model: String,
    pub ip: String,
    pub network: String,
    pub version: String,
    /// The group it is in, if any.
    pub gid: Option<i64>,
    pub volume: Option<u8>,
    pub mute: Option<bool>,
    pub state: Option<PlayState>,
    pub now: NowPlaying,
}

fn num(v: &JsonValue, k: &str) -> Option<i64> {
    match v.get(k)? {
        JsonValue::Number(n) => Some(*n as i64),
        JsonValue::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn text(v: &JsonValue, k: &str) -> String {
    v.get(k)
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
        .to_string()
}

/// `player/get_players`' payload.
pub fn parse_players(payload: &JsonValue) -> Vec<Player> {
    payload
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    Some(Player {
                        pid: num(p, "pid")?,
                        name: text(p, "name"),
                        model: text(p, "model"),
                        ip: text(p, "ip"),
                        network: text(p, "network"),
                        version: text(p, "version"),
                        gid: num(p, "gid"),
                        ..Default::default()
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A group of zones playing together; the leader's pid is the group's gid.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Group {
    pub gid: i64,
    pub name: String,
    /// (pid, name, leader), the leader first.
    pub members: Vec<(i64, String, bool)>,
    pub volume: Option<u8>,
    pub mute: Option<bool>,
}

impl Group {
    pub fn leader(&self) -> i64 {
        self.members.iter().find(|m| m.2).map_or(self.gid, |m| m.0)
    }

    pub fn has(&self, pid: i64) -> bool {
        self.members.iter().any(|m| m.0 == pid)
    }
}

/// `group/get_groups`' payload.
pub fn parse_groups(payload: &JsonValue) -> Vec<Group> {
    payload
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|g| {
                    let mut members: Vec<(i64, String, bool)> = g
                        .get("players")
                        .and_then(JsonValue::as_array)
                        .map(|ps| {
                            ps.iter()
                                .filter_map(|p| {
                                    Some((
                                        num(p, "pid")?,
                                        text(p, "name"),
                                        text(p, "role") == "leader",
                                    ))
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    members.sort_by_key(|m| !m.2);
                    Some(Group {
                        gid: num(g, "gid")?,
                        name: text(g, "name"),
                        members,
                        ..Default::default()
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `on` / `off` → bool.
pub fn on_off(s: &str) -> Option<bool> {
    match s.trim() {
        "on" => Some(true),
        "off" => Some(false),
        _ => None,
    }
}

/// A volume as the CLI takes it: 0-100.
pub fn level(v: f64) -> u8 {
    v.round().clamp(0.0, 100.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_encode_only_what_the_cli_reserves() {
        assert_eq!(
            command(
                "player/set_volume",
                &[("pid", "-15".into()), ("level", "20".into())]
            ),
            "heos://player/set_volume?pid=-15&level=20\r\n"
        );
        assert_eq!(
            command("player/get_players", &[]),
            "heos://player/get_players\r\n"
        );
        assert_eq!(
            encode_value("http://10.0.0.2:8790/master.flac?a=1&b=2"),
            "http://10.0.0.2:8790/master.flac?a%3D1%26b%3D2"
        );
        assert_eq!(
            encode_value("Kitchen + Patio 100%"),
            "Kitchen%20+%20Patio%20100%25"
        );
        assert_eq!(
            decode_value("Kitchen%20+%20Patio%20100%25"),
            "Kitchen + Patio 100%"
        );
        assert_eq!(decode_value("50%"), "50%");
        assert_eq!(decode_value("%zz"), "%zz");
    }

    #[test]
    fn replies_and_events_parse() {
        let r = Reply::parse(r#"{"heos": {"command": "player/get_volume", "result": "success", "message": "pid=-1549836569&level=21"}}"#).unwrap();
        assert!(r.ok && !r.is_event() && !r.under_process());
        assert_eq!(r.field_i64("pid"), Some(-1549836569));
        assert_eq!(r.field_i64("level"), Some(21));
        let f = Reply::parse(r#"{"heos": {"command": "player/get_volume", "result": "fail", "message": "eid=2&text=ID Not Valid&pid=1"}}"#).unwrap();
        assert!(!f.ok);
        assert_eq!(f.error_text(), "ID Not Valid (eid 2)");
        let u = Reply::parse(r#"{"heos": {"command": "player/get_players", "result": "success", "message": "command under process"}}"#).unwrap();
        assert!(u.under_process());
        let e = Reply::parse(r#"{"heos": {"command": "event/player_volume_changed", "message": "pid=5&level=30&mute=off"}}"#).unwrap();
        assert!(e.is_event() && e.ok);
        assert_eq!(e.field("mute").and_then(on_off), Some(false));
    }

    #[test]
    fn players_groups_and_now_playing_parse_from_real_replies() {
        let p = Reply::parse(r#"{"heos": {"command": "player/get_players", "result": "success", "message": ""}, "payload": [{"name": "Pool", "pid": -1668439573, "model": "Denon Home Amp", "version": "3.139.170", "ip": "192.0.2.181", "network": "wifi", "lineout": 0, "serial": "X"}, {"name": "Kitchen", "pid": -1549836569, "gid": -1549836569, "model": "Denon Home Amp", "ip": "192.0.2.241", "network": "wifi"}]}"#).unwrap();
        let ps = parse_players(p.payload.as_ref().unwrap());
        assert_eq!(ps.len(), 2);
        assert_eq!(ps[0].name, "Pool");
        assert_eq!(ps[0].pid, -1668439573);
        assert_eq!(ps[0].gid, None);
        assert_eq!(ps[1].gid, Some(-1549836569));
        let g = JsonValue::parse(r#"[{"name": "Kitchen + Patio", "gid": 7, "players": [{"name": "Patio", "pid": 9, "role": "member"}, {"name": "Kitchen", "pid": 7, "role": "leader"}]}]"#).unwrap();
        let gs = parse_groups(&g);
        assert_eq!(gs[0].leader(), 7);
        assert_eq!(gs[0].members[0].1, "Kitchen");
        assert!(gs[0].has(9) && !gs[0].has(1));
        let np = JsonValue::parse(r#"{"type": "station", "song": "Tear in My Heart", "station": "Tear in My Heart", "album": "Blurryface", "artist": "Twenty One Pilots", "mid": "spotify:track:3bn", "qid": 1, "sid": 4}"#).unwrap();
        let n = NowPlaying::parse(&np);
        assert_eq!(n.line(), "Tear in My Heart — Twenty One Pilots");
        assert_eq!(n.sid, Some(4));
        assert_eq!(NowPlaying::default().line(), "");
    }
}
