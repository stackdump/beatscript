//! The language version a text is written in, said by its version line in the `#!` header block.
//! (rename-to-beatscript: keep. This file spells the old name on purpose.)
//!
//! - `#! beatscript 2` is BeatScript 2, this spelling of the language. The version line is part of the program: its canonical
//!   text keeps the line first (`#! beatscript 2\n` and then the statements), and its CID is of the document
//!   `{"format":2,"text":<canonical text>,"type":"beatscript"}`.
//! - `#! btw 1`, or no version line at all, is BeatScript 1: the language as it was spelled when it was called btw. It is read
//!   and canonicalized exactly as it always was (the canonical text has no `#!` line) and keeps its CID, of the document
//!   `{"format":1,"text":<canonical text>,"type":"djrust.live"}`. This is the legacy path: every program, link and CID made
//!   before the rename means what it meant.
//!
//! The two versions are the same language: a BeatScript 1 text and the BeatScript 2 text it upgrades to play the same, line
//! for line. Only the identity differs, so the rename is a new CID for each program, never a new meaning. `fmt` upgrades
//! ([`upgrade_source`]); the store keeps both CIDs and says the new one `isBasedOn` the old with derivation `renamed`.
//!
//! A text is told apart by its text alone, so every place that holds a canonical text can hash it right with no other
//! argument: a canonical text that starts with [`CANONICAL_HEAD`] is BeatScript 2, and any other is BeatScript 1.

use crate::diag::{Diagnostic, Span};

/// The language versions a text can be in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Version {
    /// BeatScript 1, the old spelling (`#! btw 1`, or no version line): the legacy canonical form and CID.
    #[default]
    V1,
    /// BeatScript 2 (`#! beatscript 2`): the version line is kept in the canonical text, and the CID document says format 2.
    V2,
}

/// The version new text is written in: what `fmt` upgrades to and what the generators write.
pub const CURRENT: Version = Version::V2;

/// The version line of BeatScript 2, as it is written and as the canonical text begins.
pub const LINE: &str = "#! beatscript 2";
/// The version line of BeatScript 1, the old spelling.
pub const LEGACY_LINE: &str = "#! btw 1";
/// What a BeatScript 2 canonical text starts with.
pub const CANONICAL_HEAD: &str = "#! beatscript 2\n";

impl Version {
    /// The number the version line says (`1`, `2`).
    pub fn number(self) -> u32 {
        match self {
            Version::V1 => 1,
            Version::V2 => 2,
        }
    }

    /// The version line, as written.
    pub fn line(self) -> &'static str {
        match self {
            Version::V1 => LEGACY_LINE,
            Version::V2 => LINE,
        }
    }

    /// What the canonical text starts with: nothing in BeatScript 1, the version line in BeatScript 2.
    pub fn canonical_head(self) -> &'static str {
        match self {
            Version::V1 => "",
            Version::V2 => CANONICAL_HEAD,
        }
    }

    /// The `format` of the CID document.
    pub fn format(self) -> i64 {
        i64::from(self.number())
    }

    /// The `type` of the CID document.
    pub fn doc_type(self) -> &'static str {
        match self {
            Version::V1 => "djrust.live",
            Version::V2 => "beatscript",
        }
    }
}

/// A version line found in a text's header block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionLine {
    pub version: Version,
    /// The whole line, without its newline.
    pub span: Span,
}

/// The version a canonical text is in: BeatScript 2 when it starts with its version line, BeatScript 1 otherwise.
pub fn of_canonical(canonical: &str) -> Version {
    if canonical.starts_with(CANONICAL_HEAD) {
        Version::V2
    } else {
        Version::V1
    }
}

/// The version a text says it is in (BeatScript 1 when it says nothing, or says it wrong: [`check`] is what complains).
pub fn of_source(src: &str) -> Version {
    match find(src) {
        Ok(Some(l)) => l.version,
        _ => Version::V1,
    }
}

/// The `#!` lines a version line may be: every one before the program's first statement, wherever it stands among the
/// leading blank and comment lines, indented or not (a version line is as whitespace-free as the rest of the language: the
/// text respaced is the same program). Each with the span of the line as written, its indentation left out.
fn header_block(src: &str) -> impl Iterator<Item = (Span, &str)> {
    let mut at = 0;
    let mut done = false;
    src.split_inclusive('\n').filter_map(move |raw| {
        let start = at;
        at += raw.len();
        if done {
            return None;
        }
        let t = raw.trim();
        if t.is_empty() || (t.starts_with('#') && !t.starts_with("#!")) {
            return None;
        }
        if !t.starts_with("#!") {
            done = true;
            return None;
        }
        let lead = raw.len() - raw.trim_start().len();
        Some((Span::new(start + lead, start + lead + t.len()), t))
    })
}

/// A header line read as a version line: `Some((name, number))` for a line with no `key:` whose first word is a language
/// name (`btw`, `beatscript`); any other line is not one.
fn as_version_line(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("#!")?.trim();
    if rest.contains(':') {
        return None;
    }
    let mut words = rest.split_whitespace();
    let name = words.next()?;
    if !matches!(name, "btw" | "beatscript") {
        return None;
    }
    Some((name, rest[name.len()..].trim()))
}

/// The version line of a text's header block, if it has one, or the diagnostic that says why the one it has does not read:
/// a version that does not exist (`#! beatscript 3`, `#! btw 2`), a version line with no number, or two version lines.
pub fn find(src: &str) -> Result<Option<VersionLine>, Diagnostic> {
    let mut found: Option<VersionLine> = None;
    for (span, line) in header_block(src) {
        let Some((name, number)) = as_version_line(line) else { continue };
        let version = match (name, number) {
            ("beatscript", "2") => Version::V2,
            ("btw", "1") => Version::V1,
            ("beatscript", "") | ("btw", "") => {
                return Err(Diagnostic::error(span, "a version line says its version").help(format!("write `{LINE}`")));
            }
            ("beatscript", "1") => {
                return Err(Diagnostic::error(span, "there is no BeatScript 1 version line in this spelling").help(format!(
                    "BeatScript 1 is the old spelling, `{LEGACY_LINE}`; this spelling is `{LINE}`"
                )));
            }
            _ => {
                return Err(Diagnostic::error(span, format!("`{name} {number}` is not a version of the language"))
                    .help(format!("the versions are `{LINE}` and, in the old spelling, `{LEGACY_LINE}`")));
            }
        };
        if found.is_some() {
            return Err(Diagnostic::error(span, "a text has one version line").help("delete this one"));
        }
        found = Some(VersionLine { version, span });
    }
    Ok(found)
}

/// The diagnostics a text's version line gives: an error for one that does not read ([`find`]), none otherwise.
pub fn check(src: &str) -> Vec<Diagnostic> {
    match find(src) {
        Err(d) => vec![d],
        Ok(_) => Vec::new(),
    }
}

/// `src` in BeatScript 2 spelling: its `#! btw 1` line made `#! beatscript 2`, or `#! beatscript 2` put first when it has no
/// version line. Everything else is left byte for byte (the other header lines, the comments, the statements), so the program
/// is the one it was. A text already in BeatScript 2, or with a version line that does not read, comes back as it is.
pub fn upgrade_source(src: &str) -> String {
    match find(src) {
        Ok(Some(VersionLine { version: Version::V2, .. })) | Err(_) => src.to_string(),
        Ok(Some(VersionLine { version: Version::V1, span })) => format!("{}{LINE}{}", &src[..span.start], &src[span.end..]),
        Ok(None) => format!("{LINE}\n{src}"),
    }
}

/// A BeatScript 1 canonical text as the BeatScript 2 canonical text of the same program (the version line put first); a
/// BeatScript 2 canonical text as it is.
pub fn upgrade_canonical(canonical: &str) -> String {
    match of_canonical(canonical) {
        Version::V2 => canonical.to_string(),
        Version::V1 => format!("{CANONICAL_HEAD}{canonical}"),
    }
}

/// A BeatScript 2 canonical text as the BeatScript 1 canonical text of the same program (the version line taken off): the text
/// whose CID is the program's legacy CID. A BeatScript 1 canonical text as it is.
pub fn legacy_canonical(canonical: &str) -> &str {
    canonical.strip_prefix(CANONICAL_HEAD).unwrap_or(canonical)
}

/// `src` with its version line taken out of its header block (the line and its newline), wherever in the block it is; `src`
/// itself when it has none or one that does not read.
pub fn without_version_line(src: &str) -> String {
    match find(src) {
        Ok(Some(l)) => {
            // the whole line: its indentation, the line, the rest of it and its newline
            let start = src[..l.span.start].rfind('\n').map_or(0, |i| i + 1);
            let end = src[l.span.end..].find('\n').map_or(src.len(), |i| l.span.end + i + 1);
            format!("{}{}", &src[..start], &src[end..])
        }
        _ => src.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_text_says_its_version_or_is_beatscript_1() {
        assert_eq!(of_source("tempo 120\n"), Version::V1);
        assert_eq!(of_source("#! btw 1\ntempo 120\n"), Version::V1);
        assert_eq!(of_source("#! beatscript 2\ntempo 120\n"), Version::V2);
        assert_eq!(of_source("#! name: a\n#! beatscript 2\ntempo 120\n"), Version::V2);
        // only before the first statement; blank lines, comments and indentation before it do not matter
        assert_eq!(of_source("tempo 120\n#! beatscript 2\n"), Version::V1);
        assert_eq!(of_source("\n  # a song\n\t#! beatscript 2  \ntempo 120\n"), Version::V2);
        assert_eq!(upgrade_source("\n# a song\n   #! btw 1\ntempo 1\n"), "\n# a song\n   #! beatscript 2\ntempo 1\n");
        // a `key:` line is not a version line
        assert_eq!(of_source("#! beatscript: 2\ntempo 120\n"), Version::V1);
    }

    #[test]
    fn a_wrong_version_line_is_an_error() {
        for bad in ["#! beatscript 3\n", "#! btw 2\n", "#! beatscript\n", "#! beatscript 1\n", "#! btw 1\n#! beatscript 2\n"] {
            assert!(find(bad).is_err(), "{bad}");
            assert_eq!(check(bad).len(), 1, "{bad}");
        }
        assert!(check("#! btw 1\n").is_empty());
        assert!(check("#! hello there\ntempo 1\n").is_empty());
    }

    #[test]
    fn upgrading_rewrites_only_the_version_line() {
        assert_eq!(upgrade_source("tempo 120\n"), "#! beatscript 2\ntempo 120\n");
        assert_eq!(upgrade_source("#! btw 1\n#! name: x\n# a\ntempo 120\n"), "#! beatscript 2\n#! name: x\n# a\ntempo 120\n");
        assert_eq!(upgrade_source("#! name: x\n#! btw 1\ntempo 120\n"), "#! name: x\n#! beatscript 2\ntempo 120\n");
        let v2 = "#! beatscript 2\ntempo 120\n";
        assert_eq!(upgrade_source(v2), v2);
        assert_eq!(upgrade_source(&upgrade_source("tempo 1\n")), upgrade_source("tempo 1\n"));
    }

    #[test]
    fn canonical_texts_say_their_version_by_their_first_line() {
        assert_eq!(of_canonical("tempo 120\n"), Version::V1);
        assert_eq!(of_canonical("#! beatscript 2\ntempo 120\n"), Version::V2);
        assert_eq!(upgrade_canonical("tempo 120\n"), "#! beatscript 2\ntempo 120\n");
        assert_eq!(legacy_canonical("#! beatscript 2\ntempo 120\n"), "tempo 120\n");
        assert_eq!(legacy_canonical("tempo 120\n"), "tempo 120\n");
        assert_eq!(upgrade_canonical(&upgrade_canonical("tempo 1\n")), upgrade_canonical("tempo 1\n"));
        assert_eq!(without_version_line("#! name: a\n#! beatscript 2\ntempo 1\n"), "#! name: a\ntempo 1\n");
        assert_eq!(without_version_line("#! btw 1\ntempo 1\n"), "tempo 1\n");
        assert_eq!(without_version_line("tempo 1\n"), "tempo 1\n");
    }
}
