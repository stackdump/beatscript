//! Two read-only MCP tools, no account: `lineage` (a program's family tree: the same JSON-LD document
//! `/p/<cid>/tree.jsonld` serves, from beatscript_serve::lineage) and `genre_template` (a starting program for a genre, from the
//! ontology's conventions in beatscript_serve::genres).

use serde_json::Value;

use crate::compose::Fail;

/// The tool's description (`p` is the site's origin).
pub fn description(p: &str) -> String {
    format!(
        "A stored program's family tree, from the rights each program keeps (forked_from): its ancestors (nearest first), its siblings (the parent's other forks) and every generation of its forks, each edge labelled with what changed in music from parent to fork (the same diff summary lines check's same_as gives: \"tempo 124 -> 126\", \"drop2 bass = drop bass transposed +1\"; sameSong true when the fork only rewrote the text). Returns the JSON-LD document {p}/p/<CID>/tree.jsonld serves: program, forkCount, descendantCount, ancestors, siblings and forks as CIDs, programs (name, licence, tempo, key, credits, url, forkCount of each), derivations (from, to, sameSong, change, and a compare URL, {p}/p/<A>/compare/<B>). Only public programs appear: a private one is never listed, and a chain that passes through one stops there (ancestorsCut). A person can browse the same tree at {p}/p/<CID>/tree."
    )
}

/// The tree of `cid_or_link`, or a 404 that does not say whether a private program has that CID.
pub fn call(l: &beatscript_serve::lineage::Lineage, public: &str, cid_or_link: &str) -> Result<Value, Fail> {
    let arg = cid_or_link.trim();
    if arg.is_empty() {
        return Err(Fail::new(400, "bad_request", "argument \"cid\" is empty: give a program's CID"));
    }
    let Some(cid) = crate::mcp::tools::bare_cid(arg) else {
        return Err(Fail::new(400, "bad_request", "argument \"cid\" is not a CID (it starts with z) or a /p/<CID> link"));
    };
    match l.tree(&cid) {
        Some(t) => Ok(t.json_ld(public)),
        None => Err(Fail::new(404, "not_found", format!("no public program with CID {cid} is stored here or among the examples"))),
    }
}

/// The MCP `genre_template` tool's description.
pub fn genre_template_description() -> String {
    "A starting BeatScript program for a genre, built from the BeatScript ontology's data for it (https://beatscript.app/ontology/, CC0, forked from the bitwrap ontology): its tempo, key (root and scale), swing, drum kit and Euclidean kick, snare and hihat rhythms, its bass, arp and melody voices, its first chord progression (the chords, bass, arp and melody are Roman numerals, so a `key` line moves them all), and its family's standard song structure, each section with the roles the family's section profile makes active and its usual length. Returns {text, cid, canonical, seconds, genre{genre, label, family, tempo, tempo_range, scale, kit, voices, progressions, sections}} (and phrase, warnings with a phrase). The text passes check (it is checked before it is returned) and carries a `#! genre:` line, so check reports its style against the genre. Edit it, check it, then play or share it. For vocals, pass phrase (a phrasebook id from list_examples with phrasebook true, or any words): it is sung on the melody part of each section that has one, and the answer gains phrase with its source, year, meter, stress, syllables and phoneme escapes and whether its meter fits the genre (https://beatscript.app/docs/found-phrases.md). For a classical form, pass form (quodlibet, passacaglia, canon, rondo, fugue or theme-and-variations): the program is that form's template at the genre's tempo, built from public-domain themes, with the phrase (or the form's own) sung in it, and the answer gains form (its sections, mutation plan, themes and reference track) and expanded (the program written out to edit). To borrow one theme instead, pass theme (an id from list_examples with themes true): its lets go in, moved to the genre's key, played by the part of its role. See https://beatscript.app/docs/forms.md. For a whole track made from a phrase, write `arrange from template(deep-house, \"your words\")` (see expand). Read-only, no sign-in.".to_string()
}
