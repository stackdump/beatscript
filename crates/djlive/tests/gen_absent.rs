//! A host with no generator installed (djlive alone): a `gen` call is an error at the call that says so, the same words on every
//! such host, and the program still has its canonical text (gen.rs; btwapi installs the seeded generator, `crates/btwapi/tests/gen.rs`).

#[test]
fn a_host_without_the_generator_says_so_at_the_call() {
    let src = "let bed = gen techno seed=42 section=drop part=bass\ndeck A: bass bed\n";
    let c = djlive::check(src);
    let e: Vec<(String, String)> = c.diagnostics.iter().filter(|d| d.is_error()).map(|d| (d.span.text(src).to_string(), d.message.clone())).collect();
    assert!(e.iter().any(|(at, m)| at.starts_with("gen") && m.contains("gen@2 is not here")), "{e:?}");
    // the parse is the program as written, its version said
    let p = djlive::parse::parse(src);
    assert_eq!(djlive::print::canonical(&p.program), "let bed = gen@2 techno seed=42 section=drop part=bass\ndeck A: bass bed\n");
}
