//! A djnet peer on the command line (docs/NET-PROTOCOL.md): what the phone does, from a shell,
//! for two-process tests against a running djapp.
//!
//! ```sh
//! cargo run -p djhost --features network --example djnet_peer -- discover
//! cargo run … -- clock 127.0.0.1:8795 --code 123456 --secs 10
//! cargo run … -- call 127.0.0.1:8795 deck '{"deck":"A","action":"play"}'
//! ```
//!
//! `--store PATH` keeps the pairing token (default: a file in the temp folder), so a second run
//! needs no code.

use std::time::{Duration, Instant};

use djengine::json::JsonValue;
use djhost::net::{clock::mono_ns, Link, Me, Store};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let pos: Vec<&String> = {
        let mut out = Vec::new();
        let mut skip = false;
        for a in &args {
            if skip {
                skip = false;
                continue;
            }
            if a.starts_with("--") {
                skip = true;
                continue;
            }
            out.push(a);
        }
        out
    };
    let store = flag("--store").unwrap_or_else(|| {
        std::env::temp_dir().join("djnet_peer.json").to_string_lossy().into_owned()
    });
    let me = Me {
        name: flag("--name").unwrap_or_else(|| "djnet_peer".into()),
        app: "djnet_peer/1".into(),
    };
    match pos.first().map(|s| s.as_str()) {
        Some("discover") => {
            for h in djhost::net::ssdp::search(Duration::from_millis(1500)) {
                println!("{}\t{}\t{}\tv{}", h.addr(), h.name, h.id, h.version);
            }
        }
        Some("stream") => stream(&pos, &flag, &args, me, store),
        Some(cmd @ ("clock" | "call" | "track")) => {
            let host = pos.get(1).expect("HOST[:PORT]");
            let addr = djhost::net::client::resolve(host).expect("address");
            let link = Link::start(me, Store::open(Some(store.into())), Box::new(|| None));
            link.connect(addr, flag("--code"));
            let t0 = Instant::now();
            while link.status().state != "connected" {
                let st = link.status();
                if matches!(st.state.as_str(), "unpaired" | "refused") || t0.elapsed() > Duration::from_secs(10) {
                    eprintln!("{}: {}", st.state, st.error.unwrap_or_default());
                    std::process::exit(2);
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            if cmd == "track" {
                // track HOST DECK GENRE SEED [--play] [--tamper]: a composed spec by its CID
                let deck = pos.get(2).expect("DECK");
                let genre = pos.get(3).expect("GENRE");
                let seed: f64 = pos.get(4).and_then(|s| s.parse().ok()).expect("SEED");
                let spec = djengine::TrackSpec::new(genre, seed, None);
                let payload = spec.payload();
                let mut cid = djengine::cid::share_cid(&payload);
                if args.iter().any(|a| a == "--tamper") {
                    cid = djengine::cid::share_cid(&djengine::TrackSpec::new(genre, seed + 1.0, None).payload());
                }
                let params = djengine::json::JsonValue::Object(djengine::json::JsonObject::from_pairs([
                    ("action", "track".into()),
                    ("deck", djengine::json::JsonValue::str(deck.as_str())),
                    ("cid", djengine::json::JsonValue::str(cid.clone())),
                    ("payload", payload),
                    ("play", args.iter().any(|a| a == "--play").into()),
                ]));
                println!("sending {cid}");
                match link.call("net", params) {
                    Ok(v) => println!("{}", v.js_stringified()),
                    Err(e) => {
                        eprintln!("{} [{}]", e.message, e.code);
                        std::process::exit(1);
                    }
                }
                return;
            }
            if cmd == "call" {
                let method = pos.get(2).expect("METHOD");
                let params = pos.get(3).map_or(JsonValue::Object(Default::default()), |p| JsonValue::parse(p).expect("params JSON"));
                match link.call(method, params) {
                    Ok(v) => println!("{}", v.js_stringified()),
                    Err(e) => {
                        eprintln!("{e}");
                        std::process::exit(1);
                    }
                }
                return;
            }
            let secs: u64 = flag("--secs").and_then(|s| s.parse().ok()).unwrap_or(5);
            let end = Instant::now() + Duration::from_secs(secs);
            while Instant::now() < end {
                std::thread::sleep(Duration::from_secs(1));
                let st = link.status();
                let b = st.host_beat.as_ref();
                println!(
                    "t {:>6.3} s  offset {:>9} ns  rtt {:>8} ns  spread {:>7} ns  desktop {}",
                    t0.elapsed().as_secs_f64(),
                    st.offset_ns.unwrap_or(0),
                    st.delay_ns.unwrap_or(0),
                    st.spread_ns.unwrap_or(0),
                    b.map_or("no clock".into(), |b| format!(
                        "{:.2} BPM beat {:.3} ({}{})",
                        b.bpm,
                        b.beat_at(mono_ns()),
                        b.source,
                        if b.playing { ", playing" } else { "" }
                    ))
                );
            }
        }
        _ => {
            eprintln!("djnet_peer stream HOST [--code C] [--secs N] [--ppm P] [--genre G --seed S] [--sync-after S] [--lock] [--log FILE]");
            eprintln!("djnet_peer discover | clock HOST [--code C] [--secs N] | call HOST METHOD [JSON] [--code C] [--store PATH]");
            std::process::exit(2);
        }
    }
}

/// A stand-in phone: an engine playing a track in real time (its clock `--ppm` off this box's,
/// as a phone's crystal is), its master served as `/master.djl` with its beat clock, streamed into
/// the desktop's aux strip. Logs the desktop's view of the stream every second: its state, the
/// phase against the desktop master, the buffer, the drift correction and the latency.
fn stream(
    pos: &[&String],
    flag: &dyn Fn(&str) -> Option<String>,
    args: &[String],
    me: Me,
    store: String,
) {
    use djhost::deck::{engine, EngineConfig};
    use djhost::heos::stream::{MasterStreamSource, StreamServer};
    use djhost::net::clock::{master_point_dated, FrameClock};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::io::Write as _;

    let host = pos.get(1).expect("HOST[:PORT]");
    let addr = djhost::net::client::resolve(host).expect("address");
    let secs: u64 = flag("--secs").and_then(|s| s.parse().ok()).unwrap_or(30);
    let ppm: f64 = flag("--ppm").and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let genre = flag("--genre").unwrap_or_else(|| "techno".into());
    let seed: f64 = flag("--seed").and_then(|s| s.parse().ok()).unwrap_or(7.0);
    let sync_after: f64 = flag("--sync-after").and_then(|s| s.parse().ok()).unwrap_or(5.0);
    let lock = args.iter().any(|a| a == "--lock");
    let mut log = flag("--log").map(|p| std::fs::File::create(p).expect("log file"));

    let rate = 48_000.0;
    let mut cfg = EngineConfig::new(rate);
    cfg.extended = false;
    let (mut e, mut r) = engine(cfg);
    assert!(e.generate("A", &djengine::TrackSpec::new(&genre, seed, None)), "compose");
    e.set_crossfader(0.0);
    e.play("A");
    // the render thread: real time, `ppm` fast
    let stop = Arc::new(AtomicBool::new(false));
    let flag_stop = stop.clone();
    let render = std::thread::spawn(move || {
        let block = 256usize;
        let (mut l, mut rr) = (vec![0f32; block], vec![0f32; block]);
        let t0 = Instant::now();
        let mut done = 0u64;
        let real_rate = rate * (1.0 + ppm * 1e-6);
        while !flag_stop.load(Ordering::Relaxed) {
            r.render_block(&mut l, &mut rr);
            done += block as u64;
            let due = Duration::from_secs_f64(done as f64 / real_rate);
            if let Some(w) = due.checked_sub(t0.elapsed()) {
                std::thread::sleep(w);
            }
        }
    });
    let cell: djhost::heos::stream::ClockCell = Arc::new(Mutex::new(None));
    let srv = StreamServer::start_with_clock("127.0.0.1:0".parse().unwrap(), e.master_stream_source(), rate as u32, cell.clone())
        .expect("stream server");
    let url = format!("http://127.0.0.1:{}/master.djl", srv.port());
    let mine = cell.clone();
    let link = Link::start(me, Store::open(Some(store.into())), Box::new(move || mine.lock().ok().and_then(|m| m.clone())));
    link.connect(addr, flag("--code"));
    let mut fc = FrameClock::default();
    let t0 = Instant::now();
    let mut started = false;
    let mut synced = false;
    let mut next_log = Instant::now();
    let p = |s: &str| JsonValue::parse(s).unwrap();
    while t0.elapsed() < Duration::from_secs(secs) {
        std::thread::sleep(Duration::from_millis(10));
        let bp = master_point_dated(&e, 0, &mut fc);
        if let Ok(mut c) = cell.lock() {
            *c = Some(bp);
        }
        e.drain_retired();
        if link.status().state != "connected" {
            continue;
        }
        if !started {
            match link.call("net", JsonValue::Object(djengine::json::JsonObject::from_pairs([
                ("action", "stream".into()),
                ("url", JsonValue::str(url.clone())),
            ]))) {
                Ok(v) => println!("stream: {}", v.js_stringified()),
                Err(e) => {
                    eprintln!("stream: {e}");
                    std::process::exit(1);
                }
            }
            started = true;
        }
        let t = t0.elapsed().as_secs_f64();
        if !synced && t > sync_after {
            match link.call("net", p(r#"{"action":"aux_sync"}"#)) {
                Ok(v) => println!("{t:7.2} sync: {}", v.js_stringified()),
                Err(e) => println!("{t:7.2} sync: {e}"),
            }
            if lock {
                let _ = link.call("net", p(r#"{"action":"aux_lock","on":true}"#));
            }
            synced = true;
        }
        if Instant::now() >= next_log {
            next_log += Duration::from_secs(1);
            let Ok(s) = link.call("net", p(r#"{"action":"stream_status"}"#)) else { continue };
            let g = |k: &str| s.get(k).cloned().unwrap_or(JsonValue::Null);
            let lat = g("latency");
            let l = |k: &str| lat.get(k).and_then(JsonValue::as_f64);
            let f = |v: Option<f64>| v.map_or("—".into(), |x| format!("{x:.2}"));
            let line = format!(
                "{t:7.2} {:>10} bpm {:>7} phase {:>7} buf {:>7}/{:>6} corr {:>8} ppm  e2e {:>7} ms  transit {:>6} ms  underruns {} jumps {} reconnects {}",
                g("state").as_str().unwrap_or("?"),
                f(g("bpm").as_f64()),
                f(g("phase").as_f64()),
                f(l("buffer_ms")),
                f(l("target_ms")),
                f(g("correction_ppm").as_f64()),
                f(l("render_to_render_ms")),
                f(l("transit_ms")),
                g("underruns").as_f64().unwrap_or(0.0),
                g("jumps").as_f64().unwrap_or(0.0),
                g("reconnects").as_f64().unwrap_or(0.0),
            );
            println!("{line}");
            if let Some(f) = log.as_mut() {
                let _ = writeln!(f, "{}", s.js_stringified().replacen('{', &format!("{{\"t\":{t:.3},"), 1));
            }
        }
    }
    let _ = link.call("net", p(r#"{"action":"stream_stop"}"#));
    stop.store(true, Ordering::Relaxed);
    let _ = render.join();
    drop(srv);
}
