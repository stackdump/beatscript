//! The genre templates, frozen (`data/genre-templates.txt`): the starting program of each genre of the bitwrap ontology, as
//! the MCP `genre_template` tool writes it (beatscript-serve `genres::template`: the genre's tempo, key, kit and voices, its family's
//! standard song structure and its first chord progression), kept as text so that a program can borrow a section of one by
//! name and have it mean the same music for ever: `use genre@1 techno.drop as drop` (djlive `uses.rs`). One CC0 file, a
//! version a file: the templates a later ontology release writes differently are a version 2 beside this one, never an edit
//! of it (beatscript-serve's `genre_templates_v1_are_frozen` holds the file to what the tool writes). Pure: the file is compiled in.

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use once_cell::race::OnceBox;

static V1: &str = include_str!("../data/genre-templates.txt");

/// The versions this build has, oldest first: `genre@1` is the first, and a bare `genre` is it.
pub const VERSIONS: [u32; 1] = [1];

/// The line that opens each genre's program in the file: `=== techno`.
pub const HEAD: &str = "=== ";

/// One genre's starting program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenreTemplate {
    /// The genre's slug (`techno`, `dnb`).
    pub genre: String,
    /// The whole program, its `#!` header lines and comments as the tool wrote them.
    pub text: String,
}

fn parse(text: &str) -> Vec<GenreTemplate> {
    let mut out: Vec<GenreTemplate> = Vec::new();
    for l in text.lines() {
        if let Some(g) = l.strip_prefix(HEAD) {
            out.push(GenreTemplate { genre: g.trim().to_string(), text: String::new() });
        } else if let Some(cur) = out.last_mut() {
            cur.text.push_str(l);
            cur.text.push('\n');
        }
    }
    for t in &mut out {
        while t.text.ends_with("\n\n") {
            t.text.pop();
        }
    }
    out
}

static TABLE: OnceBox<Vec<GenreTemplate>> = OnceBox::new();

/// The file of version `v`, as it is (what its CID hashes); None for a version this build does not have.
pub fn text(v: u32) -> Option<&'static str> {
    match v {
        1 => Some(V1),
        _ => None,
    }
}

/// Every genre template of version `v`, in the file's order (by slug); empty for a version this build does not have.
pub fn templates(v: u32) -> &'static [GenreTemplate] {
    match v {
        1 => TABLE.get_or_init(|| Box::new(parse(V1))),
        _ => &[],
    }
}

/// The template of `genre` in version `v`.
pub fn find(v: u32, genre: &str) -> Option<&'static GenreTemplate> {
    templates(v).iter().find(|t| t.genre == genre)
}

/// The genres of version `v`, by slug.
pub fn genres(v: u32) -> Vec<&'static str> {
    templates(v).iter().map(|t| t.genre.as_str()).collect()
}

/// The file's version line.
pub fn version_line(v: u32) -> Option<u32> {
    text(v)?.lines().find_map(|l| l.trim().strip_prefix("version ")).and_then(|x| x.trim().parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_reads_one_program_a_genre() {
        assert_eq!(version_line(1), Some(1));
        let ts = templates(1);
        for (i, t) in ts.iter().enumerate() {
            assert!(ts[..i].iter().all(|o| o.genre != t.genre), "{} twice", t.genre);
            assert!(t.genre.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-'), "{}", t.genre);
            assert!(t.text.contains("\narrange "), "{}: no arrange", t.genre);
        }
        assert!(ts.len() >= 19 && find(1, "techno").is_some(), "{} genre templates", ts.len());
        assert!(text(2).is_none() && templates(2).is_empty());
    }
}
