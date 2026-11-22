//! `do noun verb key=value ...` (and the sugar for it) compiled against the control-language
//! registry: the noun and verb must exist, every param must be one the noun takes, with the right
//! type and (where the registry lists them) one of the allowed words, and what a verb needs it must
//! have. The errors say where, in line and column, and what would have been accepted.

use djengine::json::{JsonObject, JsonValue};

use crate::ast::*;
use crate::compile::{Call, Effect, Ramp};
use crate::diag::{Diagnostic, Span};
use crate::env::{suggest, Env, NounSpec, ParamSpec};
use crate::parse::sugar_noun;

fn word_list(items: &[String]) -> Vec<&str> {
    items.iter().map(String::as_str).collect()
}

fn num_json(n: f64) -> JsonValue {
    if n.fract() == 0.0 && n.abs() < 9e15 {
        JsonValue::int(n as i64)
    } else {
        JsonValue::Number(n)
    }
}

/// One value as the JSON the registry call takes, checked against what the param is.
fn value_json(v: &DoValue, spec: Option<&ParamSpec>, src: &str, diags: &mut Vec<Diagnostic>) -> Option<JsonValue> {
    let has = |t: &str| spec.is_none_or(|s| s.types.is_empty() || s.types.iter().any(|x| x == t || x == "any"));
    let name = spec.map_or("", |s| s.name.as_str());
    let bad = |diags: &mut Vec<Diagnostic>, span: Span, want: &str| {
        diags.push(Diagnostic::error(span, format!("`{name}` is {want}")).found(span.text(src).to_string()));
        None
    };
    match v {
        DoValue::Num(n) => {
            if has("number") {
                Some(num_json(n.value))
            } else if has("integer") {
                // (a performed value is rounded when it is read: its placeholder is not checked)
                if n.value.fract() != 0.0 && !crate::perform::is_live(n) {
                    bad(diags, n.span, "a whole number")
                } else {
                    Some(num_json(n.value))
                }
            } else if has("string") {
                // a number where text is wanted: a seed, a bank, a CID-like
                Some(JsonValue::str(if n.canon.is_some() { crate::print::num(n.value) } else { n.span.text(src).to_string() }))
            } else if has("boolean") {
                bad(diags, n.span, "on or off")
            } else {
                bad(diags, n.span, "not a number")
            }
        }
        DoValue::Ramp(a, b) => {
            let _ = b;
            if has("number") || has("integer") {
                Some(num_json(a.value))
            } else {
                bad(diags, v.span(), "not a number, so it cannot ramp")
            }
        }
        DoValue::Lfo(l) => {
            if has("number") || has("integer") {
                Some(num_json(l.center_value()))
            } else {
                bad(diags, l.span, "not a number, so an lfo cannot move it")
            }
        }
        DoValue::Str(s, sp) => {
            if has("string") {
                Some(JsonValue::str(s.clone()))
            } else {
                bad(diags, *sp, "not text")
            }
        }
        DoValue::List(items, sp) => {
            if !has("array") {
                return bad(diags, *sp, "not a list");
            }
            let mut out = Vec::new();
            for it in items {
                out.push(value_json(it, None, src, diags)?);
            }
            Some(JsonValue::Array(out))
        }
        DoValue::Word(w) => {
            let t = w.text.as_str();
            let spec = match spec {
                Some(s) => s,
                None => return Some(JsonValue::str(t)),
            };
            if let Some(opts) = (!spec.options.is_empty()).then_some(&spec.options) {
                if opts.iter().any(|o| o == t) {
                    return Some(JsonValue::str(t));
                }
                if has("boolean") && bool_word(t).is_some() {
                    return Some(JsonValue::Bool(bool_word(t).unwrap()));
                }
                if spec.types.iter().any(|x| x == "string") && opts.is_empty() {
                    return Some(JsonValue::str(t));
                }
                let mut d = Diagnostic::error(w.span, format!("`{t}` is not a value of `{name}`")).expected(&word_list(opts)).found(format!("`{t}`"));
                if let Some(n) = suggest(t, opts, 1).first() {
                    d = d.help(format!("did you mean `{n}`?"));
                }
                diags.push(d);
                return None;
            }
            if has("boolean") {
                if let Some(b) = bool_word(t) {
                    return Some(JsonValue::Bool(b));
                }
            }
            if has("string") || spec.types.is_empty() {
                return Some(JsonValue::str(t));
            }
            if has("null") && t == "none" {
                return Some(JsonValue::Null);
            }
            let want = if has("boolean") { "on or off (true / false)" } else if has("number") || has("integer") { "a number" } else { "not a word" };
            bad(diags, w.span, want)
        }
    }
}

fn bool_word(t: &str) -> Option<bool> {
    match t {
        "true" | "on" | "yes" => Some(true),
        "false" | "off" | "no" => Some(false),
        _ => None,
    }
}

/// The `btw` verbs that change or read the program that runs (`eval`, `stop`, `load`, `open` with `eval`, `replay`, `perform`): a program
/// cannot call them (R-11).
const MOVES_THE_PROGRAM: [&str; 6] = ["eval", "stop", "load", "open", "replay", "perform"];

/// The call (or ramp) a `do` line stands for.
pub fn compile_do(d: &DoCall, env: &Env, src: &str, diags: &mut Vec<Diagnostic>, seg_end: Option<u32>) -> Option<Effect> {
    let noun_word = &d.noun;
    // sugar words may stand for another noun (`pd705` is `kits`)
    let noun_name = if d.sugar { sugar_noun(&noun_word.text).unwrap_or(&noun_word.text) } else { noun_word.text.as_str() };
    let spec: Option<NounSpec> = match env.registry {
        Some(reg) => match reg.noun(noun_name) {
            Some(n) => Some(n),
            None => {
                let all = reg.noun_names();
                let mut e = Diagnostic::error(noun_word.span, format!("`{}` is not a noun of the control language", noun_word.text)).expected(&word_list(&all)).found(format!("`{}`", noun_word.text));
                if let Some(n) = suggest(&noun_word.text, &all, 3).first() {
                    e = e.help(format!("did you mean `{n}`? (`describe` lists the nouns)"));
                } else {
                    e = e.help("`describe` lists the nouns");
                }
                diags.push(e);
                return None;
            }
        },
        None => None,
    };
    let canonical_noun = spec.as_ref().map_or_else(|| noun_name.to_string(), |s| s.name.clone());
    // the verb
    let mut verb: Option<String> = d.verb.as_ref().map(|v| v.text.clone());
    if let Some(s) = &spec {
        match (&d.verb, s.actions.is_empty()) {
            (Some(v), true) => {
                diags.push(Diagnostic::error(v.span, format!("`{}` has no verbs: write its params, `do {} key=value`", s.name, s.name)).found(format!("`{}`", v.text)));
                return None;
            }
            (Some(v), false) => {
                if s.action(&v.text).is_none() {
                    let all: Vec<String> = s.actions.iter().map(|a| a.name.clone()).collect();
                    let mut e = Diagnostic::error(v.span, format!("`{}` has no verb `{}`", s.name, v.text)).expected(&word_list(&all)).found(format!("`{}`", v.text));
                    if let Some(n) = suggest(&v.text, &all, 1).first() {
                        e = e.help(format!("did you mean `{n}`?"));
                    }
                    diags.push(e);
                    return None;
                }
            }
            (None, false) => {
                if s.default_action.is_empty() {
                    let all: Vec<String> = s.actions.iter().map(|a| a.name.clone()).collect();
                    diags.push(Diagnostic::error(noun_word.span, format!("`{}` needs a verb", s.name)).expected(&word_list(&all)));
                    return None;
                }
                verb = Some(s.default_action.clone());
            }
            (None, true) => {}
        }
    }
    // `btw` verbs that act on the running program cannot be called by it (R-11): the runtime holds the program while its lines run, so
    // the call would act on an empty placeholder and be written over: `at bar 4: do btw stop` checked clean and did nothing, `do btw
    // eval` changed the window's text and not what plays
    if matches!(canonical_noun.as_str(), "btw" | "live" | "livecode") {
        if let Some(v) = verb.as_deref().filter(|v| MOVES_THE_PROGRAM.contains(v)) {
            let at = d.verb.as_ref().map_or(noun_word.span, |w| w.span);
            diags.push(
                Diagnostic::error(at, format!("`do btw {v}` cannot be called from a program: it would act on the program that is calling it"))
                    .found(format!("`{v}`"))
                    .help("edit the program in the window, or call the verb from outside it (the app's socket, `djctl app btw ...`); the verbs a program may call are `check`, `fmt`, `get`, `save`, `list`, `share`, `jsonld`, `terms`, `instruments` ..."),
            );
            return None;
        }
    }
    let action = spec.as_ref().zip(verb.as_deref()).and_then(|(s, v)| s.action(v));
    let mut o = JsonObject::new();
    if let (Some(s), Some(v)) = (&spec, &verb) {
        if !s.actions.is_empty() {
            o.set("action", JsonValue::str(v.clone()));
        }
    } else if let Some(v) = &verb {
        o.set("action", JsonValue::str(v.clone()));
    }
    // positional strings fill the verb's first required text param
    let mut errors = false;
    let mut given: Vec<String> = Vec::new();
    for p in &d.params {
        given.push(p.name.text.clone());
    }
    for (text, sp) in &d.positional {
        let target = action
            .and_then(|a| a.params.iter().find(|p| p.required && p.types.iter().any(|t| t == "string") && !given.contains(&p.name)))
            .or_else(|| action.and_then(|a| a.params.iter().find(|p| p.types.iter().any(|t| t == "string") && !given.contains(&p.name))))
            .map(|p| p.name.clone());
        match target {
            Some(n) => {
                o.set(n.as_str(), JsonValue::str(text.clone()));
                given.push(n);
            }
            None if spec.is_none() => {
                o.set("text", JsonValue::str(text.clone()));
            }
            None => {
                diags.push(Diagnostic::error(*sp, "this text has no param to go to").help("name it: key=\"...\""));
                errors = true;
            }
        }
    }
    // the params
    let mut ramp: Option<(String, f64, f64, Span)> = None;
    // `value=lfo tri 1 bar depth=0.2 center=0.3`: the one param of the line that keeps moving (F4, shape.rs)
    let mut lfo: Option<(String, &crate::shape::LfoSpec)> = None;
    for p in &d.params {
        let pname = p.name.text.as_str();
        if pname == "action" {
            diags.push(Diagnostic::error(p.name.span, "the verb is written after the noun, not as `action=`").help(format!("do {} <verb> ...", noun_name)));
            errors = true;
            continue;
        }
        let pspec = spec.as_ref().and_then(|s| s.param(pname));
        if let Some(s) = &spec {
            if pspec.is_none() {
                let all: Vec<String> = s.params.iter().map(|x| x.name.clone()).collect();
                let mut e = Diagnostic::error(p.name.span, format!("`{}` has no param `{pname}`", s.name)).expected(&word_list(&all)).found(format!("`{pname}`"));
                if let Some(n) = suggest(pname, &all, 1).first() {
                    e = e.help(format!("did you mean `{n}`?"));
                }
                diags.push(e);
                errors = true;
                continue;
            }
        }
        if o.contains_key(pname) {
            diags.push(Diagnostic::error(p.name.span, format!("`{pname}` is given twice")));
            errors = true;
            continue;
        }
        if let DoValue::Ramp(a, b) = &p.value {
            if ramp.is_some() {
                diags.push(Diagnostic::error(p.value.span(), "one param a line can ramp").help("split it into two lines, each with its own `over`"));
                errors = true;
                continue;
            }
            if lfo.is_some() {
                diags.push(Diagnostic::error(p.value.span(), "one param a line can move").help("split it into two lines: a param can be a ramp or an lfo, one of them to a line"));
                errors = true;
                continue;
            }
            ramp = Some((pname.to_string(), a.value, b.value, p.value.span()));
        }
        if let DoValue::Lfo(l) = &p.value {
            if ramp.is_some() || lfo.is_some() {
                diags.push(Diagnostic::error(p.value.span(), "one param a line can move").help("split it into two lines: a param can be a ramp or an lfo, one of them to a line"));
                errors = true;
                continue;
            }
            lfo = Some((pname.to_string(), l));
        }
        match value_json(&p.value, pspec, src, diags) {
            Some(v) => o.set(pname, v),
            None => errors = true,
        }
    }
    // what the verb needs
    if let Some(a) = action {
        for r in a.params.iter().filter(|p| p.required) {
            if !o.contains_key(r.name.as_str()) {
                diags.push(
                    Diagnostic::error(d.verb.as_ref().map_or(noun_word.span, |v| v.span), format!("`{} {}` needs `{}`", canonical_noun, a.name, r.name))
                        .help(format!("{}={}", r.name, match r.types.first().map(String::as_str) { Some("string") => "\"...\"", Some("boolean") => "on", _ => "1" })),
                );
                errors = true;
            }
        }
    }
    // a shape belongs to a ramp
    if let (Some((sh, ssp)), None) = (&d.shape, &ramp) {
        diags.push(Diagnostic::error(*ssp, format!("`{}` needs a ramp: `mix=0 -> 1 over 8 bars {}`", sh.name(), sh.name())).help("do vocoder set mix=0 -> 1 over 8 bars exp"));
        errors = true;
    }
    // the ramp
    match (&ramp, &d.over) {
        _ if lfo.is_some() => {}
        (Some(_), None) => {
            let sp = ramp.as_ref().unwrap().3;
            diags.push(Diagnostic::error(sp, "a ramp needs a length: `... over 8 bars`"));
            errors = true;
        }
        (None, Some((_, osp))) => {
            diags.push(Diagnostic::error(*osp, "`over` has nothing to move: write a param as `a -> b`").help("do vocoder set mix=0 -> 1 over 8 bars"));
            errors = true;
        }
        _ => {}
    }
    if errors {
        return None;
    }
    if let Some((field, l)) = lfo {
        // the lfo's own `over`, or the line's
        let over = l.over.or(d.over).map(|(dur, _)| dur);
        let mut base = o.clone();
        base.remove(&field);
        return Some(Effect::Lfo(crate::shape::Lfo {
            prelude: Vec::new(),
            noun: canonical_noun,
            base: JsonValue::Object(base),
            field,
            wave: l.wave,
            period: l.rate,
            center: l.center_value(),
            depth: l.depth_value(),
            phase: l.phase_value(),
            over,
            until: seg_end,
        }));
    }
    if let (Some((field, from, to, _)), Some((bars, osp))) = (ramp, &d.over) {
        let bars = *bars;
        if bars.is_zero() {
            diags.push(Diagnostic::error(*osp, "a ramp takes some time").help("over 1 bars, over 2 beats"));
            return None;
        }
        let mut base = o.clone();
        base.remove(&field);
        let shape = d.shape.map_or(crate::shape::Shape::Linear, |(s, _)| s);
        return Some(Effect::Ramp(Ramp { prelude: Vec::new(), noun: canonical_noun, base: JsonValue::Object(base), field, from, to, bars, shape }));
    }
    Some(Effect::Calls(vec![Call { noun: canonical_noun, params: JsonValue::Object(o) }]))
}

/// What puts a call back: the other half of a pair (`on`/`off`, `arm`/`cancel`, `mute`), or a switch
/// flipped (`on: true`, `mute: true`).
pub fn undo_of(c: &Call) -> Option<Call> {
    let o = c.params.as_object()?;
    let flip = |key: &str| -> Option<Call> {
        if o.get(key).and_then(JsonValue::as_bool) == Some(true) {
            let mut n = o.clone();
            n.set(key, JsonValue::Bool(false));
            Some(Call { noun: c.noun.clone(), params: JsonValue::Object(n) })
        } else {
            None
        }
    };
    if let Some(u) = flip("on").or_else(|| flip("mute")) {
        return Some(u);
    }
    // a drive is put back by turning it to 0, where the FX unit does nothing (`for 4 bars: master: drive 0.6`)
    if c.noun == "fx" && o.get("action").and_then(JsonValue::as_str) == Some("set") && o.get("param").and_then(JsonValue::as_str) == Some("drive") {
        let mut n = o.clone();
        n.set("value", JsonValue::Number(0.0));
        return Some(Call { noun: c.noun.clone(), params: JsonValue::Object(n) });
    }
    let verb = o.get("action").and_then(JsonValue::as_str)?;
    let back = match verb {
        "on" => "off",
        "start" => "stop",
        "arm" => "cancel",
        "enable" => "disable",
        "play" => "stop",
        _ => return None,
    };
    let mut n = o.clone();
    n.set("action", JsonValue::str(back));
    Some(Call { noun: c.noun.clone(), params: JsonValue::Object(n) })
}

/// The calls that put an effect back, last first; empty when it cannot be undone.
pub fn derive_undo(e: &Effect) -> Vec<Call> {
    let calls: Vec<&Call> = match e {
        Effect::Calls(cs) => cs.iter().collect(),
        Effect::Ramp(r) => r.prelude.iter().collect(),
        // a duck and a mix carry their own way back (mixmoves.rs)
        Effect::Duck(_) | Effect::Mix(_) => Vec::new(),
        Effect::Lfo(l) => l.prelude.iter().collect(),
        Effect::Performed(p) => return derive_undo(&p.base),
    };
    calls.into_iter().rev().filter_map(undo_of).collect()
}
