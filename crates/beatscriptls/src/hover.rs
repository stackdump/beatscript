//! Hover and signature help.
//!
//! Hover says what the word under the cursor is: a section's length, where it plays and for how long at the tempo; a
//! `let`'s value; a def's signature; a generator's signature and doc; a part parameter's or a registry param's doc, types
//! and options; a role, a control word, a statement keyword, a voice. Signature help names the parameters of the call the
//! cursor is in (a generator, a transform, a def) and of a `do` line's verb, with the one being typed active.
//!
//! A reference (a generator call, a seed, a stored program's section or part, a template: refs.rs) says what it is, what it
//! resolves to and its expansion; a `# from:` line an eject wrote says where its text came from.

use djengine::json::{JsonObject, JsonValue};
use djlive::diag::Span;
use djlive::lex::Tok;

use crate::analysis::Analysis;
use crate::complete::EnvNames;
use crate::json::{arr, obj, s, span_range};
use crate::lang;
use crate::symbols::Kind;
use crate::text::Encoding;
use crate::timeline::{mmss, section_bars, timeline};

fn code(t: &str) -> String {
    format!("```beatscript\n{t}\n```")
}

/// What to say of symbol `si`: its doc comment (the `#` lines above its definition), then what it is.
pub fn symbol_markdown(a: &Analysis, si: usize) -> String {
    let m = symbol_facts(a, si);
    let sym = &a.symbols.symbols[si];
    let documented = matches!(sym.kind, Kind::Section | Kind::Let | Kind::LetSection | Kind::Def | Kind::Arrangement);
    match a.doc_at(sym.name_span.start).filter(|_| documented && !m.is_empty()) {
        Some(doc) => format!("{doc}\n\n{m}"),
        None => m,
    }
}

/// What symbol `si` is, without its doc comment.
fn symbol_facts(a: &Analysis, si: usize) -> String {
    let sym = &a.symbols.symbols[si];
    let uses = a.symbols.refs_of(si).filter(|r| !r.is_def).count();
    let used = match uses {
        0 => "not used".to_string(),
        1 => "used once".to_string(),
        n => format!("used {n} times"),
    };
    match sym.kind {
        Kind::Section => {
            let mut m = code(&format!("section {} {}", sym.name, sym.detail));
            let tl = timeline(a);
            match tl.first(&sym.name) {
                Some(p) => {
                    let n = tl.count(&sym.name);
                    let mut line = format!("{} bars, bars {}–{}", p.bars, p.start + 1, p.start + p.bars);
                    if let (Some(at), Some(len)) = (p.at_secs, p.secs) {
                        let bpm = tl.tempo.map_or(String::new(), |t| format!(" at {} BPM", djlive::print::num(t)));
                        line.push_str(&format!(" · {} long, from {} to {}{bpm}", mmss(len), mmss(at), mmss(at + len)));
                    }
                    if n > 1 {
                        line.push_str(&format!(" · placed {n} times (the first shown)"));
                    }
                    if !tl.exact {
                        line.push_str(" · from the text (the check is not done)");
                    }
                    m.push_str(&format!("\n\n{line}"));
                }
                None => {
                    if let (Some(bars), Some(t)) = (section_bars(a, sym), tl.tempo) {
                        m.push_str(&format!("\n\n{bars} bars, {} at {} BPM · not placed by the arrange", mmss(f64::from(bars) * 240.0 / t), djlive::print::num(t)));
                    } else {
                        m.push_str("\n\nnot placed by an arrange");
                    }
                }
            }
            m.push_str(&format!(" · {used}"));
            m
        }
        Kind::Let => {
            let mut m = code(&format!("let {} = {}", sym.name, sym.detail));
            // a scalar let: the number it came to (the parser folded the arithmetic)
            if let Some(l) = a.scalar_lets.iter().find(|l| l.at == sym.name_span) {
                if l.text.trim() != djlive::print::num(l.value) {
                    m.push_str(&format!("\n\n= {}", djlive::print::num(l.value)));
                }
            }
            m.push_str(&format!("\n\n{used}"));
            m
        }
        Kind::LetSection => format!("{}\n\nsections composed: {used}", code(&format!("let {} = {}", sym.name, sym.detail))),
        Kind::Def => {
            let ports: Vec<String> = sym.children.iter().map(|&c| &a.symbols.symbols[c]).filter(|c| c.kind == Kind::Port).map(|c| format!("`{}` ({})", c.name, c.detail)).collect();
            let mut m = code(&format!("def {}{}", sym.name, sym.detail));
            if !ports.is_empty() {
                m.push_str(&format!("\n\nports: {}", ports.join(", ")));
            }
            m.push_str(&format!("\n\n{used}"));
            m
        }
        Kind::Param | Kind::Port => {
            let owner = sym.parent.map(|p| a.symbols.symbols[p].name.clone()).unwrap_or_default();
            format!("{}\n\n{} of `def {owner}`, {used}", code(&format!("${}", sym.name)), sym.detail)
        }
        Kind::Sample => format!("{}\n\na sound this program names: `inst={}` plays it · {used}", code(sym.full.text(&a.text)), sym.name),
        Kind::Import => format!("{}\n\na saved snippet's lets and defs", code(sym.full.text(&a.text))),
        Kind::Arrangement => format!("{}\n\na named arrangement: `then {}` goes on to it, `start {}` plays it first · {used}", code(sym.full.text(&a.text)), sym.name, sym.name),
        _ => String::new(),
    }
}

/// Is token `i` on a line that starts with `~` or `@` (an arrange mutation)?
pub fn on_mutation_line(t: &[djlive::lex::Token], i: usize) -> bool {
    let mut j = i;
    while j > 0 && !matches!(t[j - 1].tok, Tok::Newline | Tok::LBrace) {
        j -= 1;
    }
    matches!(t.get(j).map(|x| &x.tok), Some(Tok::Tilde | Tok::At))
}

/// The markdown for the word at token `i`, when it is not a symbol.
fn word_markdown(a: &Analysis, i: usize, env: &EnvNames) -> Option<String> {
    let t = &a.tokens;
    let Tok::Word(w) = &t[i].tok else { return None };
    let next = t.get(i + 1).map(|n| &n.tok);
    let prev = if i > 0 { Some(&t[i - 1].tok) } else { None };
    let at_start = i == 0 || matches!(prev, Some(Tok::Newline | Tok::LBrace));
    // an operator of an arrange mutation line (`~ acid_a rot 2 | thin 0.1`)
    if on_mutation_line(t, i) {
        if let Some((_, sig, doc)) = lang::mutation_ops().into_iter().find(|(n, _, _)| *n == w.as_str()) {
            return Some(format!("{}\n\narrange mutation · {doc}", code(&sig)));
        }
    }
    // a generator call's head (when the call cannot be worked out: a working one is hovered as a reference)
    if crate::refs::is_gen_head(w) && matches!(next, Some(Tok::Word(_))) {
        return Some(format!("{}\n\na call of the seeded generator (an engine call): the program it prints for the genre, the seed and the rest, pinned by its CID; `section=` and `part=` say which piece of it this is", code("gen@2 GENRE seed=N [bpm=B] [key=K] [length=short|standard|long] [section=NAME] [part=ROLE]")));
    }
    if next == Some(&Tok::LParen) {
        if let Some(c) = lang::call(w) {
            return Some(format!("{}\n\n{}", code(&c.label()), c.doc));
        }
    }
    // a param's name, `vel=` / `channel=`
    if next == Some(&Tok::Eq) {
        if let Some((n, d, _)) = lang::PART_PARAMS.iter().find(|(n, _, _)| n == w) {
            return Some(format!("{}\n\n{d}", code(&format!("{n}="))));
        }
        let p = djlive::partial::statement_at(&a.text, t[i].span.start);
        let words: Vec<&str> = p.tokens.iter().filter_map(|t| if let Tok::Word(w) = &t.tok { Some(w.as_str()) } else { None }).collect();
        if let Some(spec) = do_param(&words, w, env) {
            return Some(param_markdown(&spec));
        }
    }
    // the value of `inst=`
    if matches!(prev, Some(Tok::Eq)) && i >= 2 && matches!(&t[i - 2].tok, Tok::Word(p) if p == "inst") {
        if env.voices.iter().any(|v| v == w) {
            return Some(format!("`{w}`: a voice with a sound of its own"));
        }
        if let Some(f) = env.families.iter().find(|f| w.contains(f.as_str())) {
            return Some(format!("`{w}`: no sound of its own; it has the family word `{f}`, so it plays that family's generic voice"));
        }
    }
    if djlive::parse::ROLES.contains(&w.as_str()) {
        return Some(format!("`{w}`: {}", lang::role_doc(w)));
    }
    if at_start {
        if let Some((k, doc, _)) = lang::statement(w) {
            // the program's one `arrange` has no name to hover: its keyword shows its doc comment
            let own = if *k == "arrange" { a.doc_at(t[i].span.start).map(|d| format!("{d}\n\n")).unwrap_or_default() } else { String::new() };
            return Some(format!("{own}`{k}`: {doc}"));
        }
    }
    let doc = lang::action_doc(w);
    if !doc.is_empty() {
        return Some(format!("`{w}`: {doc}"));
    }
    if let Some((op, d)) = lang::EVOLVE_OPS.iter().find(|(o, _)| o == w) {
        return Some(format!("`{op}` (evolve): {d}"));
    }
    // a do line's noun or verb
    let p = djlive::partial::statement_at(&a.text, t[i].span.end);
    let mut words: Vec<&str> = p.tokens.iter().filter_map(|t| if let Tok::Word(w) = &t.tok { Some(w.as_str()) } else { None }).collect();
    words.push(w);
    if let Some(m) = do_word(&words, env) {
        return Some(m);
    }
    None
}

fn param_markdown(p: &djlive::env::ParamSpec) -> String {
    let mut m = code(&format!("{}=", p.name));
    let mut facts = Vec::new();
    if !p.types.is_empty() {
        facts.push(p.types.join(" | "));
    }
    if !p.options.is_empty() {
        facts.push(format!("one of {}", p.options.iter().map(|o| format!("`{o}`")).collect::<Vec<_>>().join(", ")));
    }
    if p.required {
        facts.push("required".into());
    }
    if !facts.is_empty() {
        m.push_str(&format!("\n\n{}", facts.join(" · ")));
    }
    if !p.doc.is_empty() {
        m.push_str(&format!("\n\n{}", p.doc));
    }
    m
}

/// The registry's spec of param `name` on the `do` line whose words are `words`.
fn do_param(words: &[&str], name: &str, env: &EnvNames) -> Option<djlive::env::ParamSpec> {
    let (noun_at, sugar) = match words.first()? {
        &"do" => (1, false),
        w if djlive::parse::sugar_noun(w).is_some() => (0, true),
        _ => return None,
    };
    let nw = *words.get(noun_at)?;
    let noun = env.registry.noun(if sugar { djlive::parse::sugar_noun(nw).unwrap_or(nw) } else { nw })?;
    let verb = words.get(noun_at + 1).and_then(|v| noun.action(v));
    verb.and_then(|v| v.params.iter().find(|p| p.name == name).cloned()).or_else(|| noun.param(name).cloned())
}

fn do_word(words: &[&str], env: &EnvNames) -> Option<String> {
    let noun_at = match words.first()? {
        &"do" => 1,
        w if djlive::parse::sugar_noun(w).is_some() && *w != "key" => 0,
        _ => return None,
    };
    let nw = *words.get(noun_at)?;
    let noun = env.registry.noun(djlive::parse::sugar_noun(nw).filter(|_| noun_at == 0).unwrap_or(nw))?;
    if words.len() == noun_at + 1 {
        let verbs: Vec<String> = noun.actions.iter().map(|a| format!("`{}` ({})", a.name, a.effect)).collect();
        let def = if noun.default_action.is_empty() { String::new() } else { format!(" · the default: `{}`", noun.default_action) };
        return Some(format!("{}\n\nverbs: {}{def}", code(&format!("do {}", noun.name)), if verbs.is_empty() { "none".into() } else { verbs.join(", ") }));
    }
    if words.len() == noun_at + 2 {
        let act = noun.action(words[noun_at + 1])?;
        let params = if act.params.is_empty() { &noun.params } else { &act.params };
        let ps: Vec<String> = params.iter().map(|p| format!("`{}=`{}", p.name, if p.required { " (required)" } else { "" })).collect();
        return Some(format!("{}\n\n{} · params: {}", code(&format!("do {} {}", noun.name, act.name)), act.effect, if ps.is_empty() { "none".into() } else { ps.join(", ") }));
    }
    None
}

fn markdown_at(a: &Analysis, md: String, span: Span, enc: Encoding) -> JsonValue {
    obj([("contents", obj([("kind", s("markdown")), ("value", s(md))])), ("range", span_range(&a.index, &a.text, span, enc))])
}

/// `textDocument/hover`: a reference's own text first (refs.rs), then the word, then the statement a reference stands in.
pub fn hover(a: &Analysis, at: usize, env: &EnvNames, enc: Encoding) -> JsonValue {
    if a.in_comment(at) {
        // a `# from: <call> -> <cid>` line eject wrote
        let line = a.index.line_of(at);
        let (ls, le) = (a.index.line_start(line), a.index.line_end(&a.text, line));
        return match crate::refs::from_line_markdown(&a.text[ls..le]) {
            Some(m) => markdown_at(a, m, Span::new(ls + (a.text[ls..le].len() - a.text[ls..le].trim_start().len()), le), enc),
            None => JsonValue::Null,
        };
    }
    if let Some(site) = crate::refs::site_on(a, at) {
        return markdown_at(a, crate::refs::hover_markdown(a, site), site.span, enc);
    }
    match word_hover(a, at, env, enc) {
        JsonValue::Null => match crate::refs::site_at(a, at) {
            Some(site) => markdown_at(a, crate::refs::hover_markdown(a, site), site.span, enc),
            None => JsonValue::Null,
        },
        h => h,
    }
}

/// The hover of the word or symbol at `at` (no reference).
fn word_hover(a: &Analysis, at: usize, env: &EnvNames, enc: Encoding) -> JsonValue {
    let (md, span) = if let Some(r) = a.symbols.ref_at(at) {
        (symbol_markdown(a, r.symbol), r.span)
    } else if let Some(i) = a.token_at(at) {
        let mark = match a.tokens[i].tok {
            Tok::At => Some("`@`: a one-shot mutation: the next section reads the targets changed, then they revert"),
            Tok::Tilde if on_mutation_line(&a.tokens, i) => Some("`~`: a persistent mutation: every section after it reads the targets changed"),
            _ => None,
        };
        if let Some(m) = mark {
            return obj([("contents", obj([("kind", s("markdown")), ("value", s(m))])), ("range", span_range(&a.index, &a.text, a.tokens[i].span, enc))]);
        }
        // `~3`: the count after the mark
        if let (Tok::Num { value, .. }, true) = (&a.tokens[i].tok, i > 0 && matches!(a.tokens[i - 1].tok, Tok::Tilde | Tok::At)) {
            let n = value.round().max(1.0) as u32;
            let m = format!("the count: fires only on the passes of its `arrange` that are a multiple of {n} (passes {n}, {}, {} ...), not on the others", 2 * n, 3 * n);
            return obj([("contents", obj([("kind", s("markdown")), ("value", s(&m))])), ("range", span_range(&a.index, &a.text, a.tokens[i].span, enc))]);
        }
        match word_markdown(a, i, env) {
            Some(m) => (m, a.tokens[i].span),
            None => return JsonValue::Null,
        }
    } else {
        return JsonValue::Null;
    };
    if md.is_empty() {
        return JsonValue::Null;
    }
    obj([("contents", obj([("kind", s("markdown")), ("value", s(md))])), ("range", span_range(&a.index, &a.text, span, enc))])
}

/// A signature: its label and the offsets of each parameter in it.
struct Signature {
    label: String,
    params: Vec<(usize, usize)>,
    doc: String,
    active: usize,
}

fn sig_of(name: &str, params: &[String], doc: &str, active: usize, open: &str, close: &str, sep: &str) -> Signature {
    let mut label = format!("{name}{open}");
    let mut offs = Vec::new();
    for (k, p) in params.iter().enumerate() {
        if k > 0 {
            label.push_str(sep);
        }
        let st = label.encode_utf16().count();
        label.push_str(p);
        offs.push((st, label.encode_utf16().count()));
    }
    label.push_str(close);
    Signature { label, params: offs, doc: doc.to_string(), active }
}

/// The call the cursor is in: the name before the unmatched `(` and which argument it is at.
fn enclosing_call(a: &Analysis, at: usize) -> Option<(String, usize, Span)> {
    let p = djlive::partial::statement_at(&a.text, at);
    let toks = &p.tokens;
    let mut depth = 0i32;
    let mut commas = 0usize;
    for k in (0..toks.len()).rev() {
        match &toks[k].tok {
            Tok::RParen | Tok::RBrack => depth += 1,
            Tok::LBrack if depth > 0 => depth -= 1,
            Tok::LBrack => {}
            Tok::LParen if depth > 0 => depth -= 1,
            Tok::LParen => {
                if k == 0 {
                    return None;
                }
                if let Tok::Word(w) = &toks[k - 1].tok {
                    return Some((w.clone(), commas, toks[k - 1].span));
                }
                return None;
            }
            Tok::Comma | Tok::OctDown if depth == 0 => commas += 1,
            _ => {}
        }
    }
    None
}

/// `textDocument/signatureHelp`.
pub fn signature_help(a: &Analysis, at: usize, env: &EnvNames) -> JsonValue {
    let sig = if let Some((name, arg, _)) = enclosing_call(a, at) {
        if let Some((_, d)) = a.symbols.named(&name).find(|(_, s)| s.kind == Kind::Def) {
            let params: Vec<String> = d.children.iter().map(|&c| &a.symbols.symbols[c]).filter(|c| c.kind == Kind::Param).map(|c| c.name.clone()).collect();
            Some(sig_of(&name, &params, &format!("def {}{}", d.name, d.detail), arg, "(", ")", ", "))
        } else if let Some(c) = lang::call(&name) {
            let params: Vec<String> = c.params.iter().map(|p| p.to_string()).collect();
            // the optional key of markov: `markov(0.7)` is the density
            Some(sig_of(c.name, &params, c.doc, arg, "(", ")", ", "))
        } else {
            None
        }
    } else {
        do_signature(a, at, env)
    };
    let Some(sig) = sig else { return JsonValue::Null };
    let active = sig.active.min(sig.params.len().saturating_sub(1));
    let mut o = JsonObject::new();
    o.set("label", s(sig.label.clone()));
    if !sig.doc.is_empty() {
        o.set("documentation", obj([("kind", s("markdown")), ("value", s(sig.doc.clone()))]));
    }
    o.set("parameters", arr(sig.params.iter().map(|(x, y)| obj([("label", arr(vec![JsonValue::int(*x as i64), JsonValue::int(*y as i64)]))])).collect()));
    obj([("signatures", arr(vec![JsonValue::Object(o)])), ("activeSignature", JsonValue::int(0)), ("activeParameter", JsonValue::int(active as i64))])
}

/// A `do` line's verb: its params, the one being typed (or the next one not given) active.
fn do_signature(a: &Analysis, at: usize, env: &EnvNames) -> Option<Signature> {
    let p = djlive::partial::statement_at(&a.text, at);
    let words: Vec<&str> = p.tokens.iter().filter_map(|t| if let Tok::Word(w) = &t.tok { Some(w.as_str()) } else { None }).collect();
    let noun_at = match words.first()? {
        &"do" => 1,
        w if djlive::parse::sugar_noun(w).is_some() && *w != "key" => 0,
        _ => return None,
    };
    let nw = *words.get(noun_at)?;
    let noun = env.registry.noun(djlive::parse::sugar_noun(nw).filter(|_| noun_at == 0).unwrap_or(nw))?;
    let act = words.get(noun_at + 1).and_then(|v| noun.action(v));
    let params = act.map(|a| &a.params).filter(|p| !p.is_empty()).unwrap_or(&noun.params);
    if params.is_empty() {
        return None;
    }
    let labels: Vec<String> = params.iter().map(|p| format!("{}={}", p.name, if p.options.is_empty() { p.types.join("|") } else { p.options.join("|") })).collect();
    // the param being typed: the last `name=` before the cursor, else the first not given
    let given: Vec<&str> = p.tokens.windows(2).filter(|w| w[1].tok == Tok::Eq).filter_map(|w| if let Tok::Word(n) = &w[0].tok { Some(n.as_str()) } else { None }).collect();
    let typing = if p.tokens.last().map(|t| &t.tok) == Some(&Tok::Eq) || matches!(p.tokens.last().map(|t| &t.tok), Some(Tok::Word(_) | Tok::Num { .. })) && !p.word.is_empty() { given.last().copied() } else { None };
    let active = typing.and_then(|n| params.iter().position(|q| q.name == n)).or_else(|| params.iter().position(|q| !given.contains(&q.name.as_str()))).unwrap_or(0);
    let head = format!("do {}{} ", noun.name, act.map_or(String::new(), |a| format!(" {}", a.name)));
    let doc = params.get(active).map(|q| q.doc.clone()).unwrap_or_default();
    Some(sig_of(&head, &labels, &doc, active, "", "", " "))
}
