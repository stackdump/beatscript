//! Programs arrive formatted. The one hook `eval`, `load`, `share`, `queue` and `push_private` (and the phone push route) run
//! a program's text through before it is sent to a tab or a phone, held in a queue or stored: the formatter behind the
//! editor's fmt button and the `fmt` tool (djlive::format), so nobody presses fmt after an agent sends a track.
//!
//! It keeps what a person wrote that the canonical form drops (the `#!` header lines, the comments, the blank lines between
//! blocks), so formatted text has the CID of the text it came from, and it adds no state: text that does not read (the checker
//! says why, in the language's terms, as it always did), or that would come out past the size limit, goes on as it is.
//! A program that is only fetched (`get`, `open`) is not formatted, and a stored program is already canonical, which
//! formatting leaves as it is.

/// `text` formatted, or `text` itself when it does not read or the formatted text is over `max_bytes`. Parses and prints
/// only: no CID is made and nothing is compiled, which is also why a private program may go through it.
pub fn autoformat(text: &str, max_bytes: usize) -> String {
    match beatscript_api::format_text(text) {
        Some(f) if f.len() <= max_bytes => f,
        _ => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UNFORMATTED: &str = "#! btw 1\n#! name: loose   one\n# a track\ntempo   120   # fast\n\n\ndeck A:   kick  e(4,16) | snare ~ x ~ x\n";
    const FORMATTED: &str = "#! btw 1\n#! name: loose   one\n# a track\ntempo 120  # fast\n\ndeck A: kick e(4, 16) | snare ~ x ~ x\n";

    #[test]
    fn a_program_comes_out_as_fmt_prints_it_with_its_cid() {
        let f = autoformat(UNFORMATTED, 16384);
        assert_eq!(f, FORMATTED);
        let cid = |t: &str| serde_json::from_str::<serde_json::Value>(&beatscript_api::check_json(t)).unwrap()["cid"].clone();
        assert_eq!(cid(&f), cid(UNFORMATTED));
        // the fmt tool (asked for by a person or an agent) answers the same text in BeatScript 2, a new CID; the hook never
        // upgrades, so what it sends has the CID it was given
        let v: serde_json::Value = serde_json::from_str(&beatscript_api::fmt_json(UNFORMATTED)).unwrap();
        assert_eq!(v["text"], f.replacen("#! btw 1", "#! beatscript 2", 1));
        assert_eq!(v["from_cid"], cid(UNFORMATTED));
        assert_eq!(beatscript_api::format_text(UNFORMATTED).as_deref(), Some(f.as_str()));
        assert_eq!(autoformat(&f, 16384), f, "formatting twice is formatting once");
    }

    #[test]
    fn the_eight_line_arrange_example_arrives_with_its_header_and_its_comments() {
        // the first example a newcomer sees: its comments are the teaching, and its text is already in the form fmt gives
        let text = include_str!("../../../examples/beatscript/arrange-in-eight-lines.beatscript");
        let f = autoformat(text, 16384);
        assert_eq!(f, text);
        assert_eq!(f.lines().filter(|l| l.starts_with("#!")).count(), 3, "the version line, the name and the licence");
        assert_eq!(f.lines().filter(|l| l.starts_with("# ")).count(), 3);
        let cid = |t: &str| serde_json::from_str::<serde_json::Value>(&beatscript_api::check_json(t)).unwrap()["cid"].clone();
        assert_eq!(cid(&f), "z4EBG9jHEKgonTqe3ucsjCSx1dMpeceWXr68HEfZrw2n6Yd62sj");
    }

    #[test]
    fn what_does_not_read_or_does_not_fit_goes_on_as_it_is() {
        for bad in ["tempo\n", "deck A: kick x |\n", "tempo 120\nkey\n"] {
            assert_eq!(autoformat(bad, 16384), bad);
        }
        assert_eq!(autoformat(UNFORMATTED, FORMATTED.len() - 1), UNFORMATTED);
        assert_eq!(autoformat(UNFORMATTED, FORMATTED.len()), FORMATTED);
        assert_eq!(autoformat("", 16384), "");
    }
}
