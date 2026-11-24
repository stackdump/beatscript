//! What a default build may contain (scripts/check-public-build.sh, which this runs): code is MIT, programs and
//! content CC0; no shipped target links the booth's private dictionary (CMUdict, not in this repo) or the wallet, and
//! only what plays a program's `wheel` lines (the browser module, djweb, and the offline render, beatscript-cli and the server that
//! links it) links windwheel.

#[test]
fn a_public_build_cannot_include_the_booth_dictionary() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = std::process::Command::new("bash")
        .arg(root.join("scripts/check-public-build.sh"))
        .current_dir(&root)
        .env("CHECK_TARGET_DIR", root.join("target-guard"))
        .output()
        .expect("bash runs");
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{text}");
    assert!(text.contains("check-public-build: ok"), "{text}");
}
