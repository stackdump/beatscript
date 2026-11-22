//! The public-domain theme library (`data/themes-pd.txt`): melodies, basses and figures of music composed and first
//! published before 1929, each written as BeatScript bars in a stated key, the melodic twin of the phrasebook
//! (`vocal::phoneme::pd_phrases`). One CC0 file; its CID is the CID of its text, as the phrasebook's is. Read by the form
//! templates (djlive `template.rs`, the `$THEME_<ID>` slots), by beatscript.app's `/themes` and by the MCP tools
//! (`list_examples` with `themes`, `genre_template` with `theme`). Pure: the file is compiled in.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use once_cell::race::OnceBox;

static THEMES_TEXT: &str = include_str!("../data/themes-pd.txt");

/// The roles a theme is written for: the closed vocabulary of the file's `role` column, with what each is.
pub const ROLES: [(&str, &str); 4] = [
    ("melody", "a tune: the line a person hums"),
    ("bass", "a bass line, a ground or an ostinato: the line under the others"),
    ("harmony", "chords, or an interval held: what the other lines are heard against"),
    ("arp", "a chord broken into a figure: the harmony as a moving line"),
];

/// The modes of the file's `mode` column.
pub const MODES: [(&str, &str); 3] = [("major", "in a major key"), ("minor", "in a minor key"), ("modal", "neither: a pedal or an ostinato on one note")];

/// A theme of the library: its bars, the key and meter they are in, and where they are from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theme {
    /// What a program names (`ode_to_joy`); its bars are `ode_to_joy_1` ...
    pub id: String,
    /// One of [`ROLES`]' names.
    pub role: String,
    /// The key the bars are written in, a BeatScript key word (`d`, `db`).
    pub key: String,
    /// One of [`MODES`]' names.
    pub mode: String,
    /// The time signature as written (`2/4`).
    pub meter: String,
    /// The sixteenths a bar of the pattern takes: 16, 12 (3/4) or 20 (5/4).
    pub steps: u32,
    /// The bars, each one bar of `steps` terms.
    pub bars: Vec<String>,
    pub work: String,
    pub composer: String,
    /// Composed, or first published when that is the earlier date known: before 1929.
    pub year: u16,
    /// The themes it morphs well with.
    pub pairs: Vec<String>,
    /// How it is written, and the dates.
    pub note: String,
}

impl Theme {
    /// The name of bar `i` (0-based): `ode_to_joy_1` for the first.
    pub fn bar_name(&self, i: usize) -> String {
        format!("{}_{}", self.id, i + 1)
    }

    /// The theme as `let` lines: a line a bar, then the id as their alternation (`let ode_to_joy = <ode_to_joy_1 ...>`).
    pub fn lets(&self) -> String {
        let mut s = String::new();
        for (i, b) in self.bars.iter().enumerate() {
            s.push_str(&format!("let {} = {b}\n", self.bar_name(i)));
        }
        let names: Vec<String> = (0..self.bars.len()).map(|i| self.bar_name(i)).collect();
        s.push_str(&format!("let {} = <{}>", self.id, names.join(" ")));
        s
    }

    /// The cycle mark a part plays it with: `""` for a 16-step bar, `/12` or `/20` otherwise.
    pub fn cycle_mark(&self) -> String {
        if self.steps == 16 {
            String::new()
        } else {
            format!("/{}", self.steps)
        }
    }

    /// The part that plays it: `melody ode_to_joy`, `bass/20 mars_ostinato`.
    pub fn part(&self) -> String {
        format!("{}{} {}", self.role, self.cycle_mark(), self.id)
    }
}

fn data_lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("version "))
}

fn parse(l: &str) -> Option<Theme> {
    let f: Vec<&str> = l.split(" | ").map(str::trim).collect();
    if f.len() != 12 {
        return None;
    }
    let list = |s: &str, sep: &str| -> Vec<String> { s.split(sep).map(str::trim).filter(|x| !x.is_empty()).map(str::to_string).collect() };
    Some(Theme {
        id: f[0].to_string(),
        role: f[1].to_string(),
        key: f[2].to_string(),
        mode: f[3].to_string(),
        meter: f[4].to_string(),
        steps: f[5].parse().ok()?,
        bars: list(f[6], ";"),
        work: f[7].to_string(),
        composer: f[8].to_string(),
        year: f[9].parse().ok()?,
        pairs: list(f[10], ","),
        note: f[11].to_string(),
    })
}

static TABLE: OnceBox<Vec<Theme>> = OnceBox::new();

/// Every theme, in the file's order.
pub fn themes() -> &'static [Theme] {
    TABLE.get_or_init(|| Box::new(data_lines(THEMES_TEXT).filter_map(parse).collect()))
}

/// The theme `id` names.
pub fn theme(id: &str) -> Option<&'static Theme> {
    themes().iter().find(|t| t.id == id)
}

/// The file's text exactly: what its CID hashes and what `/themes.txt` serves.
pub fn themes_text() -> &'static str {
    THEMES_TEXT
}

/// The file's version line.
pub fn themes_version() -> u32 {
    THEMES_TEXT.lines().find_map(|l| l.trim().strip_prefix("version ")).and_then(|v| v.trim().parse().ok()).unwrap_or(0)
}

/// The data lines that do not read (a test keeps this empty).
pub fn bad_lines() -> Vec<String> {
    data_lines(THEMES_TEXT).filter(|l| parse(l).is_none()).map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_line_reads_and_every_field_is_in_its_vocabulary() {
        assert!(bad_lines().is_empty(), "{:?}", bad_lines());
        assert_eq!(themes_version(), 1);
        let ts = themes();
        assert!(ts.len() >= 12, "{}", ts.len());
        for (i, t) in ts.iter().enumerate() {
            assert!(ts[..i].iter().all(|o| o.id != t.id), "{} twice", t.id);
            assert!(t.id.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_'), "{}", t.id);
            assert!(ROLES.iter().any(|r| r.0 == t.role), "{}: role {}", t.id, t.role);
            assert!(MODES.iter().any(|m| m.0 == t.mode), "{}: mode {}", t.id, t.mode);
            assert!(t.year < 1929 && t.year > 1600, "{}: {} is not before 1929", t.id, t.year);
            assert!(matches!(t.steps, 12 | 16 | 20), "{}: {} steps", t.id, t.steps);
            assert!(!t.bars.is_empty() && !t.work.is_empty() && !t.composer.is_empty() && !t.note.is_empty(), "{}", t.id);
            // every bar is one bar of the grid: `steps` terms, a chord one term
            for b in &t.bars {
                let mut depth = 0i32;
                let mut terms = 0;
                for w in b.split_ascii_whitespace() {
                    if depth == 0 {
                        terms += 1;
                    }
                    depth += w.matches('[').count() as i32 - w.matches(']').count() as i32;
                }
                assert_eq!(terms, t.steps as usize, "{}: a bar of {terms} terms, not {}: {b}", t.id, t.steps);
            }
            for p in &t.pairs {
                assert!(p != &t.id && ts.iter().any(|o| &o.id == p), "{}: pairs with {p}, which is not a theme", t.id);
            }
        }
    }

    #[test]
    fn a_theme_is_let_ready() {
        let t = theme("ode_to_joy").unwrap();
        let l = t.lets();
        assert!(l.starts_with("let ode_to_joy_1 = f#4 ~ ~ ~ f#4"), "{l}");
        assert!(l.ends_with("let ode_to_joy = <ode_to_joy_1 ode_to_joy_2 ode_to_joy_3 ode_to_joy_4>"), "{l}");
        assert_eq!(t.part(), "melody ode_to_joy");
        assert_eq!(theme("mars_ostinato").unwrap().part(), "bass/20 mars_ostinato");
        assert_eq!(theme("gymnopedie_melody").unwrap().cycle_mark(), "/12");
    }
}
