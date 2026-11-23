//! Found phrases: borrowing text for its shape (docs/beatscript/found-phrases.md). One technique, many views: this module is what
//! the MCP tools (`list_examples` with the phrasebook, `genre_template` with a phrase) and the ontology (a groove each
//! meter family sits on) read of it, so that none of them keeps a second copy of the advice. The corpus itself is
//! `beatscript_serve::phrasebook` over `djengine`'s `data/phrasebook-pd.txt`.

use djengine::vocal::phoneme::{pd_phrase, pd_phrases, phrase_lyric, PdPhrase, PD_FAMILIES};
use djlive::template::Profile;
use serde_json::{json, Value};

/// The groove a family of line sits on, by the meter: `(family id, the groove it wants, the genres of the ontology that
/// are that groove)`. The doc's table (docs/beatscript/found-phrases.md) says the same in prose, and a test holds the two together.
pub const FAMILY_GROOVES: [(&str, &str, &[&str]); 10] = [
    ("trochaic", "chants on the kick: techno, EDM, four on the floor", &["techno", "edm", "trance"]),
    ("iambic", "lilts on the offbeat: house", &["house"]),
    ("anapestic", "wants a triplet feel: drum and bass, breakbeat", &["dnb", "garage"]),
    ("dactylic", "wants a triplet feel: drum and bass, breakbeat", &["dnb", "garage"]),
    ("common-meter", "its 8-6-8-6 fills a 16-bar section, a line to a four-bar phrase", &[]),
    ("sprung", "counts stresses, not syllables: one stress to a beat, over any groove", &[]),
    ("nursery", "a clapping or lullaby dimeter: a triplet shuffle, or sparse", &["garage", "lofi"]),
    ("parallelism", "repeat plus variation: the second half answers the first", &[]),
    ("oratory", "speech as rhythm: a sparse spot section, not a groove", &["ambient"]),
    ("call-and-response", "call and response: the leader's syllables, then the crew's", &[]),
];

/// The groove a family wants (the sentence), None for a family the book does not have.
pub fn groove(family: &str) -> Option<&'static str> {
    FAMILY_GROOVES.iter().find(|(f, _, _)| *f == family).map(|(_, g, _)| *g)
}

/// The family a worked-out meter (`template::classify`) belongs to.
fn family_of_meter(meter: &str) -> Option<&'static str> {
    match meter {
        "iambic" => Some("iambic"),
        "trochaic" => Some("trochaic"),
        "anapestic" | "amphibrachic" => Some("anapestic"),
        "dactylic" => Some("dactylic"),
        _ => None,
    }
}

fn syllables_arg(s: &str) -> Result<(usize, usize), String> {
    let bad = || format!("syllables is a number (8) or a range (8-12), not {s:?}");
    let s = s.trim();
    match s.split_once('-') {
        Some((a, b)) => {
            let (a, b) = (a.trim().parse().map_err(|_| bad())?, b.trim().parse().map_err(|_| bad())?);
            if a > b {
                return Err(bad());
            }
            Ok((a, b))
        }
        None => s.parse().map(|n| (n, n)).map_err(|_| bad()),
    }
}

/// One line of the phrasebook as a tool answers it: the words, where they are from (work, author, year, edition), the meter
/// and its family, the stress as a step pattern, the syllables, the lyric with its phoneme escapes, and the `sing` that sings it.
pub fn line_json(p: &PdPhrase) -> Value {
    json!({
        "id": p.id, "text": p.text, "lyric": p.lyric,
        "work": p.work, "author": p.author, "year": p.year, "edition": p.edition, "source": p.source,
        "family": p.family, "meter": p.meter, "feel": p.feel, "stress": p.stress, "syllables": p.syllables, "call": p.call,
        "groove": groove(&p.family), "note": p.note, "sing": format!("sing @{} on melody", p.id),
    })
}

/// The phrasebook for `list_examples`: its lines, or those of a `meter` (a family id such as `trochaic`, or a word of the
/// meter's name: `pentameter`) with a number of `syllables` (`8` or `8-12`). An empty filter is every line. Err: a family or a
/// count that is not one, with what is.
pub fn lines(meter: Option<&str>, syllables: Option<&str>) -> Result<Value, String> {
    let meter = meter.map(|m| m.trim().to_ascii_lowercase()).filter(|m| !m.is_empty());
    let range = match syllables.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => Some(syllables_arg(s)?),
        None => None,
    };
    if let Some(m) = &meter {
        if !PD_FAMILIES.iter().any(|f| f.id == m) && !pd_phrases().iter().any(|p| p.meter.to_ascii_lowercase().contains(m.as_str())) {
            let ids: Vec<&str> = PD_FAMILIES.iter().map(|f| f.id).collect();
            return Err(format!("no phrasebook line has the meter {m:?}; the families are {}", ids.join(", ")));
        }
    }
    let picked: Vec<Value> = pd_phrases()
        .iter()
        .filter(|p| meter.as_ref().map_or(true, |m| p.family == *m || p.meter.to_ascii_lowercase().contains(m.as_str())))
        .filter(|p| range.map_or(true, |(a, b)| (a..=b).contains(&p.syllables)))
        .map(line_json)
        .collect();
    Ok(json!({
        "cid": crate::phrasebook::cid(),
        "version": djengine::vocal::phoneme::pd_phrasebook_version(),
        "license": "CC0-1.0",
        "about": "Lines first published before 1929, borrowed for their shape (docs/beatscript/found-phrases.md): sing one with `sing @id on melody`, or compile it with `arrange from template(deep-house, @id)`.",
        "families": PD_FAMILIES.iter().map(|f| json!({"id": f.id, "name": f.name, "groove": groove(f.id)})).collect::<Vec<_>>(),
        "count": picked.len(),
        "lines": picked,
    }))
}

/// A phrase as `genre_template` takes it.
pub struct Chosen {
    /// What a `sing` line says: `@id`, or the words in quotes.
    pub sing: String,
    /// The phrase as the answer reports it.
    pub json: Value,
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n"))
}

/// Resolve `phrase`: a phrasebook id (`tempest_such_stuff` or `@tempest_such_stuff`) or any words. `genre` (a slug) says how well
/// the line fits it, by the meter. Err: nothing to sing.
pub fn choose(phrase: &str, genre: Option<&str>) -> Result<Chosen, String> {
    let phrase = phrase.trim();
    let id = phrase.strip_prefix('@').unwrap_or(phrase);
    if let Some(p) = pd_phrase(id) {
        let mut j = line_json(p);
        j["fit"] = fit(&p.family, &p.feel, genre);
        return Ok(Chosen { sing: format!("@{}", p.id), json: j });
    }
    if phrase.starts_with('@') {
        return match phrase_lyric(id) {
            Some(lyric) => Ok(Chosen { sing: format!("@{id}"), json: json!({"id": id, "lyric": lyric, "sing": format!("sing @{id} on melody"), "note": "a stock phrase, not a phrasebook line: its stress is worked out, not curated"}) }),
            None => {
                let near: Vec<String> = pd_phrases().iter().map(|p| p.id.clone()).filter(|i| i.contains(id) || id.contains(i.as_str())).take(5).collect();
                Err(format!("no phrase @{id}: {}; list_examples with phrasebook true lists them, or give the words", if near.is_empty() { "the ids are in the phrasebook".to_string() } else { format!("did you mean {}?", near.join(", ")) }))
            }
        };
    }
    let p = Profile::from_text(phrase, None)?;
    let family = family_of_meter(p.meter);
    let feel = p.feel.name();
    let mut j = json!({
        "text": p.lyric, "meter": p.meter, "family": family, "feel": feel, "stress": p.stress.iter().map(|s| if *s { "x" } else { "~" }).collect::<Vec<_>>().join(" "),
        "syllables": p.syllables(), "stress_worked_out": true, "sing": format!("sing {} on melody", quote(&p.lyric)),
        "note": "any words: the stress is a plain rule (small words weak, the first syllable of the rest strong). A phrasebook line has its stress curated.",
    });
    if let Some(f) = family {
        j["groove"] = json!(groove(f));
        j["fit"] = fit(f, feel, genre);
    }
    Ok(Chosen { sing: quote(&p.lyric), json: j })
}

/// How a family fits a genre: the genres it is at home in, and whether this is one.
fn fit(family: &str, feel: &str, genre: Option<&str>) -> Value {
    let homes: &[&str] = FAMILY_GROOVES.iter().find(|(f, _, _)| *f == family).map_or(&[], |(_, _, g)| *g);
    let groove = groove(family).unwrap_or("");
    let at_home = genre.map_or(false, |g| homes.contains(&g));
    let note = match (genre, homes.is_empty()) {
        (Some(g), false) if at_home => format!("a {family} line {groove}; {g} is that groove"),
        (Some(g), false) => format!("a {family} line {groove}: against {g}'s grain, which is fine if you mean it (in {})", homes.join(", ")),
        (_, true) => format!("a {family} line {groove}"),
        (None, false) => format!("a {family} line {groove} (at home in {})", homes.join(", ")),
    };
    json!({"at_home": at_home, "feel": feel, "genres": homes, "note": note})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_family_has_its_groove_once() {
        for f in PD_FAMILIES.iter() {
            assert_eq!(FAMILY_GROOVES.iter().filter(|(id, _, _)| *id == f.id).count(), 1, "{}", f.id);
        }
        assert_eq!(FAMILY_GROOVES.len(), PD_FAMILIES.len());
    }

    #[test]
    fn the_phrasebook_filters_by_meter_and_syllables() {
        let all = lines(None, None).unwrap();
        assert_eq!(all["count"].as_u64().unwrap() as usize, pd_phrases().len());
        let t = lines(Some("trochaic"), None).unwrap();
        assert!(t["lines"].as_array().unwrap().iter().all(|l| l["family"] == "trochaic") && t["count"].as_u64().unwrap() >= 5);
        let short = lines(None, Some("8-10")).unwrap();
        assert!(short["lines"].as_array().unwrap().iter().all(|l| (8..=10).contains(&l["syllables"].as_u64().unwrap())));
        let both = lines(Some("pentameter"), Some("10")).unwrap();
        assert!(both["count"].as_u64().unwrap() >= 3 && both["lines"].as_array().unwrap().iter().all(|l| l["meter"].as_str().unwrap().contains("pentameter") && l["syllables"] == 10));
        assert!(lines(Some("limerick-ish"), None).unwrap_err().contains("the families are"));
        assert!(lines(None, Some("lots")).is_err());
        // each line says its source, year, stress and escapes
        let l = &all["lines"][0];
        for k in ["id", "text", "lyric", "work", "author", "year", "edition", "stress", "syllables", "sing"] {
            assert!(!l[k].is_null(), "{k}");
        }
    }

    #[test]
    fn a_phrase_is_an_id_or_words() {
        let c = choose("@tyger_burning_bright", Some("techno")).unwrap();
        assert_eq!(c.sing, "@tyger_burning_bright");
        assert_eq!(c.json["fit"]["at_home"], true);
        let c = choose("tempest_such_stuff", Some("techno")).unwrap();
        assert_eq!(c.json["fit"]["at_home"], false);
        assert!(c.json["fit"]["note"].as_str().unwrap().contains("against techno's grain"));
        let c = choose("robots on the road", None).unwrap();
        assert_eq!(c.sing, "\"robots on the road\"");
        assert_eq!(c.json["stress_worked_out"], true);
        assert!(choose("@no_such_phrase", None).is_err());
        assert!(choose("   ", None).is_err());
    }
}
