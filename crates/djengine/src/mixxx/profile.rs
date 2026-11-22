//! A Mixxx mapping as a djrust device: a [`SurfaceProfile`] the router attaches like any other,
//! the [`Report`] of what mapped and what did not, and the [`Prefilter`] for the options a
//! profile cannot say (`<invert/>`, encoders around 64).
//!
//! The profile has one group per message type and channel (`note-ch1`, `cc-ch1`, …), one control
//! per MIDI number the mapping mentions, so every control the mapping knows is a control of the
//! device (the Map window draws it, MIDI learn and the Bindings window can bind it) whether or
//! not it translated. The `layout` holds the bindings that did.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;

use djrust::control::regex::PortPattern;
use djrust::control::surface::{Binding, ControlType, Group, Leds, SurfaceProfile};
use djrust::json::{JsonObject, JsonValue};

use super::preset::{Control, Preset};
use super::vocab::{translate, Drive, Target, Translation};

/// The prefix of a device made from a Mixxx mapping. A hand-made profile never starts with it.
pub const DEVICE_PREFIX: &str = "Mixxx: ";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// An XML control with `<script-binding/>`: a function of the mapping's script.
    Script,
    /// djrust has nothing to put it on.
    Unsupported,
    /// Another control of the mapping already owns this MIDI message.
    Shadowed,
    /// The control's kind does not suit the target (a fader on a button action).
    Mismatch,
}

impl Why {
    pub fn name(self) -> &'static str {
        match self {
            Why::Script => "script",
            Why::Unsupported => "unsupported",
            Why::Shadowed => "shadowed",
            Why::Mismatch => "mismatch",
        }
    }
}

/// A control of the mapping that did not become a binding.
#[derive(Clone, Debug, PartialEq)]
pub struct Unmapped {
    pub group: String,
    pub key: String,
    pub status: u8,
    pub midino: u8,
    pub why: Why,
    pub reason: String,
}

/// A control that did.
#[derive(Clone, Debug, PartialEq)]
pub struct Mapped {
    pub group: String,
    pub key: String,
    pub status: u8,
    pub midino: u8,
    /// The target's registry name and deck: `play A`, `hotcue A #2`, `crossfader`.
    pub target: String,
    pub approx: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub mapped: Vec<Mapped>,
    pub unmapped: Vec<Unmapped>,
    /// Outputs (LEDs) the mapping lists, and how many were put on the profile's LEDs.
    pub outputs_total: usize,
    pub outputs_lit: usize,
    /// Things worth knowing about the whole mapping (a script it needs, options ignored).
    pub notes: Vec<String>,
}

impl Report {
    pub fn count(&self, why: Why) -> usize {
        self.unmapped.iter().filter(|u| u.why == why).count()
    }

    pub fn to_json(&self, with_lists: bool) -> JsonValue {
        let mut o = JsonObject::new();
        o.set("mapped", JsonValue::int(self.mapped.len() as i64));
        o.set("unmapped", JsonValue::int(self.unmapped.len() as i64));
        let mut by = JsonObject::new();
        for w in [Why::Script, Why::Unsupported, Why::Shadowed, Why::Mismatch] {
            by.set(w.name(), JsonValue::int(self.count(w) as i64));
        }
        o.set("unmapped_by", JsonValue::Object(by));
        o.set("leds", JsonValue::int(self.outputs_lit as i64));
        o.set("outputs", JsonValue::int(self.outputs_total as i64));
        o.set("notes", JsonValue::Array(self.notes.iter().map(|n| JsonValue::str(n.clone())).collect()));
        if with_lists {
            let ctl = |g: &str, k: &str, s: u8, m: u8| {
                format!("{g} {k} ({}", msg_name(s, m)) + ")"
            };
            o.set(
                "mapped_list",
                JsonValue::Array(
                    self.mapped
                        .iter()
                        .map(|m| {
                            let mut e = JsonObject::new();
                            e.set("control", JsonValue::str(ctl(&m.group, &m.key, m.status, m.midino)));
                            e.set("target", JsonValue::str(m.target.clone()));
                            if let Some(a) = &m.approx {
                                e.set("approx", JsonValue::str(a.clone()));
                            }
                            JsonValue::Object(e)
                        })
                        .collect(),
                ),
            );
            o.set(
                "unmapped_list",
                JsonValue::Array(
                    self.unmapped
                        .iter()
                        .map(|u| {
                            let mut e = JsonObject::new();
                            e.set("control", JsonValue::str(ctl(&u.group, &u.key, u.status, u.midino)));
                            e.set("why", JsonValue::str(u.why.name()));
                            e.set("reason", JsonValue::str(u.reason.clone()));
                            JsonValue::Object(e)
                        })
                        .collect(),
                ),
            );
        }
        JsonValue::Object(o)
    }
}

/// `ch1 note 0x07`, for reports.
pub fn msg_name(status: u8, midino: u8) -> String {
    let kind = match status & 0xF0 {
        0x80 | 0x90 => "note",
        0xB0 => "cc",
        0xC0 => "program",
        0xE0 => "bend",
        _ => "msg",
    };
    format!("ch{} {kind} {midino}", (status & 0x0F) + 1)
}

/// A rewrite of one incoming message, for the options a profile cannot express.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fix {
    /// `<invert/>` on a 7-bit control: 127 - v.
    Invert,
    /// An encoder that sends 64 + n: to two's complement (`n & 0x7F`).
    Rot64,
    /// The same, opposite way round.
    Rot64Inv,
}

/// What to do to the messages of one device before the router reads them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Prefilter {
    /// (status nibble, channel, number) → rewrite.
    fixes: BTreeMap<(u8, u8, u8), Fix>,
}

impl Prefilter {
    pub fn is_empty(&self) -> bool {
        self.fixes.is_empty()
    }

    pub fn len(&self) -> usize {
        self.fixes.len()
    }

    /// `bytes` as the profile expects them.
    pub fn apply(&self, bytes: &[u8]) -> [u8; 3] {
        let mut b = [bytes.first().copied().unwrap_or(0), bytes.get(1).copied().unwrap_or(0), bytes.get(2).copied().unwrap_or(0)];
        let kind = b[0] & 0xF0;
        if !matches!(kind, 0x90 | 0x80 | 0xB0) {
            return b;
        }
        if let Some(fix) = self.fixes.get(&(kind, b[0] & 0x0F, b[1])) {
            let v = b[2] as i32;
            b[2] = match fix {
                Fix::Invert => 127 - b[2].min(127),
                Fix::Rot64 => ((v - 64) & 0x7F) as u8,
                Fix::Rot64Inv => ((64 - v) & 0x7F) as u8,
            };
        }
        b
    }
}

/// The result of reading one mapping.
#[derive(Clone, Debug)]
pub struct MixxxProfile {
    pub profile: Arc<SurfaceProfile>,
    pub report: Report,
    pub prefilter: Prefilter,
    /// The mapping's own name (`info/name`), its author and the scripts it wants.
    pub mapping_name: String,
    pub author: String,
    pub scripts: Vec<String>,
}

fn control_type(c: &Control) -> Option<ControlType> {
    Some(match c.kind() {
        0x80 | 0x90 => ControlType::Note,
        0xB0 if c.options.fourteen_bit_msb => ControlType::Cc14,
        0xB0 if c.options.is_relative() => ControlType::Relative,
        0xB0 => ControlType::Cc,
        0xC0 => ControlType::Program,
        0xE0 => ControlType::Pitchbend,
        _ => return None,
    })
}

fn group_id(kind: ControlType, ch: u8) -> String {
    let k = match kind {
        ControlType::Note => "note",
        ControlType::Cc => "cc",
        ControlType::Cc14 => "cc14",
        ControlType::Relative => "rel",
        ControlType::Pitchbend => "bend",
        ControlType::Program => "prog",
    };
    format!("{k}-ch{}", ch + 1)
}

fn target_name(t: &Target) -> String {
    let mut s = t.action.raw().to_string();
    if let Some(d) = &t.deck {
        s.push(' ');
        s.push_str(d);
    }
    if let Some(slot) = t.slot {
        s.push_str(&format!(" #{}", slot + 1));
    }
    if let Some(p) = &t.param {
        s.push(' ');
        s.push_str(p);
    }
    s
}

/// A pattern for a device's port: the port as the OS names it. The mapping's own name is not
/// the port's, so the profile is attached to the port the matcher found (`for_port`), by name.
pub fn device_name(mapping_name: &str) -> String {
    format!("{DEVICE_PREFIX}{mapping_name}")
}

/// Build the device for `preset`, attached to the port called `port`.
pub fn build(preset: &Preset, port: &str) -> MixxxProfile {
    let mut report = Report::default();
    let mut prefilter = Prefilter::default();

    // 1. the controls, in file order; the first to claim a (message, number) owns it
    struct Slot<'a> {
        c: &'a Control,
        kind: ControlType,
    }
    let mut claimed: BTreeMap<(u8, u8, u8), usize> = BTreeMap::new();
    let mut slots: Vec<Slot> = Vec::new();
    let mut lsb_skipped = 0usize;
    for c in &preset.controls {
        let Some(kind) = control_type(c) else {
            report.unmapped.push(Unmapped {
                group: c.group.clone(),
                key: c.key.clone(),
                status: c.status,
                midino: c.midino,
                why: Why::Unsupported,
                reason: "a message type the router does not read (system or aftertouch)".into(),
            });
            continue;
        };
        if c.options.fourteen_bit_lsb {
            // the LSB rides with its MSB control (CC n + 32); it is not a control of its own
            lsb_skipped += 1;
            continue;
        }
        let number = if kind == ControlType::Pitchbend { 0 } else { c.midino };
        let key = (kind as u8, c.channel(), number);
        if let Some(&first) = claimed.get(&key) {
            // note on and note off of one key are one control; two *different* bindings are not
            let f = slots[first].c;
            if (f.group.as_str(), f.key.as_str()) != (c.group.as_str(), c.key.as_str()) {
                report.unmapped.push(Unmapped {
                    group: c.group.clone(),
                    key: c.key.clone(),
                    status: c.status,
                    midino: c.midino,
                    why: Why::Shadowed,
                    reason: format!("{} {} already owns this message", f.group, f.key),
                });
            }
            continue;
        }
        claimed.insert(key, slots.len());
        slots.push(Slot { c, kind });
    }
    if lsb_skipped > 0 {
        report.notes.push(format!("{lsb_skipped} 14-bit LSB control(s) read with their MSB"));
    }

    // 2. groups: (kind, channel) → the numbers in first-seen order, so indexes are stable
    let mut order: Vec<(ControlType, u8)> = Vec::new();
    let mut numbers: BTreeMap<(u8, u8), Vec<u8>> = BTreeMap::new(); // (kind rank, ch)
    let rank = |k: ControlType| k as u8;
    let mut names: BTreeMap<(u8, u8), Vec<String>> = BTreeMap::new();
    for s in &slots {
        let key = (s.kind, s.c.channel());
        if !order.iter().any(|(k, c)| *k == key.0 && *c == key.1) {
            order.push(key);
        }
        let r = (rank(s.kind), s.c.channel());
        let n = if s.kind == ControlType::Pitchbend { 0 } else { s.c.midino };
        numbers.entry(r).or_default().push(n);
        let label = if s.c.options.script_binding {
            format!("{} (script)", s.c.key)
        } else {
            format!("{} {}", s.c.group, s.c.key)
        };
        names.entry(r).or_default().push(label);
    }
    let mut groups: Vec<Group> = Vec::new();
    for (kind, ch) in &order {
        let r = (rank(*kind), *ch);
        groups.push(Group {
            id: group_id(*kind, *ch),
            kind: *kind,
            first: None,
            count: None,
            numbers: Some(numbers[&r].clone()),
            names: Some(names[&r].clone()),
            channel: Some(*ch),
            comment: Some(format!("from the Mixxx mapping \"{}\"", preset.name)),
        });
    }
    let index_of = |s: &Slot| -> usize {
        let n = if s.kind == ControlType::Pitchbend { 0 } else { s.c.midino };
        numbers[&(rank(s.kind), s.c.channel())].iter().position(|x| *x == n).unwrap_or(0)
    };

    // 3. bindings
    let mut layout: Vec<Binding> = Vec::new();
    let mut lit_groups: BTreeSet<String> = BTreeSet::new();
    for s in &slots {
        let c = s.c;
        let gid = group_id(s.kind, c.channel());
        let idx = index_of(s);
        // an encoder that sends 64 + n is read as two's complement (`<selectknob/>` already is)
        if c.kind() == 0xB0 && c.options.rot64 && s.kind == ControlType::Relative {
            let fix = if c.options.rot64_inv { Fix::Rot64Inv } else { Fix::Rot64 };
            prefilter.fixes.insert((0xB0, c.channel(), c.midino), fix);
        }
        let unmapped = |why: Why, reason: String| Unmapped {
            group: c.group.clone(),
            key: c.key.clone(),
            status: c.status,
            midino: c.midino,
            why,
            reason,
        };
        match translate(&c.group, &c.key, c.options.script_binding) {
            Translation::Script => report.unmapped.push(unmapped(
                Why::Script,
                format!("script function {}: needs the mapping's script", c.key),
            )),
            Translation::Unsupported(r) => report.unmapped.push(unmapped(Why::Unsupported, r)),
            Translation::Maps(t) => {
                let relative = s.kind == ControlType::Relative;
                let button = s.kind == ControlType::Note || s.kind == ControlType::Program;
                if relative {
                    report.unmapped.push(unmapped(
                        Why::Mismatch,
                        "an encoder (relative) on a target that takes a position or a press".into(),
                    ));
                    continue;
                }
                if button && t.drive == Drive::Continuous {
                    report.unmapped.push(unmapped(Why::Mismatch, "a button on a fader / knob target".into()));
                    continue;
                }
                if s.kind == ControlType::Cc14 && t.drive == Drive::Trigger {
                    report.unmapped.push(unmapped(Why::Mismatch, "a 14-bit fader on a button target".into()));
                    continue;
                }
                if c.options.invert && matches!(s.kind, ControlType::Cc | ControlType::Note) {
                    let fix = (c.kind() & 0xF0, c.channel(), c.midino);
                    // a note control hears both its note-on and its note-off; only a CC is flipped
                    if s.kind == ControlType::Cc {
                        prefilter.fixes.insert(fix, Fix::Invert);
                    }
                }
                layout.push(Binding {
                    group: gid.clone(),
                    index: Some(idx),
                    action: t.action,
                    deck: t.deck.clone(),
                    slot: t.slot,
                    shift: None,
                    reason: None,
                    role: None,
                    macro_id: None,
                    param: t.param.clone(),
                    args: Default::default(),
                });
                report.mapped.push(Mapped {
                    group: c.group.clone(),
                    key: c.key.clone(),
                    status: c.status,
                    midino: c.midino,
                    target: target_name(&t),
                    approx: t.approx.map(str::to_string),
                });
            }
        }
    }

    // 4. LEDs: a note output whose control is a note of the profile lights that note
    report.outputs_total = preset.outputs.len();
    let mut on: BTreeMap<u8, usize> = BTreeMap::new();
    let mut off: BTreeMap<u8, usize> = BTreeMap::new();
    for o in &preset.outputs {
        if !matches!(o.status & 0xF0, 0x90 | 0x80) {
            continue;
        }
        let gid = group_id(ControlType::Note, o.status & 0x0F);
        if groups.iter().any(|g| g.id == gid) {
            lit_groups.insert(gid);
            *on.entry(o.on).or_default() += 1;
            *off.entry(o.off).or_default() += 1;
            report.outputs_lit += 1;
        }
    }
    let common = |m: &BTreeMap<u8, usize>, d: u8| m.iter().max_by_key(|(_, n)| **n).map(|(v, _)| *v).unwrap_or(d);
    let leds = (!lit_groups.is_empty()).then(|| Leds {
        rgb_groups: Vec::new(),
        single_groups: lit_groups.iter().cloned().collect(),
        rgb_dim_status: 0,
        rgb_full_status: 0,
        rgb_blink_status: None,
        single_off: common(&off, 0),
        single_on: common(&on, 127),
        single_blink: common(&on, 127),
        comment: Some("from the Mixxx mapping's <outputs>; one on / off value for the whole device".into()),
    });
    if report.outputs_total > report.outputs_lit {
        report.notes.push(format!(
            "{} of {} LED output(s) not used (not a note, or its note is not a control)",
            report.outputs_total - report.outputs_lit,
            report.outputs_total
        ));
    }
    if !preset.scripts.is_empty() {
        let n = report.count(Why::Script);
        report.notes.push(format!(
            "the mapping also loads {} script file(s){}",
            preset.scripts.len(),
            if n > 0 { format!("; {n} control(s) call into them") } else { String::new() }
        ));
    }

    let name = device_name(if preset.name.is_empty() { &preset.controller_id } else { &preset.name });
    let profile = SurfaceProfile {
        name,
        manufacturer: "Mixxx mapping".into(),
        match_pattern: PortPattern::literal(port),
        match_comment: Some("the port this mapping was matched to".into()),
        source: Some(format!(
            "Mixxx MIDI mapping \"{}\" by {} (read from the user's own copy; docs/MIXXX-MAPPINGS.md)",
            preset.name, preset.author
        )),
        jog: None,
        groups,
        leds,
        layout,
    };
    MixxxProfile {
        profile: Arc::new(profile),
        report,
        prefilter,
        mapping_name: preset.name.clone(),
        author: preset.author.clone(),
        scripts: preset.scripts.iter().map(|s| s.filename.clone()).collect(),
    }
}
