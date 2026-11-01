//! The third-party notices DropLaunch shows (About) cover what it's built with right now, each
//! library with the text of its license.

use serde_json::Value;

fn notices() -> Value {
    serde_json::from_str(include_str!("../../ui/licenses.json")).expect("ui/licenses.json")
}

/// Cargo.lock without DropLaunch's own version: a new release doesn't change the libraries.
/// (The same in examples/notices.rs.)
fn lock_hash(lock: &str) -> String {
    let lock = lock.replace("\r\n", "\n");
    let mut lines: Vec<&str> = lock.split('\n').collect();
    if let Some(i) = lines.iter().position(|l| *l == "name = \"droplaunch\"") {
        if lines.get(i + 1).is_some_and(|l| l.starts_with("version = ")) {
            lines.remove(i + 1);
        }
    }
    blake3::hash(lines.join("\n").as_bytes()).to_hex().to_string()
}

#[test]
fn third_party_notices_are_up_to_date() {
    assert_eq!(
        notices()["cargoLock"].as_str(),
        Some(lock_hash(include_str!("../Cargo.lock")).as_str()),
        "the dependencies changed since ui/licenses.json was made: run `cargo run --example notices`"
    );
}

#[test]
fn a_new_version_of_droplaunch_alone_needs_no_new_notices() {
    let lock = include_str!("../Cargo.lock").replace("\r\n", "\n");
    let own = format!("name = \"droplaunch\"\nversion = \"{}\"", env!("CARGO_PKG_VERSION"));
    let bumped = lock.replace(&own, "name = \"droplaunch\"\nversion = \"99.0.0\"");
    assert_ne!(bumped, lock, "DropLaunch's own entry wasn't found in Cargo.lock");
    assert_eq!(lock_hash(&bumped), lock_hash(&lock));
}

/// Words that show a file is the text of a license (by its SPDX id).
fn marks(id: &str) -> Option<&'static [&'static str]> {
    Some(match id {
        "MIT" | "MIT-0" => &["permission is hereby granted"],
        "Apache-2.0" => &["apache license"],
        "BSD-2-Clause" | "BSD-3-Clause" => &["redistribution and use"],
        "0BSD" | "ISC" => &["permission to use, copy, modify"],
        "MPL-2.0" => &["mozilla public license"],
        "Unlicense" => &["free and unencumbered"],
        "Zlib" => &["provided 'as-is'", "provided \"as-is\""],
        "CC0-1.0" => &["cc0", "creative commons"],
        "Unicode-3.0" => &["unicode"],
        "BSL-1.0" => &["boost software license"],
        "CDLA-Permissive-2.0" => &["community data license agreement"],
        _ => return None,
    })
}

#[test]
fn every_library_comes_with_its_license_text() {
    let notices = notices();
    let mut problems = Vec::new();
    for c in notices["crates"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap_or("?");
        let license = c["license"].as_str().unwrap_or("");
        let text: String = c["files"].as_array().unwrap().iter().filter_map(|f| notices["texts"][f["text"].as_str()?].as_str()).collect::<Vec<_>>().join("\n").to_lowercase();
        // "A AND B" needs both, "A OR B" either ("Apache-2.0/MIT" is an old way to write OR).
        let groups = license.replace(['(', ')'], "").replace('/', " OR ");
        for group in groups.split(" AND ") {
            let options: Vec<&str> = group.split(" OR ").map(|o| o.split(" WITH ").next().unwrap_or(o).trim()).filter(|o| !o.is_empty()).collect();
            let known: Vec<&[&str]> = options.iter().filter_map(|o| marks(o)).collect();
            if known.is_empty() {
                problems.push(format!("{name}: unknown license {group} (add it to marks())"));
            } else if !known.iter().any(|words| words.iter().any(|w| text.contains(w))) {
                problems.push(format!("{name} ({license}): no {group} text among its files (add it to notices-extra/{name}/)"));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
