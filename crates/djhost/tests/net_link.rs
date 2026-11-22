//! djnet host ↔ peer over real TCP on this box (docs/NET-PROTOCOL.md §4-§6, §9-§10):
//! pairing by code, re-auth by token, the clock, the allowlist, and a reconnect after the host
//! restarts.
#![cfg(feature = "network")]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use djengine::json::{JsonObject, JsonValue};
use djhost::control_socket::Handler;
use djhost::net::{clock::mono_ns, BeatPoint, Host, HostConfig, Link, Me, Store, Trust};

fn handler(seen: Arc<Mutex<Vec<String>>>) -> Arc<Handler> {
    Arc::new(move |m: &str, p: &JsonValue| {
        seen.lock().unwrap().push(format!("{m} {}", p.js_stringified()));
        Ok(JsonValue::Object(JsonObject::from_pairs([("did", JsonValue::str(m))])))
    })
}

fn host(port: u16, store: Store, seen: Arc<Mutex<Vec<String>>>) -> Host {
    Host::start(
        HostConfig {
            bind: format!("127.0.0.1:{port}").parse().unwrap(),
            name: "desk".into(),
            app: "test/1".into(),
            trust: Trust::Code,
            store,
            caps: vec!["clock".into(), "remote".into()],
            strip: "Phone".into(),
        },
        handler(seen),
    )
    .unwrap()
}

fn wait_for(link: &Link, state: &str) {
    let t0 = Instant::now();
    while link.status().state != state {
        assert!(t0.elapsed() < Duration::from_secs(8), "never {state}: {:?}", link.status());
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn p(s: &str) -> JsonValue {
    JsonValue::parse(s).unwrap()
}

#[test]
fn pair_by_code_then_token_clock_allowlist_and_reconnect() {
    let tmp = std::env::temp_dir().join(format!("djnet-it-{}", std::process::id()));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let port = free_port();
    let h = host(port, Store::open(Some(tmp.join("host.json"))), seen.clone());
    let addr = h.addr;
    // the host plays 120 BPM from beat 0 at its t = 0
    h.publish_clock(BeatPoint {
        bpm: 120.0,
        beat: 0.0,
        t: 0,
        frame: 0,
        rate: 48_000.0,
        playing: true,
        source: "A".into(),
        out_ns: 0,
    });
    let me = Me { name: "phone".into(), app: "test/1".into() };
    let peer_store = Store::open(Some(tmp.join("peer.json")));
    let link = Link::start(me.clone(), peer_store, Box::new(|| None));

    // no code: unpaired, and it says so
    link.connect(addr, None);
    wait_for(&link, "unpaired");
    assert!(link.status().error.unwrap().contains("pairing code"));
    assert!(link.call("deck", p(r#"{"deck":"A","action":"play"}"#)).is_err());

    // a wrong code, then the right one
    link.connect(addr, Some("000000x".into()));
    wait_for(&link, "unpaired");
    link.connect(addr, Some(h.pair_code()));
    wait_for(&link, "connected");
    assert_eq!(h.paired().len(), 1);

    // requests go to the handler; the allowlist holds
    let r = link.call("deck", p(r#"{"deck":"A","action":"play"}"#)).unwrap();
    assert_eq!(r.get("did").and_then(JsonValue::as_str), Some("deck"));
    let e = link.call("config", p(r#"{"action":"set"}"#)).unwrap_err();
    assert_eq!(e.code, "forbidden");
    let e = link.call("deck", p(r#"{"deck":"A","action":"file","path":"/x"}"#)).unwrap_err();
    assert_eq!(e.code, "forbidden");
    assert_eq!(seen.lock().unwrap().len(), 1, "{:?}", seen.lock().unwrap());

    // the clock: same box, so the offset is ~0 and the host's beat maps through it
    let t0 = Instant::now();
    while link.status().host_beat.is_none() && t0.elapsed() < Duration::from_secs(3) {
        std::thread::sleep(Duration::from_millis(20));
    }
    let st = link.status();
    let off = st.offset_ns.unwrap();
    assert!(off.abs() < 2_000_000, "offset {off} ns on loopback");
    let (beat, bpm) = link.host_beat_at(mono_ns()).unwrap();
    assert_eq!(bpm, 120.0);
    let want = mono_ns() as f64 * 2.0 / 1e9;
    assert!((beat - want).abs() < 0.01, "beat {beat} vs {want}");
    eprintln!("loopback clock: offset {off} ns, delay {} ns", st.delay_ns.unwrap());

    // the host restarts on the same port with the same store: the link comes back by token
    drop(h);
    std::thread::sleep(Duration::from_millis(100));
    let _ = link.call("ping", p("{}")); // finds the dead connection
    let h2 = host(port, Store::open(Some(tmp.join("host.json"))), seen.clone());
    assert_ne!(h2.pair_code(), "", "a new code");
    wait_for(&link, "connected");
    assert!(link.call("transport", p(r#"{"action":"state"}"#)).is_ok());

    // forgotten on the desktop: unpaired again until a code
    assert_eq!(h2.forget("all"), 1);
    drop(h2);
    std::thread::sleep(Duration::from_millis(100));
    let _ = link.call("ping", p("{}"));
    let _h3 = host(port, Store::open(Some(tmp.join("host.json"))), seen);
    wait_for(&link, "unpaired");
    let _ = std::fs::remove_dir_all(tmp);
}

#[test]
fn five_wrong_codes_lock_pairing() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let h = host(free_port(), Store::open(None), seen);
    let store = Mutex::new(Store::open(None));
    let me = Me { name: "x".into(), app: "t".into() };
    for k in 0..5 {
        let e = djhost::net::Conn::open(h.addr, &me, &store, Some("999999")).err().unwrap();
        assert_eq!(e.code, if h.pair_code() == "999999" { "x" } else { "bad_code" }, "try {k}");
    }
    let e = djhost::net::Conn::open(h.addr, &me, &store, Some(&h.pair_code())).err().unwrap();
    assert_eq!(e.code, "locked_out");
    h.new_code(); // lifts it
    assert!(djhost::net::Conn::open(h.addr, &me, &store, Some(&h.pair_code())).is_ok());
}
