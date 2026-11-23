use std::collections::BTreeMap;

use super::*;

/// N-Triples as a [`Graph`]: the published ontology's triples for five genres (testdata/genres.nt, CC0) and the
/// conventions module's (testdata/conventions.nt).
struct Nt {
    by: BTreeMap<(String, String), Vec<String>>,
}

fn term(t: &str) -> String {
    let t = t.trim();
    if let Some(i) = t.strip_prefix('<').and_then(|x| x.strip_suffix('>')) {
        return i.to_string();
    }
    let body = t.strip_prefix('"').unwrap_or(t);
    body[..body.rfind('"').unwrap_or(body.len())].to_string()
}

impl Nt {
    fn parse(texts: &[&str]) -> Nt {
        let mut by: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
        for text in texts {
            for l in text.lines().filter(|l| !l.trim().is_empty()) {
                let l = l.trim().strip_suffix('.').unwrap().trim();
                let (s, rest) = l.split_once(' ').unwrap();
                let (p, o) = rest.split_once(' ').unwrap();
                by.entry((term(s), term(p))).or_default().push(term(o));
            }
        }
        for v in by.values_mut() {
            v.sort();
        }
        Nt { by }
    }
}

impl Graph for Nt {
    fn all(&self, s: &str, p: &str) -> Vec<String> {
        self.by.get(&(s.to_string(), p.to_string())).cloned().unwrap_or_default()
    }
    fn of_type(&self, class: &str) -> Vec<String> {
        self.by.iter().filter(|((_, p), o)| p == RDF_TYPE && o.iter().any(|x| x == class)).map(|((s, _), _)| s.clone()).collect()
    }
}

fn onto() -> Nt {
    Nt::parse(&[include_str!("testdata/genres.nt"), include_str!("testdata/conventions.nt")])
}

fn compile(text: &str) -> Compiled {
    let env = beatscript_api::web_env().env();
    let c = djlive::compile::check_with(text, &env);
    c.compiled.unwrap_or_else(|| panic!("{text}\n{:?}", c.diagnostics.iter().map(|d| d.render(text)).collect::<Vec<_>>()))
}

fn check_ok(text: &str) -> Value {
    let rep: Value = serde_json::from_str(&beatscript_api::check_json(text)).unwrap();
    assert_eq!(rep["ok"], true, "{text}\n{rep}");
    rep
}

#[test]
fn a_genre_is_read_from_the_graph() {
    let g = onto();
    let t = find(&g, "techno").unwrap();
    assert_eq!(t.iri, "https://beatscript.app/ontology/genre/techno");
    assert_eq!(t.bpm, Some(128.0));
    assert_eq!((t.min_bpm, t.max_bpm), (Some(120.0), Some(150.0)));
    assert_eq!(t.scale.as_deref(), Some("major"));
    assert_eq!(t.kit.as_ref().unwrap().voice, "drums");
    assert_eq!(t.voices["melody"].voice, "trance-lead", "supersaw is played by its beatscriptVoice");
    assert_eq!(t.voices["bass"].name, "acid");
    assert_eq!(t.progressions, [vec![1, 7, 1, 7]]);
    assert_eq!(t.rhythms["kick"], Rhythm { pulses: 4, steps: 16, rotation: 0 });
    for name in ["Techno", "genre:techno", "https://beatscript.app/ontology/genre/techno", " TECHNO "] {
        assert_eq!(find(&g, name).unwrap().slug, "techno", "{name}");
    }
    assert!(find(&g, "polka").is_none());
    // a genre whose generator data has no instruments takes the convention's
    let tr = find(&g, "trance").unwrap();
    assert_eq!(tr.voices["bass"].voice, "sub-bass");
    assert_eq!(tr.kit.as_ref().unwrap().voice, "drums");
}

#[test]
fn the_family_structure_has_profiles_and_lengths() {
    let g = onto();
    let t = find(&g, "techno").unwrap();
    let s = structure(&g, &t);
    let names: Vec<&str> = s.iter().map(|x| x.name.as_str()).collect();
    assert_eq!(names, ["intro", "buildup", "drop", "breakdown", "buildup", "drop", "outro"]);
    assert_eq!(s[0].roles, ["hihat", "kick"]);
    assert!(s[2].roles.contains(&"bass".to_string()) && s[2].roles.contains(&"melody".to_string()));
    assert_eq!(s[2].bars, Some(16));
    assert_eq!(s[0].bars, Some(8));
}

#[test]
fn every_template_passes_check_and_is_in_its_genre() {
    let g = onto();
    let (voices, kits) = instrument_names();
    for slug in ["techno", "house", "jazz", "trance", "dnb"] {
        let genre = find(&g, slug).unwrap();
        let text = template(&g, &genre, &voices, &kits);
        if std::env::var_os("SHOW_TEMPLATES").is_some() {
            eprintln!("{text}");
        }
        let rep = check_ok(&text);
        assert_eq!(rep["tempo"].as_f64(), genre.bpm, "{slug}");
        // and it is in style for its own genre
        let c = compile(&text);
        let w = style(&c, &text, &genre, &family_of(&g, &genre));
        assert!(w.is_empty(), "{slug}: {w:?}\n{text}");
        assert!(text.contains(&format!("#! genre: {slug}")));
    }
    let t = find(&g, "techno").unwrap();
    let text = template(&g, &t, &voices, &kits);
    assert!(text.contains("tempo 128\n"), "{text}");
    assert!(text.contains("kit=drums"), "{text}");
    assert!(text.contains("inst=trance-lead"), "{text}");
    assert!(text.contains("arrange intro buildup drop breakdown buildup drop outro\n"), "{text}");
    assert!(text.contains("section drop 16 bars {"), "{text}");
    // jazz: swung, dorian, the cr78 kit
    let j = template(&g, &find(&g, "jazz").unwrap(), &voices, &kits);
    assert!(j.contains("swing 0.6\n") && j.contains(":dorian\n") && j.contains("kit=drums-cr78"), "{j}");
}

#[test]
fn style_warns_on_tempo_kit_voice_and_chords_never_errors() {
    let g = onto();
    let t = find(&g, "techno").unwrap();
    let fam = family_of(&g, &t);
    let text = "tempo 90\nkey am\ndeck A: kick x ~ x ~ kit=drums-cr78 | bass a1 ~ c2 ~ inst=sitar\ndeck B: harmony <chord(dm) chord(g) chord(em) chord(f)>\n";
    let c = compile(text);
    let w = style(&c, text, &t, &fam);
    let codes: Vec<&str> = w.iter().map(|x| x.code).collect();
    assert_eq!(codes, ["tempo", "kit", "voice", "progression"], "{w:?}");
    assert_eq!(w[0].message, "tempo 90 is slow for techno: it usually runs 120-150 BPM");
    assert_eq!((w[0].line, w[0].col), (Some(1), Some(1)));
    assert!(w[1].message.contains("kit drums-cr78 is unusual for techno (its kit is drums)"), "{}", w[1].message);
    assert_eq!((w[1].line, w[1].col), (Some(3), Some(22)));
    assert!(w[2].message.starts_with("voice sitar is unusual for techno"), "{}", w[2].message);
    assert!(w[3].message.contains("degrees 4 7 5 6 of the key"), "{}", w[3].message);
    assert!(w[3].message.contains("(1 7 1 7)"), "{}", w[3].message);
    assert_eq!(w[0].json()["severity"], "style");
    // the family's kit is not unusual: dnb (edm) uses drums-breakbeat
    let text = "tempo 128\nkey c\ndeck A: kick x ~ x ~ kit=drums-breakbeat\n";
    assert!(style(&compile(text), text, &t, &fam).is_empty());
    // a family word the genre itself names (acid) is not unusual either
    let text = "tempo 128\nkey c\ndeck A: bass c2 ~ c2 ~ inst=acid\n";
    assert!(style(&compile(text), text, &t, &fam).is_empty());
}

#[test]
fn what_the_ontology_does_not_say_is_not_warned() {
    let g = Nt::parse(&[include_str!("testdata/genres.nt")]);
    let t = find(&g, "techno").unwrap();
    assert_eq!((t.min_bpm, t.max_bpm), (None, None));
    let text = "tempo 60\nkey c\ndeck A: kick x ~ x ~\n";
    assert!(style(&compile(text), text, &t, &[]).is_empty());
}

#[test]
fn degrees_follow_the_harmony_bar_by_bar() {
    let c = compile("tempo 120\nkey am\ndeck B: harmony <chord(am) chord(f) chord(c) chord(g)>\n");
    assert_eq!(degrees(&c), Some(vec![1, 6, 3, 7]));
    let c = compile("tempo 120\nkey c\ndeck A: bass <c2 ~ ~ ~ g1 ~ ~ ~>\n");
    assert_eq!(degrees(&c).unwrap()[0], 1);
}

/// 6672: every chord of a bar is read, not only the first (a whole-bar pad Em .. C was "degrees 1").
#[test]
fn degrees_read_every_chord_of_a_bar() {
    let c = compile("tempo 140\nkey em\ndeck B: harmony [e3, g3, b3] ~ ~ ~ ~ ~ ~ ~ [c3, e3, g3] ~ ~ ~ ~ ~ ~ ~\n");
    assert_eq!(degrees(&c), Some(vec![1, 6]));
    let c = compile("tempo 140\nkey em\ndeck B: harmony [e3,g3,b3] ~ ~ ~ [d3,f#3,a3] ~ ~ ~ [c3,e3,g3] ~ ~ ~ [d3,f#3,a3] ~ ~ ~\n");
    assert_eq!(degrees(&c), Some(vec![1, 7, 6, 7]));
    // an inversion is the same chord; a chord struck again is not a new one
    let c = compile("tempo 140\nkey em\ndeck B: harmony [e3,g3,b3] ~ [e3,g3,b3] ~ [g3,b3,e4] ~ ~ ~ [c3,e3,g3] ~ ~ ~ ~ ~ ~ ~\n");
    assert_eq!(degrees(&c), Some(vec![1, 6]));
    // a line moving over the chord in the harmony part is not a chord change
    let c = compile("tempo 140\nkey em\ndeck B: harmony [e3,g3,b3] ~ ~ ~ a3 ~ ~ ~ c4 ~ ~ ~ ~ ~ ~ ~\n");
    assert_eq!(degrees(&c), Some(vec![1]));
}

#[test]
fn check_style_reads_the_header_and_never_fails() {
    let g = onto();
    let text = "#! genre: techno\ntempo 90\nkey am\ndeck A: kick x ~ x ~\n";
    assert_eq!(wanted(text, None).as_deref(), Some("techno"));
    assert_eq!(wanted(text, Some("jazz")).as_deref(), Some("jazz"));
    assert_eq!(wanted("tempo 90\n", Some("  ")), None);
    let v = check_style(Some(&g), text, "techno");
    assert_eq!(v["genre"], "https://beatscript.app/ontology/genre/techno");
    assert_eq!(v["warnings"][0]["code"], "tempo");
    assert_eq!(v["warnings"][0]["line"], 2);
    let v = check_style(Some(&g), text, "polka");
    assert_eq!(v["warnings"], json!([]));
    assert!(v["note"].as_str().unwrap().contains("no genre"));
    let v = check_style(None, text, "techno");
    assert!(v["note"].is_string());
    // a program with errors: no style warnings
    assert_eq!(check_style(Some(&g), "tempo 90\ndeck A: kick (\n", "techno")["warnings"], json!([]));
}

#[test]
fn the_template_answer_is_checked() {
    let g = onto();
    let a = template_answer(Some(&g), "house").unwrap();
    assert!(a["cid"].as_str().unwrap().starts_with('z'));
    assert_eq!(a["genre"]["label"], "house");
    assert_eq!(a["genre"]["tempo_range"], json!([118.0, 130.0]));
    assert!(template_answer(Some(&g), "polka").unwrap_err().contains("techno"));
    assert!(template_answer(None, "house").is_err());
}

/// Every pitched part of every segment, as (segment, deck, role, bar, step) -> the pitch classes sounding there.
fn pitch_classes(text: &str) -> BTreeMap<String, Vec<u8>> {
    let c = compile(text);
    let mut out = BTreeMap::new();
    for (i, seg) in c.segments.iter().enumerate() {
        for (deck, ir) in &seg.decks {
            for p in ir.parts.iter().filter(|p| matches!(p.role.as_str(), "bass" | "melody" | "harmony" | "arp")) {
                for (bar, hits) in p.cycles.iter().enumerate() {
                    let mut by_step: BTreeMap<u8, Vec<u8>> = BTreeMap::new();
                    for h in hits {
                        by_step.entry(h.step).or_default().push(h.note % 12);
                    }
                    for (step, mut pcs) in by_step {
                        pcs.sort_unstable();
                        pcs.dedup();
                        out.insert(format!("{i}:{} {deck:?} {} bar {bar} step {step}", seg.name, p.role), pcs);
                    }
                }
            }
        }
    }
    out
}

#[test]
fn a_template_follows_its_key() {
    // The template wrote the chords, the bass, the arp and the tune as pitch names: a `key` line moved nothing. Now they are
    // numerals, so the key moves every pitched note of every genre (each by the same number of semitones, in its own octave).
    let g = onto();
    let (voices, kits) = instrument_names();
    let mut seen = [0usize; 4];
    for slug in ["techno", "house", "jazz", "trance", "dnb"] {
        let genre = find(&g, slug).unwrap();
        let text = template(&g, &genre, &voices, &kits);
        assert!(!text.contains("chord("), "{slug}: {text}");
        let before = pitch_classes(&text);
        for (i, part) in [" bass ", " harmony ", " arp ", " melody "].iter().enumerate() {
            seen[i] += before.keys().filter(|k| k.contains(part)).count();
        }
        let key = format!("key {}\n", genre_key(&genre));
        assert!(text.contains(&key), "{slug}: {key:?} in {text}");
        for k in [1u8, 5, 7] {
            let k0 = Key::parse(genre_key(&genre).as_str()).unwrap();
            let moved = Key { tonic: (k0.tonic + k) % 12, mode: k0.mode };
            let after = pitch_classes(&text.replacen(&key, &format!("key {}\n", moved.text()), 1));
            assert_eq!(before.len(), after.len(), "{slug} +{k}");
            for (place, pcs) in &before {
                let mut want: Vec<u8> = pcs.iter().map(|p| (p + k) % 12).collect();
                want.sort_unstable();
                assert_eq!(after[place], want, "{slug} +{k}: {place} did not move with the key\n{text}");
            }
        }
    }
    // (so the walk met a bass, a pad, an arp and a melody to move)
    assert!(seen.iter().all(|n| *n > 8), "{seen:?}");
}

fn genre_key(genre: &Genre) -> String {
    Key { tonic: (genre.root_note.unwrap_or(0) % 12) as u8, mode: genre.scale.as_deref().and_then(mode_of).unwrap_or(1) }.text()
}

#[test]
fn the_template_is_the_progression_the_ontology_gives() {
    let g = onto();
    let (voices, kits) = instrument_names();
    // techno is 1 7 1 7 in C major: I vii° I vii°
    let text = template(&g, &find(&g, "techno").unwrap(), &voices, &kits);
    assert!(text.contains("let chords = <I vii° I vii°>\n"), "{text}");
    assert!(text.contains("let arp1 = I*4\n") && text.contains("let arp2 = vii°*4\n"), "{text}");
    assert!(text.contains("let tune1 = I/5 ~ ~ ~ ~ ~ I/3 ~ I ~ ~ ~ ~ ~ ~ ~\n"), "{text}");
    assert!(text.contains("let bass1 = I ~ ~ ~ I ~ ~ ~ I ~ ~ ~ I ~ ~ ~\n"), "{text}");
    // jazz is dorian: the diatonic triads of the mode
    let j = template(&g, &find(&g, "jazz").unwrap(), &voices, &kits);
    assert!(j.contains("let chords = <") && !j.contains("chord("), "{j}");
}

#[test]
fn degrees_see_through_voice_leading() {
    // a chord voiced in an inversion is still the chord on its degree: A C F, the lowest note of which is A, is VI in A minor
    let plain = compile("tempo 120\nkey am\ndeck B: harmony <i VI III VII>\n");
    let led = compile("tempo 120\nkey am\ndeck B: harmony <i VI III VII> voice-lead\n");
    assert_eq!(degrees(&plain), Some(vec![1, 6, 3, 7]));
    assert_eq!(degrees(&led), degrees(&plain));
    let one = |text: &str| degrees(&compile(&format!("tempo 120\n{text}\n"))).unwrap();
    assert_eq!(one("key am\ndeck B: harmony [a3,c4,f4]"), [6], "VI, first inversion");
    assert_eq!(one("key c\ndeck B: harmony [e3,g3,c4,e4]"), [1], "I, a note doubled");
    assert_eq!(one("key c\ndeck B: harmony [g3,c4,e4]"), [1], "I, second inversion");
    // notes that are no triad of the key are read by their lowest, as before
    assert_eq!(one("key c\ndeck B: harmony [c3,e3,g3,b3]"), [1], "a seventh chord");
    assert_eq!(one("key c\ndeck B: harmony [d3,g3]"), [2], "two notes");
    assert_eq!(one("key c\ndeck B: bass c2 ~ ~ ~"), [1]);
}

#[test]
fn a_generated_program_is_in_style_for_its_own_genre() {
    // `generate` writes `#! genre:` and voice-leads the pad: `check` reads the chords' roots, not their lowest notes, so it has
    // nothing to say about a progression the generator itself chose
    let g = onto();
    for slug in ["techno", "house", "jazz", "trance", "dnb"] {
        for seed in 1..=30 {
            let t = beatscript_api::generate::generate(&beatscript_api::generate::GenRequest { genre: slug.into(), seed: seed.to_string(), ..Default::default() }).unwrap().text;
            let v = check_style(Some(&g), &t, slug);
            let w: Vec<&Value> = v["warnings"].as_array().unwrap().iter().filter(|x| x["code"] == "progression").collect();
            assert!(w.is_empty(), "{slug} {seed}: {w:?}\n{t}");
        }
    }
}

/// Found phrases (docs/beatscript/found-phrases.md): a starting program with a phrase sung on each melody part, still checked, and the
/// answer says the line's source and meter and whether the meter fits the genre.
#[test]
fn a_template_with_a_phrase_sings_it_and_says_how_it_fits() {
    let g = onto();
    let plain = template_answer(Some(&g), "techno").unwrap();
    assert!(plain.get("phrase").is_none(), "no phrase asked for, none answered");
    let a = template_answer_with(Some(&g), "techno", Some("tyger_burning_bright")).unwrap();
    let text = a["text"].as_str().unwrap();
    assert!(text.contains("deck B: sing @tyger_burning_bright on melody"), "{text}");
    assert_eq!(a["phrase"]["id"], "tyger_burning_bright");
    assert_eq!(a["phrase"]["year"], 1794);
    assert_eq!(a["phrase"]["family"], "trochaic");
    assert_eq!(a["phrase"]["fit"]["at_home"], true, "{}", a["phrase"]["fit"]);
    assert!(a["phrase"]["sung_in_sections"].as_u64().unwrap() >= 1);
    assert_ne!(a["cid"], plain["cid"]);
    // a meter against the genre's grain is said so, and still sung
    let b = template_answer_with(Some(&g), "techno", Some("@byron_assyrian_wolf")).unwrap();
    assert_eq!(b["phrase"]["fit"]["at_home"], false);
    assert!(b["phrase"]["fit"]["note"].as_str().unwrap().contains("against techno's grain"), "{}", b["phrase"]["fit"]);
    // any words: their stress is worked out, and the answer says so
    let c = template_answer_with(Some(&g), "house", Some("robots on the road")).unwrap();
    assert!(c["text"].as_str().unwrap().contains("sing \"robots on the road\" on melody"));
    assert_eq!(c["phrase"]["stress_worked_out"], true);
    // an unknown id is an error with the way out; an empty phrase asks for none
    assert!(template_answer_with(Some(&g), "house", Some("@nope")).unwrap_err().contains("no phrase @nope"));
    assert_eq!(template_answer_with(Some(&g), "house", Some("  ")).unwrap()["text"], template_answer(Some(&g), "house").unwrap()["text"]);
}

/// Forms (docs/beatscript/forms.md): with `form`, the program is the form's template at the genre's tempo, still checked, and the
/// answer says the form's shape, plan, themes and reference and writes the program out; with `theme`, one theme of the library
/// goes into the genre's own program, in its key, on the part of its role. One or the other.
#[test]
fn a_template_takes_a_form_or_a_theme() {
    let g = onto();
    let o = "https://beatscript.app";
    let a = template_answer_full(Some(&g), "house", None, Some("quodlibet"), None, o).unwrap();
    let text = a["text"].as_str().unwrap();
    assert!(text.contains("arrange from template(quodlibet, @psalm42_deep_calleth)") && text.contains("#! genre: house"), "{text}");
    assert_eq!(a["form"]["name"], "quodlibet");
    assert_eq!(a["form"]["reference"]["name"], "canon-of-fate");
    assert!(a["form"]["themes"].as_array().unwrap().iter().any(|t| t == "ode_to_joy"));
    assert!(a["expanded"].as_str().unwrap().contains("let ode_to_joy_1 = "), "the answer writes the form out");
    // a phrase of one's own is sung instead, and reported as with no form
    let b = template_answer_full(Some(&g), "techno", Some("tyger_burning_bright"), Some("Theme and Variations"), None, o).unwrap();
    assert!(b["text"].as_str().unwrap().contains("template(theme-and-variations, @tyger_burning_bright)"));
    assert_eq!(b["phrase"]["year"], 1794);
    for f in crate::forms::names() {
        let r = template_answer_full(Some(&g), "techno", None, Some(f), None, o).unwrap_or_else(|e| panic!("{f}: {e}"));
        assert!(r["cid"].as_str().unwrap().starts_with('z'), "{f}");
    }
    assert!(template_answer_full(Some(&g), "house", None, Some("sonata"), None, o).unwrap_err().contains("the forms are"));
    // a theme: in the genre's own program, moved to its key, on its role's part
    let plain = template_answer(Some(&g), "house").unwrap();
    let t = template_answer_full(Some(&g), "house", None, None, Some("ode_to_joy"), o).unwrap();
    assert_ne!(t["cid"], plain["cid"]);
    let tt = t["text"].as_str().unwrap();
    assert!(tt.contains("let ode_to_joy_1 = ") && tt.contains(t["theme"]["played_by"].as_str().unwrap()), "{tt}");
    assert!(t["theme"]["played_in_sections"].as_u64().unwrap() >= 1, "{}", t["theme"]);
    assert!(template_answer_full(Some(&g), "house", None, Some("rondo"), Some("ode_to_joy"), o).unwrap_err().contains("not both"));
    assert!(template_answer_full(Some(&g), "house", None, None, Some("greensleeves"), o).unwrap_err().contains("no theme"));
}
