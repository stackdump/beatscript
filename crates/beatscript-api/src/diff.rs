//! The semantic diff of two programs that are not the same song (`same_as`'s `diff`): what changed, said in music
//! rather than text, in three layers.
//!
//! 1. **Song**: each song-level setting that differs, old -> new.
//! 2. **Arrangement**: the two section sequences aligned (a scored LCS: a section matches one with the same nets,
//!    else one with the same name), then each section labelled `renamed`, `changed`, `inserted`, `removed`,
//!    `moved` (it is in both, out of order) or `repeated` (B plays an A section once more).
//! 3. **Parts**: within sections that match (and an inserted section against the A section it most resembles),
//!    deck by deck and role by role. The same net CID is unchanged; otherwise the known transforms are tried on
//!    the notes: `transpose` by k semitones, `octave` by n, `rotate` by n steps, `reverse`, `fast` / `slow` by n,
//!    `thin` (a strict subset of the hits) and `fill` (a strict superset). A transform that explains the change is
//!    reported; else a step-aligned note diff (`added`, `removed`, `changed`, each at `bar:step`, 1-based and
//!    counted from the section's first bar). Only velocities, lengths or accents: `dynamics`.
//!
//! Every change carries `summary`, one line a person can read ("drop2 bass = drop bass transposed +1"); the
//! diff's own `summary` is all of them in order.

use std::collections::{BTreeMap, BTreeSet};

use djengine::json::{JsonObject, JsonValue};
use djlive::compile::{Compiled, DeckIr, PartIr};
use djlive::perform::note_name;

use crate::report::{int, obj, opt_str, strs};

/// The most hits listed per kind in a note diff (the counts are always whole).
const MAX_LISTED: usize = 32;
/// The longest stretch, in sixteenth steps, two parts are compared over (64 bars: one net).
const MAX_STEPS: u32 = 64 * 16;

/// One section of a program's timeline: its stretches (an evolving section's generations), each a deck -> net.
struct Sec {
    name: Option<String>,
    start: u32,
    bars: u32,
    segs: Vec<BTreeMap<String, DeckIr>>,
    sig: String,
}

/// What a section plays: each stretch's decks, each deck's length and its parts' fingerprints. Not the net CIDs:
/// the tempo, swing and key are in a net too, and those are the song layer's.
fn sig_of(segs: &[BTreeMap<String, DeckIr>]) -> String {
    segs.iter()
        .map(|m| {
            m.iter()
                .map(|(d, ir)| {
                    format!(
                        "{d}/{}={}",
                        ir.period,
                        ir.fingerprints()
                            .iter()
                            .map(|(r, f)| format!("{r}:{f}"))
                            .collect::<Vec<_>>()
                            .join("+")
                    )
                })
                .collect::<Vec<_>>()
                .join(",")
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// The sections of a program: an arranged song's timeline (a chained stretch belongs to the section before it),
/// or one section for a program without one.
fn sections_of(c: &Compiled) -> Vec<Sec> {
    let named = |m: &BTreeMap<djlive::ast::Deck, DeckIr>| {
        m.iter()
            .map(|(d, ir)| (d.name().to_string(), ir.clone()))
            .collect::<BTreeMap<_, _>>()
    };
    let mut out: Vec<Sec> = Vec::new();
    if c.arranged {
        for s in &c.segments {
            match out.last_mut() {
                Some(last) if s.chain => {
                    last.bars += s.bars;
                    last.segs.push(named(&s.decks));
                }
                _ => out.push(Sec {
                    name: Some(s.name.clone()),
                    start: s.start,
                    bars: s.bars,
                    segs: vec![named(&s.decks)],
                    sig: String::new(),
                }),
            }
        }
    } else {
        let bars = c
            .decks
            .values()
            .map(|ir| ir.period as u32)
            .max()
            .unwrap_or(0);
        out.push(Sec {
            name: None,
            start: 0,
            bars,
            segs: vec![named(&c.decks)],
            sig: String::new(),
        });
    }
    for s in &mut out {
        s.sig = sig_of(&s.segs);
    }
    out
}

/// The longhand of a program (one net per deck for the whole song) as one section, when it has one.
fn longhand_section(c: &Compiled) -> Option<Sec> {
    let nets = c.song_nets().ok()?;
    let m: BTreeMap<String, DeckIr> = nets
        .into_iter()
        .map(|(d, ir)| (d.name().to_string(), ir))
        .collect();
    let bars = m.values().map(|ir| ir.period as u32).max().unwrap_or(0);
    let segs = vec![m];
    let sig = sig_of(&segs);
    Some(Sec {
        name: None,
        start: 0,
        bars,
        segs,
        sig,
    })
}

fn sec_name(s: &Sec) -> String {
    s.name.clone().unwrap_or_else(|| "the program".to_string())
}

fn sec_json(s: &Sec) -> JsonValue {
    obj([
        ("name", opt_str(s.name.clone())),
        ("start_bar", int(s.start as usize)),
        ("bars", int(s.bars as usize)),
    ])
}

// ---------- layer 3: parts ----------

/// A hit as compared: (step from the section's start, note).
type Key = (u32, u8);

/// How a hit is played: (dur, vel, accent, slide, its expression marks).
type Play = (u8, u8, bool, bool, djlive::ast::Marks);

/// A part's hits over `bars` bars of its deck: (step, note) -> how it is played.
fn unroll(p: &PartIr, bars: usize) -> BTreeMap<Key, Play> {
    let n = p.cycles.len().max(1);
    let mut m = BTreeMap::new();
    for bar in 0..bars {
        if let Some(hits) = p.cycles.get(bar % n) {
            for h in hits {
                m.insert(
                    ((bar * 16) as u32 + u32::from(h.step), h.note),
                    (h.dur, h.vel, h.accent, h.slide, h.marks),
                );
            }
        }
    }
    m
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

fn lcm(a: u32, b: u32) -> u32 {
    if a == 0 || b == 0 {
        a.max(b)
    } else {
        a / gcd(a, b) * b
    }
}

/// The set moved round by `n` steps in a loop of `len`.
fn shift(s: &BTreeSet<Key>, n: u32, len: u32) -> BTreeSet<Key> {
    s.iter().map(|&(p, k)| ((p + n) % len, k)).collect()
}

/// The shortest loop the hits repeat in (a divisor of `len`), and the hits of one turn of it.
fn min_cycle(s: &BTreeSet<Key>, len: u32) -> (u32, BTreeSet<Key>) {
    for d in 1..=len {
        if len % d == 0 && shift(s, d, len) == *s {
            return (d, s.iter().filter(|(p, _)| *p < d).copied().collect());
        }
    }
    (len, s.clone())
}

/// One turn of a cycle `c` (length `p`) repeated to `len`.
fn repeat(c: &BTreeSet<Key>, p: u32, len: u32) -> BTreeSet<Key> {
    let mut out = BTreeSet::new();
    let mut o = 0;
    while o < len {
        out.extend(c.iter().map(|&(q, k)| (q + o, k)));
        o += p;
    }
    out
}

/// A part's full hits repeated to `len` steps.
fn repeat_map(
    m: &BTreeMap<Key, Play>,
    p: u32,
    len: u32,
) -> BTreeMap<Key, Play> {
    let mut out = BTreeMap::new();
    let mut o = 0;
    while o < len {
        out.extend(m.iter().map(|(&(q, k), v)| ((q + o, k), *v)));
        o += p;
    }
    out
}

/// "a bar", "4 bars": how long a stretch of `steps` sixteenths is.
fn per(steps: u32) -> String {
    let bars = steps.div_ceil(16);
    if bars == 1 {
        "per bar".to_string()
    } else {
        format!("in {bars} bars")
    }
}

/// One transform: (kind, by, words).
type Tf = (&'static str, i64, String);

/// Every known transform that turns `a` into `b`, the plainest first (`la` / `lb`: the parts' lengths in steps).
/// For a regular pulse several fit (halving a four-on-the-floor kick is `thin` and `slow 2`); the first is the
/// answer, the rest go in `also`.
fn transforms(a: &BTreeSet<Key>, la: u32, b: &BTreeSet<Key>, lb: u32) -> Vec<Tf> {
    let mut out = Vec::new();
    if a.is_empty() || b.is_empty() {
        return out;
    }
    // thin / fill, over the two parts' own lengths: hits taken out or added, every other hit where it was
    let len = lcm(la.max(1), lb.max(1));
    if len <= MAX_STEPS * 4 {
        let (ra, rb) = (repeat(a, la.max(1), len), repeat(b, lb.max(1), len));
        if rb.is_subset(&ra) && rb.len() < ra.len() {
            out.push((
                "thin",
                (ra.len() - rb.len()) as i64,
                format!(
                    "thinned ({} of {} hits {} taken out)",
                    ra.len() - rb.len(),
                    ra.len(),
                    per(len)
                ),
            ));
        }
        if ra.is_subset(&rb) && ra.len() < rb.len() {
            out.push((
                "fill",
                (rb.len() - ra.len()) as i64,
                format!(
                    "filled ({} hits added to {} {})",
                    rb.len() - ra.len(),
                    ra.len(),
                    per(len)
                ),
            ));
        }
    }
    if let Some(t) = transform(a, la, b, lb) {
        out.push(t);
    }
    // fast / slow as well, when thin / fill said it first
    out
}

/// The first of transpose, octave, rotate, reverse, fast, slow that turns `a` into `b`.
fn transform(a: &BTreeSet<Key>, la: u32, b: &BTreeSet<Key>, lb: u32) -> Option<Tf> {
    let (pa, ca) = min_cycle(a, la);
    let (pb, cb) = min_cycle(b, lb);
    let len = lcm(pa, pb);
    if len > MAX_STEPS * 4 {
        return None;
    }
    let (ea, eb) = (repeat(&ca, pa, len), repeat(&cb, pb, len));
    // transpose / octave: the same steps, every note moved by the same interval
    if ea.len() == eb.len() {
        let (va, vb): (Vec<Key>, Vec<Key>) =
            (ea.iter().copied().collect(), eb.iter().copied().collect());
        if va.iter().zip(&vb).all(|(x, y)| x.0 == y.0) {
            let k = i64::from(vb[0].1) - i64::from(va[0].1);
            if k != 0
                && va
                    .iter()
                    .zip(&vb)
                    .all(|(x, y)| i64::from(y.1) - i64::from(x.1) == k)
            {
                return Some(if k % 12 == 0 {
                    (
                        "octave",
                        k / 12,
                        format!(
                            "an octave {} ({:+} octave{})",
                            if k > 0 { "up" } else { "down" },
                            k / 12,
                            if k.abs() == 12 { "" } else { "s" }
                        ),
                    )
                } else {
                    ("transpose", k, format!("transposed {k:+}"))
                });
            }
        }
        // rotate: the same notes, every step moved by the same amount round the loop
        if let Some(&(b0, bk)) = vb.first() {
            let mut best: Option<i64> = None;
            for &(p, k) in &ea {
                if k != bk {
                    continue;
                }
                let n = (b0 + len - p) % len;
                if n != 0 && shift(&ea, n, len) == eb {
                    let signed = if n > len / 2 {
                        i64::from(n) - i64::from(len)
                    } else {
                        i64::from(n)
                    };
                    if best.is_none_or(|b| signed.abs() < b.abs()) {
                        best = Some(signed);
                    }
                }
            }
            if let Some(n) = best {
                let w = format!("rotated {n:+} step{}", if n.abs() == 1 { "" } else { "s" });
                return Some(("rotate", n, w));
            }
        }
        // reverse: played backwards
        let rev: BTreeSet<Key> = ea.iter().map(|&(p, k)| (len - 1 - p, k)).collect();
        if rev == eb {
            return Some(("reverse", 0, "reversed".to_string()));
        }
    }
    // fast / slow: the same cycle squeezed or stretched by n
    for n in [2u32, 3, 4, 8] {
        // squeezed steps land where the engine puts them: rounded half up (step 3 of `fast(p, 2)` plays on 2)
        if pa == n * pb
            && ca
                .iter()
                .map(|&(p, k)| ((2 * p + n) / (2 * n), k))
                .collect::<BTreeSet<_>>()
                == cb
        {
            return Some(("fast", i64::from(n), format!("{n} times as fast")));
        }
        if pb == n * pa && ca.iter().map(|&(p, k)| (p * n, k)).collect::<BTreeSet<_>>() == cb {
            return Some(("slow", i64::from(n), format!("{n} times as slow")));
        }
    }
    None
}

fn at(p: u32) -> String {
    format!("{}:{}", p / 16 + 1, p % 16 + 1)
}

/// The step-aligned note diff of two parts over a common length: added, removed and changed notes.
fn note_diff(a: &BTreeSet<Key>, la: u32, b: &BTreeSet<Key>, lb: u32) -> (JsonValue, String) {
    let len = lcm(la.max(1), lb.max(1)).min(MAX_STEPS);
    let ext = |s: &BTreeSet<Key>, l: u32| -> BTreeMap<u32, Vec<u8>> {
        let mut m: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
        if l == 0 {
            return m;
        }
        let mut o = 0;
        while o < len {
            for &(p, k) in s {
                if p + o < len {
                    m.entry(p + o).or_default().push(k);
                }
            }
            o += l;
        }
        m
    };
    let (ma, mb) = (ext(a, la), ext(b, lb));
    let steps: BTreeSet<u32> = ma.keys().chain(mb.keys()).copied().collect();
    let (mut added, mut removed, mut changed) = (Vec::new(), Vec::new(), Vec::new());
    for p in steps {
        let empty = Vec::new();
        let (na, nb) = (ma.get(&p).unwrap_or(&empty), mb.get(&p).unwrap_or(&empty));
        let only_a: Vec<u8> = na.iter().filter(|k| !nb.contains(k)).copied().collect();
        let only_b: Vec<u8> = nb.iter().filter(|k| !na.contains(k)).copied().collect();
        if only_a.len() == 1 && only_b.len() == 1 {
            changed.push((p, only_a[0], only_b[0]));
        } else {
            removed.extend(only_a.into_iter().map(|k| (p, k)));
            added.extend(only_b.into_iter().map(|k| (p, k)));
        }
    }
    let list = |v: &[(u32, u8)]| {
        JsonValue::Array(
            v.iter()
                .take(MAX_LISTED)
                .map(|&(p, k)| {
                    obj([
                        ("at", JsonValue::str(at(p))),
                        ("note", JsonValue::str(note_name(k))),
                    ])
                })
                .collect(),
        )
    };
    let json = obj([
        ("steps", int(len as usize)),
        ("added", list(&added)),
        ("removed", list(&removed)),
        (
            "changed",
            JsonValue::Array(
                changed
                    .iter()
                    .take(MAX_LISTED)
                    .map(|&(p, x, y)| {
                        obj([
                            ("at", JsonValue::str(at(p))),
                            ("old", JsonValue::str(note_name(x))),
                            ("new", JsonValue::str(note_name(y))),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("added_count", int(added.len())),
        ("removed_count", int(removed.len())),
        ("changed_count", int(changed.len())),
    ]);
    let mut words = Vec::new();
    let plural = |n: usize, w: &str| format!("{n} {w}{}", if n == 1 { "" } else { "s" });
    if !changed.is_empty() {
        let first = changed
            .iter()
            .take(3)
            .map(|&(p, x, y)| format!("{} {} -> {}", at(p), note_name(x), note_name(y)))
            .collect::<Vec<_>>()
            .join(", ");
        words.push(format!(
            "{} changed ({first}{})",
            plural(changed.len(), "note"),
            if changed.len() > 3 { ", ..." } else { "" }
        ));
    }
    for (v, w) in [(&added, "added"), (&removed, "removed")] {
        if !v.is_empty() {
            let first = v
                .iter()
                .take(3)
                .map(|&(p, k)| format!("{} {}", at(p), note_name(k)))
                .collect::<Vec<_>>()
                .join(", ");
            words.push(format!(
                "{} {w} ({first}{})",
                plural(v.len(), "note"),
                if v.len() > 3 { ", ..." } else { "" }
            ));
        }
    }
    (json, words.join("; "))
}

fn sound_of(p: &PartIr) -> String {
    let mut s = p
        .instrument
        .clone()
        .unwrap_or_else(|| "default".to_string());
    if let Some(k) = &p.kit {
        s.push_str(&format!(" kit={k}"));
    }
    if let Some(x) = &p.sound {
        s.push_str(&format!(" sound={x}"));
    }
    s
}

/// What happened to the right copy of a part with a `jux` (added, dropped, or changed as any part is), said as a part's is,
/// with `copy: right`. None when the copies play the same.
fn twin_change(
    pa: Option<&PartIr>,
    la: usize,
    pb: Option<&PartIr>,
    lb: usize,
) -> Option<(JsonValue, String)> {
    let (ta, tb) = (pa.and_then(|p| p.twin.as_deref()), pb.and_then(|p| p.twin.as_deref()));
    let (mut j, words) = compare_part(ta, la, tb, lb)?;
    if let JsonValue::Object(o) = &mut j {
        o.set("copy", JsonValue::str("right"));
    }
    Some((j, format!("right copy {words}")))
}

/// What happened to one part: None when it plays the same. `label` is how the summary names it.
fn compare_part(
    pa: Option<&PartIr>,
    la: usize,
    pb: Option<&PartIr>,
    lb: usize,
) -> Option<(JsonValue, String)> {
    match (pa, pb) {
        (None, None) => None,
        (None, Some(p)) => Some((
            obj([
                ("change", JsonValue::str("added")),
                ("instrument", opt_str(p.instrument.clone())),
            ]),
            "added".to_string(),
        )),
        (Some(p), None) => Some((
            obj([
                ("change", JsonValue::str("removed")),
                ("instrument", opt_str(p.instrument.clone())),
            ]),
            "removed".to_string(),
        )),
        (Some(a), Some(b)) => {
            let (ha, hb) = (unroll(a, la), unroll(b, lb));
            let (sa, sb) = (sound_of(a), sound_of(b));
            if sa == sb && la == lb && ha == hb && a.expr == b.expr {
                return None;
            }
            let (ka, kb): (BTreeSet<Key>, BTreeSet<Key>) =
                (ha.keys().copied().collect(), hb.keys().copied().collect());
            let (sla, slb) = ((la * 16) as u32, (lb * 16) as u32);
            let mut o = JsonObject::new();
            let mut words = Vec::new();
            if sa != sb {
                o.set(
                    "instrument",
                    obj([
                        ("old", JsonValue::str(sa.clone())),
                        ("new", JsonValue::str(sb.clone())),
                    ]),
                );
                words.push(format!("instrument {sa} -> {sb}"));
            }
            // the same notes on the same steps (over a common loop): only how they are played changed
            let same_notes = {
                let len = lcm(sla.max(1), slb.max(1));
                len <= MAX_STEPS * 4 && repeat(&ka, sla.max(1), len) == repeat(&kb, slb.max(1), len)
            };
            if same_notes {
                let len = lcm(sla.max(1), slb.max(1));
                if repeat_map(&ha, sla.max(1), len) != repeat_map(&hb, slb.max(1), len)
                    || a.expr != b.expr
                {
                    o.set("change", JsonValue::str("dynamics"));
                    words.push(
                        "the same notes, played differently (velocity, length, accent or expression)"
                            .to_string(),
                    );
                } else if words.is_empty() {
                    // the same loop said over a different length: nothing to hear
                    return None;
                } else {
                    o.set("change", JsonValue::str("instrument"));
                }
            } else if let Some(((kind, by, w), rest)) = transforms(&ka, sla, &kb, slb)
                .split_first()
                .map(|(f, r)| (f.clone(), r.to_vec()))
            {
                o.set("change", JsonValue::str("transform"));
                o.set(
                    "transform",
                    obj([("kind", JsonValue::str(kind)), ("by", JsonValue::int(by))]),
                );
                if !rest.is_empty() {
                    o.set(
                        "also",
                        JsonValue::Array(
                            rest.iter()
                                .map(|(k, b, _)| {
                                    obj([("kind", JsonValue::str(*k)), ("by", JsonValue::int(*b))])
                                })
                                .collect(),
                        ),
                    );
                }
                words.push(w);
            } else {
                let (nd, w) = note_diff(&ka, sla, &kb, slb);
                o.set("change", JsonValue::str("notes"));
                o.set("notes", nd);
                words.push(w);
            }
            Some((JsonValue::Object(o), words.join("; ")))
        }
    }
}

/// The part changes between two sections, stretch by stretch, deck by deck, role by role.
fn compare_sections(a: &Sec, b: &Sec) -> Vec<JsonValue> {
    let mut out = Vec::new();
    let renamed = a.name != b.name;
    for (g, (ma, mb)) in a.segs.iter().zip(b.segs.iter()).enumerate() {
        let decks: BTreeSet<&String> = ma.keys().chain(mb.keys()).collect();
        // a role on more than one deck is named with its deck
        let mut role_decks: BTreeMap<&str, usize> = BTreeMap::new();
        for d in &decks {
            let roles: BTreeSet<&str> = ma
                .get(*d)
                .into_iter()
                .chain(mb.get(*d))
                .flat_map(|ir| ir.parts.iter().map(|p| p.role.as_str()))
                .collect();
            for r in roles {
                *role_decks.entry(r).or_default() += 1;
            }
        }
        for d in decks {
            let (da, db) = (ma.get(d), mb.get(d));
            if let (Some(x), Some(y)) = (da, db) {
                if x.period == y.period && x.fingerprints() == y.fingerprints() {
                    continue;
                }
            }
            let roles: BTreeSet<&str> = da
                .into_iter()
                .chain(db)
                .flat_map(|ir| ir.parts.iter().map(|p| p.role.as_str()))
                .collect();
            for role in roles {
                let part = |ir: Option<&DeckIr>| {
                    ir.and_then(|ir| ir.parts.iter().find(|p| p.role == role).cloned())
                };
                let (pa, pb) = (part(da), part(db));
                let (la, lb) = (da.map_or(0, |i| i.period), db.map_or(0, |i| i.period));
                // a part's own change, else the change of its right copy (a `jux` added, dropped or changed)
                let Some((mut j, words)) = compare_part(pa.as_ref(), la, pb.as_ref(), lb)
                    .or_else(|| twin_change(pa.as_ref(), la, pb.as_ref(), lb))
                else {
                    continue;
                };
                let who = if role_decks.get(role).copied().unwrap_or(0) > 1 {
                    format!("{role} ({d})")
                } else {
                    role.to_string()
                };
                let gen = if a.segs.len() > 1 || b.segs.len() > 1 {
                    format!(" gen {g}")
                } else {
                    String::new()
                };
                let summary = if renamed && (words == "added" || words == "removed") {
                    let w = if words == "added" {
                        format!("added (not in {})", sec_name(a))
                    } else {
                        format!("dropped ({} has it)", sec_name(a))
                    };
                    format!("{}{gen} {who}: {w}", sec_name(b))
                } else if renamed {
                    format!(
                        "{}{gen} {who} = {}{gen} {who} {words}",
                        sec_name(b),
                        sec_name(a)
                    )
                } else {
                    format!("{}{gen} {who}: {words}", sec_name(b))
                };
                if let JsonValue::Object(o) = &mut j {
                    o.set("a_section", opt_str(a.name.clone()));
                    o.set("b_section", opt_str(b.name.clone()));
                    o.set("deck", JsonValue::str(d.clone()));
                    o.set("role", JsonValue::str(role));
                    if a.segs.len() > 1 || b.segs.len() > 1 {
                        o.set("gen", int(g));
                    }
                    o.set("summary", JsonValue::str(summary));
                }
                out.push(j);
            }
        }
    }
    if a.segs.len() != b.segs.len() {
        out.push(obj([
            ("a_section", opt_str(a.name.clone())),
            ("b_section", opt_str(b.name.clone())),
            ("change", JsonValue::str("generations")),
            (
                "summary",
                JsonValue::str(format!(
                    "{}: {} generation(s) -> {}",
                    sec_name(b),
                    a.segs.len(),
                    b.segs.len()
                )),
            ),
        ]));
    }
    out
}

/// How much of `b` the section `a` explains: parts that are the same or one transform away.
fn resemblance(a: &Sec, b: &Sec) -> usize {
    let (Some(ma), Some(mb)) = (a.segs.first(), b.segs.first()) else {
        return 0;
    };
    let mut n = 0;
    for (d, y) in mb {
        let Some(x) = ma.get(d) else { continue };
        for p in &y.parts {
            let Some(q) = x.parts.iter().find(|q| q.role == p.role) else {
                continue;
            };
            match compare_part(Some(q), x.period, Some(p), y.period) {
                None => n += 2,
                Some((j, _)) if j.get("change").and_then(|c| c.as_str()) == Some("transform") => {
                    n += 1
                }
                _ => {}
            }
        }
    }
    n
}

// ---------- layer 2: the arrangement ----------

/// Matched pairs (i in a, j in b), order kept: a scored LCS, the same nets scoring above the same name.
fn align(a: &[Sec], b: &[Sec]) -> Vec<(usize, usize)> {
    let score = |x: &Sec, y: &Sec| -> Option<i32> {
        let names = x.name.is_some() && x.name == y.name;
        let w = x.bars.max(1) as i32;
        if x.sig == y.sig {
            Some(w * if names { 4 } else { 3 })
        } else if names {
            Some(w)
        } else {
            None
        }
    };
    let (n, m) = (a.len(), b.len());
    let mut dp = vec![vec![0i32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            let mut best = dp[i + 1][j].max(dp[i][j + 1]);
            if let Some(s) = score(&a[i], &b[j]) {
                best = best.max(s + dp[i + 1][j + 1]);
            }
            dp[i][j] = best;
        }
    }
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < n && j < m {
        if let Some(s) = score(&a[i], &b[j]) {
            if dp[i][j] == s + dp[i + 1][j + 1] {
                out.push((i, j));
                i += 1;
                j += 1;
                continue;
            }
        }
        if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

fn arrangement(a: &[Sec], b: &[Sec]) -> (Vec<JsonValue>, Vec<JsonValue>, usize) {
    let pairs = align(a, b);
    let mut a_used: BTreeSet<usize> = pairs.iter().map(|p| p.0).collect();
    let b_used: BTreeSet<usize> = pairs.iter().map(|p| p.1).collect();
    let mut events: Vec<(usize, JsonValue)> = Vec::new(); // (b position, event); removals sort last
    let mut parts = Vec::new();
    let mut unchanged = 0;
    let ev = |change: &str, x: Option<&Sec>, y: Option<&Sec>, summary: String| {
        obj([
            ("change", JsonValue::str(change)),
            ("a", x.map_or(JsonValue::Null, sec_json)),
            ("b", y.map_or(JsonValue::Null, sec_json)),
            ("summary", JsonValue::str(summary)),
        ])
    };
    for &(i, j) in &pairs {
        let (x, y) = (&a[i], &b[j]);
        if x.sig == y.sig {
            if x.name == y.name {
                unchanged += 1;
            } else {
                events.push((
                    j,
                    ev(
                        "renamed",
                        Some(x),
                        Some(y),
                        format!("renamed {} -> {}", sec_name(x), sec_name(y)),
                    ),
                ));
            }
        } else {
            let ps = compare_sections(x, y);
            if ps.is_empty() && x.name == y.name {
                unchanged += 1;
                continue;
            }
            let what = if x.name == y.name {
                format!("{} changed", sec_name(y))
            } else {
                format!("renamed {} -> {} and changed", sec_name(x), sec_name(y))
            };
            events.push((
                j,
                ev(
                    "changed",
                    Some(x),
                    Some(y),
                    format!(
                        "{what} ({} part{})",
                        ps.len(),
                        if ps.len() == 1 { "" } else { "s" }
                    ),
                ),
            ));
            parts.extend(ps);
        }
    }
    // what the alignment left: out of order (moved), played again (repeated), new (inserted)
    for (j, y) in b.iter().enumerate() {
        if b_used.contains(&j) {
            continue;
        }
        let free_same = (0..a.len()).find(|i| !a_used.contains(i) && a[*i].sig == y.sig);
        let free_name = (0..a.len())
            .find(|i| !a_used.contains(i) && a[*i].name.is_some() && a[*i].name == y.name);
        if let Some(i) = free_same.or(free_name) {
            a_used.insert(i);
            let x = &a[i];
            let mut s = format!(
                "moved {}: bar {} -> bar {}",
                sec_name(y),
                x.start + 1,
                y.start + 1
            );
            if x.name != y.name {
                s.push_str(&format!(" (was {})", sec_name(x)));
            }
            if x.sig != y.sig {
                let ps = compare_sections(x, y);
                s.push_str(&format!(
                    ", and changed ({} part{})",
                    ps.len(),
                    if ps.len() == 1 { "" } else { "s" }
                ));
                parts.extend(ps);
            }
            events.push((j, ev("moved", Some(x), Some(y), s)));
        } else if let Some(i) = (0..a.len()).find(|i| a[*i].sig == y.sig) {
            events.push((
                j,
                ev(
                    "repeated",
                    Some(&a[i]),
                    Some(y),
                    format!("repeated {} at bar {}", sec_name(&a[i]), y.start + 1),
                ),
            ));
        } else {
            // inserted: described against the A section it most resembles, when one explains any of it
            let like = (0..a.len())
                .map(|i| (resemblance(&a[i], y), a[i].bars == y.bars, i))
                .filter(|r| r.0 > 0)
                .max_by_key(|r| (r.0, r.1, std::cmp::Reverse(r.2)));
            let mut s = format!(
                "inserted {} ({} bars) at bar {}",
                sec_name(y),
                y.bars,
                y.start + 1
            );
            let mut e = ev("inserted", None, Some(y), String::new());
            if let Some((_, _, i)) = like {
                s.push_str(&format!(", a variant of {}", sec_name(&a[i])));
                if let JsonValue::Object(o) = &mut e {
                    o.set("like", sec_json(&a[i]));
                }
                parts.extend(compare_sections(&a[i], y));
            }
            if let JsonValue::Object(o) = &mut e {
                o.set("summary", JsonValue::str(s));
            }
            events.push((j, e));
        }
    }
    for (i, x) in a.iter().enumerate() {
        if a_used.contains(&i) {
            continue;
        }
        let again = b.iter().any(|y| y.sig == x.sig);
        let s = if again {
            format!(
                "removed a repeat of {} (was at bar {})",
                sec_name(x),
                x.start + 1
            )
        } else {
            format!(
                "removed {} ({} bars, was at bar {})",
                sec_name(x),
                x.bars,
                x.start + 1
            )
        };
        events.push((usize::MAX, ev("removed", Some(x), None, s)));
    }
    events.sort_by_key(|e| e.0);
    (events.into_iter().map(|e| e.1).collect(), parts, unchanged)
}

// ---------- the whole diff ----------

/// The diff of `x` (a) against `y` (b). `song` is same_as's song-level differences ({field: {a, b}}).
pub(crate) fn diff(x: &Compiled, y: &Compiled, song: &JsonObject) -> JsonValue {
    let mut summary: Vec<String> = Vec::new();
    // 1. the song
    let mut song_changes = Vec::new();
    for (k, v) in song.iter() {
        let (old, new) = (
            v.get("a").cloned().unwrap_or(JsonValue::Null),
            v.get("b").cloned().unwrap_or(JsonValue::Null),
        );
        let show = |j: &JsonValue| match j {
            JsonValue::Null => "none".to_string(),
            JsonValue::String(s) => s.clone(),
            other => other.js_stringified(),
        };
        let s = format!("{k} {} -> {}", show(&old), show(&new));
        let echo = k == "end_bar"
            && song
                .get("song_bars")
                .is_some_and(|sb| sb.get("a") == Some(&old) && sb.get("b") == Some(&new));
        if !echo {
            summary.push(s.clone());
        }
        song_changes.push(obj([
            ("field", JsonValue::str(k)),
            ("old", old),
            ("new", new),
            ("summary", JsonValue::str(s)),
        ]));
    }
    // 2 and 3. the arrangement and the parts: section by section when both have sections (or neither does); one
    // with sections and one without are compared as the whole song written out longhand
    let (arr, parts, unchanged, basis) = if x.arranged == y.arranged {
        let (a, b) = (sections_of(x), sections_of(y));
        let (arr, parts, unchanged) = arrangement(&a, &b);
        (arr, parts, unchanged, "sections")
    } else {
        match (longhand_section(x), longhand_section(y)) {
            (Some(a), Some(b)) => (
                Vec::new(),
                if a.sig == b.sig {
                    Vec::new()
                } else {
                    compare_sections(&a, &b)
                },
                0,
                "song_nets",
            ),
            _ => {
                summary.push("one program has sections and the other has none, and one of them has no longhand to compare".to_string());
                (Vec::new(), Vec::new(), 0, "none")
            }
        }
    };
    // an arrangement change's own line, then its parts' lines
    for e in &arr {
        if let Some(s) = e.get("summary").and_then(|s| s.as_str()) {
            summary.push(s.to_string());
        }
    }
    for p in &parts {
        if let Some(s) = p.get("summary").and_then(|s| s.as_str()) {
            summary.push(s.to_string());
        }
    }
    obj([
        ("basis", JsonValue::str(basis)),
        ("summary", strs(summary)),
        ("song", JsonValue::Array(song_changes)),
        ("arrangement", JsonValue::Array(arr)),
        ("unchanged_sections", int(unchanged)),
        ("parts", JsonValue::Array(parts)),
    ])
}
