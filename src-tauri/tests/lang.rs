//! The language file (ui/lang/en.json) and the code agree: every text the code asks for is there
//! (with the placeholders it fills in), and every text in the file is used somewhere.

use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn english() -> HashMap<String, String> {
    let text = std::fs::read_to_string(root().join("ui/lang/en.json")).unwrap();
    let file: Value = serde_json::from_str(&text).expect("en.json is JSON");
    assert_eq!(file["language"]["code"], "en");
    file["strings"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_str().expect("texts are strings").to_string())).collect()
}

fn files(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files(&path, ext, out);
        } else if path.extension().is_some_and(|e| e == ext) {
            out.push(path);
        }
    }
}

/// In a comment (an example in the docs, not a text the code asks for).
fn in_comment(text: &str, at: usize) -> bool {
    let line = &text[text[..at].rfind('\n').map_or(0, |i| i + 1)..at];
    let line = line.trim_start();
    line.starts_with("//") || line.starts_with("* ") || line.starts_with("/*") || line.contains(" // ")
}

fn placeholders(text: &str) -> BTreeSet<String> {
    Regex::new(r"\{(\w+)\}").unwrap().captures_iter(text).map(|c| c[1].to_string()).collect()
}

/// The arguments of a macro call that starts at `open` (just after its "("), split at top-level
/// commas (strings and brackets kept together).
fn call_args(text: &str, open: usize) -> Vec<String> {
    let (mut depth, mut in_str, mut escaped) = (0i32, false, false);
    let (mut args, mut current) = (Vec::new(), String::new());
    for c in text[open..].chars() {
        if in_str {
            current.push(c);
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => in_str = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                current.push(c);
            }
            '(' | '[' | '{' => {
                depth += 1;
                current.push(c);
            }
            ')' | ']' | '}' if depth == 0 => break,
            ')' | ']' | '}' => {
                depth -= 1;
                current.push(c);
            }
            ',' if depth == 0 => args.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        args.push(current);
    }
    args.into_iter().map(|a| a.trim().to_string()).collect()
}

#[test]
fn every_text_the_code_uses_is_in_the_language_file() {
    let english = english();
    let mut used: BTreeSet<String> = BTreeSet::new();
    let mut problems = Vec::new();
    let has = |key: &str| english.contains_key(key);
    let has_plural = |key: &str| english.contains_key(&format!("{key}.other"));

    // The backend: t!("key", name = value, other).
    let mut rust = Vec::new();
    files(&root().join("src-tauri/src"), "rs", &mut rust);
    let macro_call = Regex::new(r#"\bt!\(\s*"([^"]+)""#).unwrap();
    for path in &rust {
        let text = std::fs::read_to_string(path).unwrap();
        for m in macro_call.captures_iter(&text) {
            if in_comment(&text, m.get(0).unwrap().start()) {
                continue;
            }
            let key = m[1].to_string();
            let Some(template) = english.get(&key) else {
                problems.push(format!("{}: no text for \"{key}\"", path.display()));
                continue;
            };
            used.insert(key.clone());
            let open = m.get(0).unwrap().start() + "t!(".len();
            let given: BTreeSet<String> = call_args(&text, open)
                .iter()
                .skip(1)
                .map(|a| a.split('=').next().unwrap().trim().to_string())
                .collect();
            for name in placeholders(template) {
                if !given.contains(&name) {
                    problems.push(format!("{}: \"{key}\" needs {{{name}}}", path.display()));
                }
            }
        }
    }

    // The UI: t("key"), tn("key", n), rich("key"), richn(...), and keys kept in code ("tip.x.text").
    let mut js = Vec::new();
    files(&root().join("ui"), "js", &mut js);
    let groups: BTreeSet<&str> = english.keys().map(|k| k.split('.').next().unwrap()).collect();
    let call = Regex::new(r#"\b(t|tn|rich|richn)\(\s*"([^"]+)""#).unwrap();
    let literal = Regex::new(r#""([a-z]+(?:\.[A-Za-z0-9]+)+)""#).unwrap();
    for path in &js {
        let text = std::fs::read_to_string(path).unwrap();
        for c in call.captures_iter(&text) {
            if in_comment(&text, c.get(0).unwrap().start()) {
                continue;
            }
            let key = &c[2];
            let ok = if matches!(&c[1], "tn" | "richn") { has_plural(key) } else { has(key) };
            if !ok {
                problems.push(format!("{}: no text for \"{key}\"", path.display()));
            }
        }
        for c in literal.captures_iter(&text) {
            let key = &c[1];
            if !groups.contains(key.split('.').next().unwrap()) {
                continue;
            }
            if has(key) || has_plural(key) {
                used.insert(key.to_string());
            } else if call.is_match(&text[c.get(0).unwrap().start().saturating_sub(8)..c.get(0).unwrap().end()]) || key.contains('.') {
                problems.push(format!("{}: no text for \"{key}\"", path.display()));
            }
        }
    }

    // index.html: data-t, data-t-title, data-t-placeholder, data-t-label.
    let html = std::fs::read_to_string(root().join("ui/index.html")).unwrap();
    for c in Regex::new(r#"data-t(?:-title|-placeholder|-label)?="([^"]+)""#).unwrap().captures_iter(&html) {
        if has(&c[1]) {
            used.insert(c[1].to_string());
        } else {
            problems.push(format!("index.html: no text for \"{}\"", &c[1]));
        }
    }

    // Counts come as pairs: "x.one" needs "x.other".
    for key in english.keys().filter(|k| k.ends_with(".one")) {
        let base = key.trim_end_matches(".one");
        if !has(&format!("{base}.other")) {
            problems.push(format!("en.json: \"{key}\" has no \"{base}.other\""));
        }
    }

    // Every text is used (counts by their base key).
    let unused: Vec<&String> = english
        .keys()
        .filter(|k| !used.contains(*k))
        .filter(|k| !["one", "other", "two", "few", "many", "zero"].iter().any(|f| k.strip_suffix(&format!(".{f}")).is_some_and(|base| used.contains(base))))
        .collect();
    for key in unused {
        problems.push(format!("en.json: \"{key}\" isn't used"));
    }

    problems.sort();
    assert!(problems.is_empty(), "{} problem(s):\n{}", problems.len(), problems.join("\n"));
}
