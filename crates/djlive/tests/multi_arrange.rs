//! Named arrangements (src/flow.rs, docs/BTW.md "Several arrangements"): parse, canonical text, errors, the flow and the
//! length of the song, the CID of old programs, and the order a Live plays them in.

use djlive::compile::check;

fn sections() -> &'static str {
    "tempo 120\nsection intro 1 bars { deck A: kick x ~ ~ ~ }\nsection build 2 bars { deck A: kick x ~ x ~ }\nsection drop 2 bars { deck A: kick x x x x }\nsection outro 1 bars { deck A: snare x ~ ~ ~ }\n"
}

fn compiled(tail: &str) -> djlive::compile::Compiled {
    let src = format!("{}{tail}", sections());
    let c = check(&src);
    assert!(c.diagnostics.iter().all(|d| !d.is_error()), "{:?}", c.diagnostics.iter().map(|d| d.message.clone()).collect::<Vec<_>>());
    c.compiled.expect("compiles")
}

fn errors(tail: &str) -> Vec<String> {
    check(&format!("{}{tail}", sections())).diagnostics.iter().filter(|d| d.is_error()).map(|d| d.message.clone()).collect()
}

fn names(c: &djlive::compile::Compiled) -> Vec<String> {
    c.segments.iter().map(|s| s.name.clone()).collect()
}

#[test]
fn the_flow_is_unrolled_into_one_timeline() {
    let c = compiled("arrange open: intro build then set\narrange set: drop outro loop 2\narrange close: intro end\n");
    assert_eq!(names(&c), ["intro", "build", "drop", "outro", "drop", "outro", "intro"]);
    assert!(!c.looped);
    assert_eq!(c.song_bars(), Some(1 + 2 + 2 * (2 + 1) + 1));
    assert_eq!(c.end, c.song_bars());
    let f = c.flow.as_ref().unwrap();
    assert_eq!(f.order, ["open", "set", "set", "close"]);
    assert!(f.unreachable.is_empty() && f.loop_from.is_none());
}

#[test]
fn loop_repeats_one_arrangement_for_ever_and_the_lap_starts_there() {
    let c = compiled("arrange open: intro build then set\narrange set: drop outro loop\n");
    assert_eq!(names(&c), ["intro", "build", "drop", "outro"]);
    assert!(c.looped);
    assert_eq!(c.loop_from, 3);
    assert_eq!(c.song_bars(), Some(6));
}

#[test]
fn start_picks_the_first_arrangement_and_unreachable_ones_warn() {
    let c = check(&format!("{}arrange open: intro then set\narrange set: drop loop\narrange spare: outro end\nstart set\n", sections()));
    let comp = c.compiled.expect("compiles");
    assert_eq!(names(&comp), ["drop"]);
    let w: Vec<_> = c.diagnostics.iter().filter(|d| !d.is_error()).map(|d| d.message.clone()).collect();
    assert!(w.iter().any(|m| m.contains("`open`") && m.contains("not reachable")), "{w:?}");
    assert!(w.iter().any(|m| m.contains("`spare`")), "{w:?}");
}

#[test]
fn a_then_cycle_is_a_loop_and_is_reported() {
    let c = compiled("arrange left: intro then right\narrange right: build then left\n");
    assert!(c.looped);
    assert_eq!(c.loop_from, 0);
    let f = c.flow.as_ref().unwrap();
    assert!(f.cycle);
    assert_eq!(f.loop_from, Some(0));
}

#[test]
fn falling_through_goes_to_the_next_written_and_the_last_ends() {
    let c = compiled("arrange left: intro\narrange right: build\n");
    assert_eq!(names(&c), ["intro", "build"]);
    assert!(!c.looped);
}

#[test]
fn canonical_text_round_trips_with_the_same_cid() {
    let src = format!("{}arrange   open :  intro build   then   set\narrange set: drop outro loop 3\narrange close: intro end\nstart   open\n", sections());
    let c = check(&src).compiled.unwrap();
    let again = check(&c.text).compiled.unwrap();
    assert_eq!(c.text, again.text);
    assert_eq!(c.cid, again.cid);
    assert!(c.text.contains("arrange open: intro build then set\n"), "{}", c.text);
    assert!(c.text.contains("arrange set: drop outro loop 3\n") && c.text.contains("start open"), "{}", c.text);
    // an arrangement's spelling does not matter, its content does
    let other = check(&src.replace("loop 3", "loop 4")).compiled.unwrap();
    assert_ne!(c.cid, other.cid);
}

#[test]
fn what_is_wrong_is_said_where_it_is() {
    assert!(errors("arrange left: intro then nowhere\n").iter().any(|m| m.contains("there is no arrangement `nowhere`")));
    assert!(errors("arrange left: intro zz end\n").iter().any(|m| m.contains("zz")));
    assert!(errors("arrange left: intro end\nstart right\n").iter().any(|m| m.contains("no arrangement `right`")));
    assert!(errors("arrange left: intro end\narrange left: build end\n").iter().any(|m| m.contains("left")));
    assert!(errors("arrange left: intro end\narrange intro build\n").iter().any(|m| m.contains("not both")));
    assert!(errors("arrange left: intro end\nloop\n").iter().any(|m| m.contains("named arrangements say how they end")));
    assert!(errors("arrange left: intro loop\nend at bar 4\n").iter().any(|m| m.contains("ends or loops")));
    assert!(errors("start left\narrange intro build\n").iter().any(|m| m.contains("names an arrangement")));
    assert!(errors("arrange left: intro loop 0\n").iter().any(|m| m.contains("1 to 256")));
    assert!(errors("arrange left:\n").iter().any(|m| m.contains("needs the sections")));
}

#[test]
fn a_plain_arrange_and_its_loop_are_what_they_were() {
    let plain = "arrange intro build drop\n";
    let c = compiled(&format!("{plain}loop\n"));
    assert!(c.looped && c.loop_from == 0 && c.flow.is_none());
    assert_eq!(c.text.lines().filter(|l| l.starts_with("arrange")).collect::<Vec<_>>(), ["arrange intro build drop"]);
}

#[test]
fn every_example_program_keeps_its_canonical_text_and_cid_through_a_round_trip() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/btw");
    let mut n = 0;
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "btw") {
            let src = std::fs::read_to_string(&p).unwrap();
            if let Some(c) = check(&src).compiled {
                let again = check(&c.text).compiled.expect("canonical text compiles");
                assert_eq!((c.cid, c.text), (again.cid, again.text), "{p:?}");
                n += 1;
            }
        }
    }
    assert!(n > 5);
}
