//! djhost::heos against a mock HEOS player: a TCP server speaking the CLI as the real Denon
//! players do (pids negative, `command under process` before `get_players`' answer, events on
//! the connection that registered for them). On `browse/play_stream` the mock fetches the URL
//! as a player would and checks what it gets is the stream it was promised.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use djhost::heos::proto::parse_message;
use djhost::heos::{Cmd, Format, Heos, PlayState, StreamServer, Target};

#[derive(Clone)]
struct Zone {
    pid: i64,
    name: &'static str,
    volume: i64,
    mute: bool,
    state: &'static str,
    song: String,
}

#[derive(Default)]
struct World {
    zones: Vec<Zone>,
    /// (leader, members) for each group.
    groups: Vec<Vec<i64>>,
    /// Every command line received.
    commands: Vec<String>,
    /// Event connections.
    listeners: Vec<TcpStream>,
    /// What a play_stream fetch got: (url, first bytes).
    fetched: Vec<(String, Vec<u8>)>,
}

struct Mock {
    addr: SocketAddr,
    world: Arc<Mutex<World>>,
}

fn reply(cmd: &str, ok: bool, msg: &str, payload: Option<String>) -> String {
    let r = if ok { "success" } else { "fail" };
    match payload {
        Some(p) => format!(
            r#"{{"heos": {{"command": "{cmd}", "result": "{r}", "message": "{msg}"}}, "payload": {p}}}"#
        ),
        None => {
            format!(r#"{{"heos": {{"command": "{cmd}", "result": "{r}", "message": "{msg}"}}}}"#)
        }
    }
}

fn event(w: &mut World, name: &str, msg: &str) {
    let line =
        format!("{{\"heos\": {{\"command\": \"event/{name}\", \"message\": \"{msg}\"}}}}\r\n");
    w.listeners
        .retain_mut(|s| s.write_all(line.as_bytes()).is_ok());
}

fn fetch(url: &str) -> Vec<u8> {
    let rest = url.strip_prefix("http://").unwrap();
    let (host, path) = rest.split_at(rest.find('/').unwrap());
    let mut s = TcpStream::connect(host).unwrap();
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nIcy-MetaData: 1\r\n\r\n"
    )
    .unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut got = Vec::new();
    let mut buf = [0u8; 8192];
    let t0 = Instant::now();
    // the header and the first FLAC frame (4096 stereo 16-bit frames ≈ 16 KB)
    while got.len() < 40_000 && t0.elapsed() < Duration::from_secs(5) {
        match s.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => got.extend_from_slice(&buf[..n]),
        }
    }
    got
}

impl Mock {
    fn start() -> Mock {
        let world = Arc::new(Mutex::new(World {
            zones: vec![
                Zone {
                    pid: -1549836569,
                    name: "Kitchen",
                    volume: 21,
                    mute: false,
                    state: "play",
                    song: "Song A".into(),
                },
                Zone {
                    pid: -1978493045,
                    name: "Patio",
                    volume: 23,
                    mute: false,
                    state: "stop",
                    song: String::new(),
                },
                Zone {
                    pid: -1668439573,
                    name: "Pool",
                    volume: 25,
                    mute: false,
                    state: "stop",
                    song: String::new(),
                },
            ],
            ..Default::default()
        }));
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let w = world.clone();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let w = w.clone();
                std::thread::spawn(move || Mock::serve(s, w));
            }
        });
        Mock { addr, world }
    }

    fn serve(s: TcpStream, world: Arc<Mutex<World>>) {
        let mut out = s.try_clone().unwrap();
        let r = BufReader::new(s.try_clone().unwrap());
        for line in r.lines() {
            let Ok(line) = line else { return };
            let line = line.trim().to_string();
            let Some(rest) = line.strip_prefix("heos://") else {
                continue;
            };
            let (cmd, q) = rest.split_once('?').unwrap_or((rest, ""));
            let args: BTreeMap<String, String> = parse_message(q).into_iter().collect();
            let mut w = world.lock().unwrap();
            w.commands.push(line.clone());
            let pid = args
                .get("pid")
                .and_then(|p| p.split(',').next()?.parse::<i64>().ok());
            let zi = pid.and_then(|p| w.zones.iter().position(|z| z.pid == p));
            let bad = || {
                reply(
                    cmd,
                    false,
                    &format!(
                        "eid=2&text=ID Not Valid&pid={}",
                        args.get("pid").cloned().unwrap_or_default()
                    ),
                    None,
                )
            };
            let answer = match cmd {
                "system/register_for_change_events" => {
                    if args.get("enable").map(String::as_str) == Some("on") {
                        w.listeners.push(s.try_clone().unwrap());
                    }
                    reply(cmd, true, &format!("enable={}", args["enable"]), None)
                }
                "player/get_players" => {
                    // as the real players: an acknowledgement first
                    let _ = out.write_all(
                        format!("{}\r\n", reply(cmd, true, "command under process", None))
                            .as_bytes(),
                    );
                    let ps: Vec<String> = w
                        .zones
                        .iter()
                        .map(|z| {
                            let gid = w.groups.iter().find(|g| g.contains(&z.pid)).map(|g| format!(", \"gid\": {}", g[0])).unwrap_or_default();
                            format!(r#"{{"name": "{}", "pid": {}, "model": "Denon Home Amp", "version": "3.139.170", "ip": "127.0.0.1", "network": "wifi", "lineout": 0{gid}}}"#, z.name, z.pid)
                        })
                        .collect();
                    reply(cmd, true, "", Some(format!("[{}]", ps.join(", "))))
                }
                "player/get_volume" => match zi {
                    Some(i) => reply(
                        cmd,
                        true,
                        &format!("pid={}&level={}", w.zones[i].pid, w.zones[i].volume),
                        None,
                    ),
                    None => bad(),
                },
                "player/set_volume" | "player/volume_up" | "player/volume_down" => match zi {
                    Some(i) => {
                        let z = &mut w.zones[i];
                        let step: i64 = args.get("step").and_then(|s| s.parse().ok()).unwrap_or(5);
                        z.volume = match cmd {
                            "player/set_volume" => args["level"].parse().unwrap(),
                            "player/volume_up" => (z.volume + step).min(100),
                            _ => (z.volume - step).max(0),
                        };
                        let m = format!(
                            "pid={}&level={}&mute={}",
                            z.pid,
                            z.volume,
                            if z.mute { "on" } else { "off" }
                        );
                        event(&mut w, "player_volume_changed", &m);
                        reply(cmd, true, &q.to_string(), None)
                    }
                    None => bad(),
                },
                "player/get_mute" => match zi {
                    Some(i) => reply(
                        cmd,
                        true,
                        &format!(
                            "pid={}&state={}",
                            w.zones[i].pid,
                            if w.zones[i].mute { "on" } else { "off" }
                        ),
                        None,
                    ),
                    None => bad(),
                },
                "player/set_mute" | "player/toggle_mute" => match zi {
                    Some(i) => {
                        let z = &mut w.zones[i];
                        z.mute = if cmd == "player/toggle_mute" {
                            !z.mute
                        } else {
                            args["state"] == "on"
                        };
                        let m = format!(
                            "pid={}&level={}&mute={}",
                            z.pid,
                            z.volume,
                            if z.mute { "on" } else { "off" }
                        );
                        event(&mut w, "player_volume_changed", &m);
                        reply(cmd, true, q, None)
                    }
                    None => bad(),
                },
                "player/get_play_state" => match zi {
                    Some(i) => reply(
                        cmd,
                        true,
                        &format!("pid={}&state={}", w.zones[i].pid, w.zones[i].state),
                        None,
                    ),
                    None => bad(),
                },
                "player/set_play_state" => match zi {
                    Some(i) => {
                        let st = match args["state"].as_str() {
                            "play" => "play",
                            "pause" => "pause",
                            _ => "stop",
                        };
                        w.zones[i].state = st;
                        let m = format!("pid={}&state={st}", w.zones[i].pid);
                        event(&mut w, "player_state_changed", &m);
                        reply(cmd, true, q, None)
                    }
                    None => bad(),
                },
                "player/get_now_playing_media" => match zi {
                    Some(i) => {
                        let z = &w.zones[i];
                        let p = format!(
                            r#"{{"type": "song", "song": "{0}", "album": "{0}", "artist": "{0}", "image_url": "", "album_id": "1", "mid": "1", "qid": 0, "sid": 1024}}"#,
                            z.song
                        );
                        reply(cmd, true, &format!("pid={}", z.pid), Some(p))
                    }
                    None => bad(),
                },
                "group/get_groups" => {
                    let gs: Vec<String> = w
                        .groups
                        .iter()
                        .map(|g| {
                            let name = g
                                .iter()
                                .map(|p| w.zones.iter().find(|z| z.pid == *p).unwrap().name)
                                .collect::<Vec<_>>()
                                .join(" + ");
                            let ps: Vec<String> = g
                                .iter()
                                .enumerate()
                                .map(|(k, p)| {
                                    let z = w.zones.iter().find(|z| z.pid == *p).unwrap();
                                    format!(
                                        r#"{{"name": "{}", "pid": {}, "role": "{}"}}"#,
                                        z.name,
                                        p,
                                        if k == 0 { "leader" } else { "member" }
                                    )
                                })
                                .collect();
                            format!(
                                r#"{{"name": "{name}", "gid": {}, "players": [{}]}}"#,
                                g[0],
                                ps.join(", ")
                            )
                        })
                        .collect();
                    reply(cmd, true, "", Some(format!("[{}]", gs.join(", "))))
                }
                "group/set_group" => {
                    let pids: Vec<i64> = args["pid"]
                        .split(',')
                        .filter_map(|p| p.parse().ok())
                        .collect();
                    w.groups.retain(|g| !g.iter().any(|p| pids.contains(p)));
                    if pids.len() > 1 {
                        w.groups.push(pids.clone());
                    }
                    event(&mut w, "groups_changed", "");
                    reply(cmd, true, q, None)
                }
                "group/get_volume" => {
                    let gid: i64 = args["gid"].parse().unwrap();
                    let v = w
                        .zones
                        .iter()
                        .find(|z| z.pid == gid)
                        .map_or(0, |z| z.volume);
                    reply(cmd, true, &format!("gid={gid}&level={v}"), None)
                }
                "group/get_mute" => {
                    reply(cmd, true, &format!("gid={}&state=off", args["gid"]), None)
                }
                "group/set_volume" => {
                    let gid: i64 = args["gid"].parse().unwrap();
                    let level: i64 = args["level"].parse().unwrap();
                    let members = w
                        .groups
                        .iter()
                        .find(|g| g[0] == gid)
                        .cloned()
                        .unwrap_or_default();
                    for z in w.zones.iter_mut().filter(|z| members.contains(&z.pid)) {
                        z.volume = level;
                    }
                    event(
                        &mut w,
                        "group_volume_changed",
                        &format!("gid={gid}&level={level}&mute=off"),
                    );
                    reply(cmd, true, q, None)
                }
                "browse/play_stream" => match zi {
                    Some(i) => {
                        let url = args["url"].clone();
                        // fetch it as a player would, outside the lock
                        drop(w);
                        let got = fetch(&url);
                        let mut w = world.lock().unwrap();
                        w.fetched.push((url, got));
                        w.zones[i].state = "play";
                        w.zones[i].song = "Url Stream".into();
                        let m = format!("pid={}", w.zones[i].pid);
                        event(&mut w, "player_now_playing_changed", &m);
                        let _ =
                            out.write_all(format!("{}\r\n", reply(cmd, true, q, None)).as_bytes());
                        continue;
                    }
                    None => bad(),
                },
                _ => reply(cmd, false, "eid=1&text=Unrecognized Command", None),
            };
            drop(w);
            if out.write_all(format!("{answer}\r\n").as_bytes()).is_err() {
                return;
            }
        }
    }

    fn count(&self, prefix: &str) -> usize {
        self.world
            .lock()
            .unwrap()
            .commands
            .iter()
            .filter(|c| c.starts_with(prefix))
            .count()
    }

    /// The HEOS app (or the remote) changes a zone: the event is all the client hears.
    fn outside_volume(&self, name: &str, level: i64) {
        let mut w = self.world.lock().unwrap();
        let z = w.zones.iter_mut().find(|z| z.name == name).unwrap();
        z.volume = level;
        let m = format!("pid={}&level={level}&mute=off", z.pid);
        event(&mut w, "player_volume_changed", &m);
    }
}

fn wait_for(
    h: &Heos,
    what: &str,
    ok: impl Fn(&djhost::heos::Snapshot) -> bool,
) -> djhost::heos::Snapshot {
    let t0 = Instant::now();
    loop {
        let s = h.snapshot();
        if ok(&s) {
            return s;
        }
        assert!(
            t0.elapsed() < Duration::from_secs(10),
            "timed out waiting for {what}: {s:#?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

const KITCHEN: i64 = -1549836569;
const PATIO: i64 = -1978493045;
const POOL: i64 = -1668439573;

fn connected(mock: &Mock) -> Heos {
    let h = Heos::start(Target::Host(mock.addr.to_string()));
    wait_for(&h, "three zones with volumes", |s| {
        s.connected
            && s.players.len() == 3
            && s.players
                .iter()
                .all(|p| p.volume.is_some() && p.state.is_some())
    });
    // the event connection is registered
    let t0 = Instant::now();
    while mock.world.lock().unwrap().listeners.is_empty() {
        assert!(t0.elapsed() < Duration::from_secs(5), "no event connection");
        std::thread::sleep(Duration::from_millis(10));
    }
    h
}

#[test]
fn zones_are_read_and_each_one_s_volume_mute_and_transport_set() {
    let mock = Mock::start();
    let h = connected(&mock);
    let s = h.snapshot();
    let k = s.player(KITCHEN).unwrap();
    assert_eq!(
        (k.name.as_str(), k.volume, k.mute, k.state),
        ("Kitchen", Some(21), Some(false), Some(PlayState::Play))
    );
    assert_eq!(k.now.song, "Song A");
    assert_eq!(s.player(POOL).unwrap().volume, Some(25));

    h.send(Cmd::Volume {
        pid: PATIO,
        level: 40,
    });
    h.send(Cmd::Mute {
        pid: POOL,
        on: None,
    });
    h.send(Cmd::PlayState {
        pid: KITCHEN,
        state: PlayState::Pause,
    });
    let s = wait_for(&h, "the three moves", |s| {
        s.player(PATIO).unwrap().volume == Some(40)
            && s.player(POOL).unwrap().mute == Some(true)
            && s.player(KITCHEN).unwrap().state == Some(PlayState::Pause)
    });
    assert!(s
        .log
        .iter()
        .any(|l| l.ok && l.text.contains("player/set_volume")));
    h.send(Cmd::VolumeStep {
        pid: PATIO,
        delta: -3,
    });
    wait_for(&h, "a step down", |s| {
        s.player(PATIO).unwrap().volume == Some(37)
    });

    // an unknown zone fails, and says so in the log
    h.send(Cmd::Volume {
        pid: 12345,
        level: 10,
    });
    wait_for(&h, "the failure", |s| {
        s.log
            .iter()
            .any(|l| !l.ok && l.text.contains("ID Not Valid"))
    });
}

#[test]
fn a_fader_sweep_reaches_the_zone_as_a_few_moves_ending_where_the_fader_did() {
    let mock = Mock::start();
    let h = connected(&mock);
    for v in 0..=100u8 {
        h.send(Cmd::Volume {
            pid: KITCHEN,
            level: v / 2,
        });
    }
    wait_for(&h, "the end of the sweep", |s| {
        s.player(KITCHEN).unwrap().volume == Some(50)
    });
    let n = mock.count("heos://player/set_volume");
    assert!(n < 101, "{n} set_volumes for 101 moves");
    assert_eq!(mock.world.lock().unwrap().zones[0].volume, 50);
}

#[test]
fn changes_made_elsewhere_arrive_as_events() {
    let mock = Mock::start();
    let h = connected(&mock);
    mock.outside_volume("Pool", 9);
    let s = wait_for(&h, "Pool at 9", |s| {
        s.player(POOL).unwrap().volume == Some(9)
    });
    assert!(s.events > 0);
}

#[test]
fn zones_group_and_ungroup() {
    let mock = Mock::start();
    let h = connected(&mock);
    h.send(Cmd::SetGroup {
        pids: vec![KITCHEN, PATIO],
    });
    let s = wait_for(&h, "a group", |s| s.groups.len() == 1);
    let g = &s.groups[0];
    assert_eq!(g.leader(), KITCHEN);
    assert_eq!(g.name, "Kitchen + Patio");
    assert!(g.has(PATIO) && !g.has(POOL));
    assert_eq!(s.player(PATIO).unwrap().gid, Some(KITCHEN));
    assert_eq!(s.group_of(PATIO).map(|g| g.gid), Some(KITCHEN));
    h.send(Cmd::GroupVolume {
        gid: KITCHEN,
        level: 33,
    });
    wait_for(&h, "group volume", |s| s.groups[0].volume == Some(33));
    assert_eq!(mock.world.lock().unwrap().zones[1].volume, 33);
    h.send(Cmd::SetGroup {
        pids: vec![KITCHEN],
    });
    let s = wait_for(&h, "no groups", |s| s.groups.is_empty());
    assert_eq!(s.player(PATIO).unwrap().gid, None);
}

/// A 440 Hz tone into a capture tap, as the render thread writes the master.
fn tone_tap(stop: Arc<std::sync::atomic::AtomicBool>) -> Arc<djhost::deck::capture::CaptureTap> {
    let tap = Arc::new(djhost::deck::capture::CaptureTap::new());
    let t = tap.clone();
    std::thread::spawn(move || {
        let mut frame = 0i64;
        let block = 480;
        while !stop.load(std::sync::atomic::Ordering::Relaxed) {
            let l: Vec<f32> = (0..block)
                .map(|i| {
                    0.5 * ((frame + i) as f32 * 440.0 * 2.0 * std::f32::consts::PI / 48_000.0).sin()
                })
                .collect();
            t.write(frame, &l, &l);
            frame += block;
            std::thread::sleep(Duration::from_millis(10));
        }
    });
    tap
}

#[test]
fn play_stream_hands_the_zone_a_url_that_serves_the_master_as_flac() {
    let mock = Mock::start();
    let h = connected(&mock);
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let tap = tone_tap(stop.clone());
    let srv = StreamServer::start("127.0.0.1:0".parse().unwrap(), tap, 48_000).unwrap();
    let url = srv.url("127.0.0.1".parse().unwrap(), Format::Flac);
    h.send(Cmd::PlayUrl {
        pid: POOL,
        url: url.clone(),
    });
    let s = wait_for(&h, "Pool playing the stream", |s| {
        let p = s.player(POOL).unwrap();
        p.state == Some(PlayState::Play) && p.now.song == "Url Stream"
    });
    assert!(s
        .log
        .iter()
        .any(|l| l.ok && l.text.contains("browse/play_stream")));
    let w = mock.world.lock().unwrap();
    let (got_url, body) = &w.fetched[0];
    assert_eq!(got_url, &url, "the URL arrives unmangled");
    let head = body
        .windows(4)
        .position(|x| x == b"\r\n\r\n")
        .expect("an HTTP answer")
        + 4;
    assert!(String::from_utf8_lossy(&body[..head]).contains("audio/flac"));
    let flac = &body[head..];
    assert_eq!(&flac[..4], b"fLaC");
    // the first frame: sync, 4096-frame blocks at 48 kHz, stereo 16-bit, frame 0, header CRC
    let f = &flac[42..];
    assert_eq!(&f[..5], &[0xFF, 0xF8, 0xCA, 0x18, 0x00]);
    assert_eq!(f[5], djhost::heos::encode::crc8(&f[..5]));
    let frame_len = 6 + 2 * (1 + 2 * 4096) + 2;
    assert!(
        f.len() >= frame_len,
        "a whole frame arrived ({} bytes)",
        f.len()
    );
    let crc = u16::from_be_bytes([f[frame_len - 2], f[frame_len - 1]]);
    assert_eq!(crc, djhost::heos::encode::crc16(&f[..frame_len - 2]));
    // and it is the tone, not silence
    let peak = f[7..7 + 2 * 4096]
        .chunks(2)
        .map(|b| i16::from_be_bytes([b[0], b[1]]).unsigned_abs())
        .max()
        .unwrap();
    assert!(peak > 10_000, "peak {peak}");
    assert!(srv.stats().served >= 1);
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Two seconds of a tone as the stream's FLAC bytes.
fn flac_bytes(sr: u32) -> (Vec<u8>, Vec<f32>) {
    let mut f = djhost::heos::encode::FlacStream::new(sr);
    let mut out = f.header();
    let n = sr as usize * 2;
    let l: Vec<f32> = (0..n)
        .map(|i| 0.5 * (i as f32 * 440.0 * 2.0 * std::f32::consts::PI / sr as f32).sin())
        .collect();
    let r: Vec<f32> = l.iter().map(|v| -v * 0.5).collect();
    for k in (0..n).step_by(1000) {
        let e = (k + 1000).min(n);
        f.push(&l[k..e], &r[k..e], &mut out);
    }
    (out, l)
}

/// An independent decoder (symphonia's FLAC) reads the stream back sample for sample.
#[cfg(feature = "decode")]
#[test]
fn the_flac_stream_decodes_with_another_decoder() {
    for sr in [44_100u32, 48_000] {
        let (bytes, l) = flac_bytes(sr);
        let dir =
            std::env::temp_dir().join(format!("djhost-heos-flac-{}-{sr}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("tone.flac");
        std::fs::write(&p, &bytes).unwrap();
        let buf = djhost::decode::decode_file(&p, sr as f64).expect("symphonia decodes it");
        let whole = (l.len() / 4096) * 4096;
        assert_eq!(buf.length(), whole, "every whole block, at {sr}");
        for i in (0..whole).step_by(997) {
            let want = djhost::heos::encode::to_i16(l[i]) as f32 / 32768.0;
            assert!((buf.left[i] - want).abs() < 1e-4, "{sr} frame {i}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// `HEOS_FLAC_DUMP=path cargo test -p djhost --test heos_mock -- --ignored` writes the stream
/// for `flac -t` / ffprobe.
#[test]
#[ignore]
fn dump_flac() {
    if let Ok(p) = std::env::var("HEOS_FLAC_DUMP") {
        std::fs::write(p, flac_bytes(48_000).0).unwrap();
    }
}
