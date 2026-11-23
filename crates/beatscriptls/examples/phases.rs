//! Where the time of one check goes, on one file: lex, parse, the whole check.
use std::time::Instant;
fn main() {
    let f = std::env::args().nth(1).expect("a file");
    let text = std::fs::read_to_string(&f).unwrap();
    let env = beatscript_api::web_env().env();
    let t = Instant::now();
    let _ = djlive::lex::lex(&text);
    println!("lex    {:>9.2} ms", t.elapsed().as_secs_f64() * 1e3);
    let t = Instant::now();
    let _ = djlive::parse::parse_with(&text, &env);
    println!("parse  {:>9.2} ms", t.elapsed().as_secs_f64() * 1e3);
    let t = Instant::now();
    let _ = djlive::compile::check_with(&text, &env);
    println!("check  {:>9.2} ms", t.elapsed().as_secs_f64() * 1e3);
}
