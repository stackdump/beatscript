//! A Mixxx MIDI mapping (`*.midi.xml`), as data.
//!
//! This is a reader for a file format (what the XML says), written from the format's public
//! description. It copies nothing from Mixxx's own sources or from any mapping. The files it
//! reads are GPL and stay out of this repo: they are read from the user's own Mixxx install or
//! a download of theirs (docs/MIXXX-MAPPINGS.md).
//!
//! ```text
//! <MixxxMIDIPreset>
//!   <info><name/><author/><description/>…</info>
//!   <controller id="…">
//!     <scriptfiles><file filename="x.js" functionprefix="X"/></scriptfiles>
//!     <controls><control><group/><key/><status/><midino/><options>…</options></control>…</controls>
//!     <outputs><output><group/><key/><status/><midino/><on/><off/><minimum/><maximum/></output>…</outputs>
//!   </controller>
//! </MixxxMIDIPreset>
//! ```

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::import::xml::{self, XmlNode};

/// What a control's options say. `normal` is the default and has no field.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Options {
    pub invert: bool,
    /// `<script-binding/>`: the value goes to a script function (`key` is its name), not to a
    /// Mixxx control.
    pub script_binding: bool,
    pub soft_takeover: bool,
    /// `<diff/>`, `<selectknob/>`: relative, two's complement (0x01 = +1, 0x7F = -1).
    pub relative: bool,
    /// `<rot64/>`, `<rot64-inv/>`, `<rot64-fast/>`, `<spread64/>`, `<hercjog/>`: relative around 64.
    pub rot64: bool,
    pub rot64_inv: bool,
    pub fourteen_bit_msb: bool,
    pub fourteen_bit_lsb: bool,
    pub button: bool,
    pub switch: bool,
    /// Option names this reader does not know, as written.
    pub other: Vec<String>,
}

impl Options {
    fn add(&mut self, name: &str) {
        match name.to_ascii_lowercase().replace('_', "-").as_str() {
            "normal" => {}
            "invert" => self.invert = true,
            "script-binding" => self.script_binding = true,
            "soft-takeover" => self.soft_takeover = true,
            "diff" | "selectknob" => self.relative = true,
            "rot64" | "rot64-fast" | "spread64" | "hercjog" => self.rot64 = true,
            "rot64-inv" => {
                self.rot64 = true;
                self.rot64_inv = true;
            }
            "fourteen-bit-msb" => self.fourteen_bit_msb = true,
            "fourteen-bit-lsb" => self.fourteen_bit_lsb = true,
            "button" => self.button = true,
            "switch" => self.switch = true,
            other => self.other.push(other.to_string()),
        }
    }

    /// Does the control send a relative value (an encoder), not a position?
    pub fn is_relative(&self) -> bool {
        self.relative || self.rot64
    }
}

/// One `<control>`: a MIDI message and what it drives.
#[derive(Clone, Debug, PartialEq)]
pub struct Control {
    /// `[Channel1]`, `[Master]`, …
    pub group: String,
    /// `play`, `hotcue_3_activate`, or a script function's name with `script-binding`.
    pub key: String,
    pub status: u8,
    pub midino: u8,
    pub options: Options,
    pub description: String,
}

impl Control {
    /// 0-based MIDI channel.
    pub fn channel(&self) -> u8 {
        self.status & 0x0F
    }
    /// The message type: 0x80 note off, 0x90 note on, 0xB0 CC, 0xC0 program, 0xE0 bend.
    pub fn kind(&self) -> u8 {
        self.status & 0xF0
    }
}

/// One `<output>`: the message sent to light an LED when a control is on.
#[derive(Clone, Debug, PartialEq)]
pub struct Output {
    pub group: String,
    pub key: String,
    pub status: u8,
    pub midino: u8,
    /// Data byte for on and for off (defaults 0x7F and 0x00).
    pub on: u8,
    pub off: u8,
    pub minimum: f64,
    pub maximum: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScriptFile {
    pub filename: String,
    pub prefix: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Preset {
    pub name: String,
    pub author: String,
    pub description: String,
    /// `<controller id="…">`: the device's name, as the mapping's author knew it.
    pub controller_id: String,
    pub scripts: Vec<ScriptFile>,
    pub controls: Vec<Control>,
    pub outputs: Vec<Output>,
}

/// `0x91`, `0X91`, `145`, with whitespace round it.
fn number(text: &str) -> Option<u8> {
    let t = text.trim();
    let n = if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        u32::from_str_radix(h, 16).ok()?
    } else {
        t.parse::<u32>().ok()?
    };
    u8::try_from(n).ok()
}

fn text(n: &XmlNode, child: &str) -> String {
    n.child(child).map(|c| c.text.trim().to_string()).unwrap_or_default()
}

fn options_of(c: &XmlNode) -> Options {
    let mut o = Options::default();
    // `<options><normal/></options>`, or the option right under `<control>` (both occur)
    if let Some(opts) = c.child("options") {
        for n in &opts.children {
            o.add(&n.name);
        }
    }
    for n in &c.children {
        if !matches!(n.name.as_str(), "group" | "key" | "status" | "midino" | "description" | "options") {
            o.add(&n.name);
        }
    }
    o
}

/// A few mappings declare their own entities in the DOCTYPE (`<!ENTITY noteOn "0x90">`) and write
/// `&noteOn;` where a value goes. The XML reader knows none, so the internal ones are replaced
/// here, once, as text. (External entities are never read.)
fn expand_entities(text: &str) -> String {
    let Some(start) = text.find("<!DOCTYPE") else {
        return text.to_string();
    };
    let Some(end_rel) = text[start..].find("]>") else {
        return text.to_string();
    };
    let decl = &text[start..start + end_rel];
    let mut defs: Vec<(String, String)> = Vec::new();
    let mut rest = decl;
    while let Some(i) = rest.find("<!ENTITY") {
        rest = &rest[i + 8..];
        let t = rest.trim_start();
        let name_end = t.find(char::is_whitespace).unwrap_or(0);
        let name = &t[..name_end];
        let after = t[name_end..].trim_start();
        let Some(q) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else {
            continue; // SYSTEM / PUBLIC: external, never read
        };
        if let Some(close) = after[1..].find(q) {
            defs.push((name.to_string(), after[1..1 + close].to_string()));
        }
    }
    let mut out = text[start + end_rel + 2..].to_string();
    out.insert_str(0, &text[..start]);
    for (n, v) in defs {
        out = out.replace(&alloc::format!("&{n};"), &v);
    }
    out
}

/// Parse a mapping. A control with no readable status or midino is left out; a file that is
/// not a Mixxx MIDI preset is an error.
pub fn parse(data: &[u8]) -> Result<Preset, String> {
    // some mappings are Latin-1: read them as text anyway
    let owned;
    let bytes: &[u8] = match core::str::from_utf8(data) {
        Ok(_) => data,
        Err(_) => {
            owned = String::from_utf8_lossy(data).into_owned();
            owned.as_bytes()
        }
    };
    let expanded = expand_entities(core::str::from_utf8(bytes).unwrap_or(""));
    let root = xml::parse(expanded.as_bytes())?;
    // `MixxxControllerPreset` is the newer name of the same root
    if root.name != "MixxxMIDIPreset" && root.name != "MixxxControllerPreset" {
        return Err("not a Mixxx mapping (no <MixxxMIDIPreset> / <MixxxControllerPreset>)".to_string());
    }
    let mut p = Preset::default();
    if let Some(info) = root.child("info") {
        p.name = text(info, "name");
        p.author = text(info, "author");
        p.description = text(info, "description");
    }
    let Some(ctl) = root.child("controller") else {
        return Ok(p);
    };
    p.controller_id = ctl.attr("id").unwrap_or("").trim().to_string();
    if let Some(sf) = ctl.child("scriptfiles") {
        for f in sf.all("file") {
            if let Some(name) = f.attr("filename") {
                p.scripts.push(ScriptFile {
                    filename: name.to_string(),
                    prefix: f.attr("functionprefix").unwrap_or("").to_string(),
                });
            }
        }
    }
    if let Some(cs) = ctl.child("controls") {
        for c in cs.all("control") {
            let (Some(status), Some(midino)) = (
                c.child("status").and_then(|n| number(&n.text)),
                c.child("midino").and_then(|n| number(&n.text)),
            ) else {
                continue;
            };
            p.controls.push(Control {
                group: text(c, "group"),
                key: text(c, "key"),
                status,
                midino,
                options: options_of(c),
                description: text(c, "description"),
            });
        }
    }
    if let Some(os) = ctl.child("outputs") {
        for o in os.all("output") {
            let (Some(status), Some(midino)) = (
                o.child("status").and_then(|n| number(&n.text)),
                o.child("midino").and_then(|n| number(&n.text)),
            ) else {
                continue;
            };
            p.outputs.push(Output {
                group: text(o, "group"),
                key: text(o, "key"),
                status,
                midino,
                on: o.child("on").and_then(|n| number(&n.text)).unwrap_or(0x7F),
                off: o.child("off").and_then(|n| number(&n.text)).unwrap_or(0),
                minimum: o.child("minimum").and_then(|n| n.double()).unwrap_or(0.0),
                maximum: o.child("maximum").and_then(|n| n.double()).unwrap_or(1.0),
            });
        }
    }
    if p.name.is_empty() {
        p.name = p.controller_id.clone();
    }
    Ok(p)
}
