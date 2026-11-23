//! BeatScript 1 to BeatScript 2 (djlive::version): the code action "Upgrade to BeatScript 2" on a text in the old spelling,
//! and document formatting, which upgrades as every fmt a person asks for does.
//!
//! The upgrade is the version line alone: `#! btw 1` becomes `#! beatscript 2`, or `#! beatscript 2` is put first when the
//! text has no version line. The program plays the same; its CID is new (the CID of the same program in BeatScript 2).

use djengine::json::JsonValue;
use djlive::diag::Span;
use djlive::version::{self, Version, VersionLine};

use crate::analysis::Analysis;
use crate::json::{arr, obj, s, text_edit};
use crate::text::Encoding;

pub const TITLE: &str = "Upgrade to BeatScript 2";
/// The kind it is offered as: a rewrite (the lightbulb shows it; a `source` action would be in the Source Action menu only).
pub const KIND: &str = "refactor.rewrite";

/// The edit that upgrades `text`: its version line rewritten, or the version line inserted at the start. None for a text
/// already in BeatScript 2 or whose version line does not read (its diagnostic says what to write).
pub fn edit_of(text: &str) -> Option<(Span, String)> {
    match version::find(text) {
        Ok(None) => Some((Span::new(0, 0), format!("{}\n", version::LINE))),
        Ok(Some(VersionLine { version: Version::V1, span })) => Some((span, version::LINE.to_string())),
        Ok(Some(VersionLine { version: Version::V2, .. })) | Err(_) => None,
    }
}

/// The action, on a BeatScript 1 text, when `range` starts in the header block or on the first line (where the version line
/// is or goes), and the client wants a rewrite.
pub fn action(a: &Analysis, uri: &str, range: Span, only: &[String], enc: Encoding) -> Option<JsonValue> {
    let wants = only.is_empty() || only.iter().any(|o| KIND == o || KIND.starts_with(&format!("{o}.")));
    if !wants {
        return None;
    }
    let first_line = a.text.find('\n').map_or(a.text.len(), |n| n + 1);
    if range.start > djlive::linked::header_end(&a.text).max(first_line) {
        return None;
    }
    let (span, new) = edit_of(&a.text)?;
    Some(obj([("title", s(TITLE)), ("kind", s(KIND)), ("isPreferred", JsonValue::Bool(false)), ("edit", crate::fix::workspace_edit(a, uri, &[(span, new)], enc))]))
}

/// `textDocument/formatting`: the document formatted and in BeatScript 2 (djlive::format::format_upgrading), as one edit of
/// the whole text; no edit when it is formatted already, or when it does not read (fix the errors first).
pub fn formatting(a: &Analysis, enc: Encoding) -> JsonValue {
    match djlive::format::format_upgrading(&a.text, &beatscript_api::web_env().env()) {
        Ok(t) if t != a.text => arr(vec![text_edit(&a.index, &a.text, Span::new(0, a.text.len()), &t, enc)]),
        _ => arr(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(text: &str) -> Option<String> {
        let (sp, new) = edit_of(text)?;
        let mut t = text.to_string();
        t.replace_range(sp.start..sp.end, &new);
        Some(t)
    }

    #[test]
    fn the_edit_is_upgrade_source() {
        for text in ["tempo 120\n", "#! btw 1\ntempo 120\n", "#! name: x\n#! btw 1\n# a\ntempo 120\n", "#! name: x\ntempo 1\n", ""] {
            assert_eq!(apply(text).as_deref(), Some(version::upgrade_source(text).as_str()), "{text:?}");
        }
        for text in ["#! beatscript 2\ntempo 120\n", "#! beatscript 3\ntempo 1\n", "#! btw 1\n#! beatscript 2\n"] {
            assert_eq!(apply(text), None, "{text:?}");
        }
    }
}
