//! Add-on pages must never get into DropLaunch's own page. Tauri counts every page of a registered
//! address (`http://addon.localhost` too) as the app's own, with every command, so what keeps an
//! add-on to the calls it may make is that its frame has no origin: every frame the window makes
//! is sandboxed without `allow-same-origin`, and nothing in the window lets a page take the top.
//! A frame made without that would hand an add-on every command; this fails first.

use regex::Regex;
use std::path::{Path, PathBuf};

fn ui() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("ui")
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files(&path, out);
        } else if path.extension().is_some_and(|e| e == "js" || e == "html") {
            out.push(path);
        }
    }
}

#[test]
fn every_frame_is_sandboxed() {
    let mut all = Vec::new();
    files(&ui(), &mut all);
    let frame = Regex::new(r#"el\(\s*"iframe"|createElement\(\s*["']iframe["']\)|<iframe\b"#).unwrap();
    let mut frames = 0;
    for path in &all {
        let text = std::fs::read_to_string(path).unwrap();
        let name = path.display();
        for found in frame.find_iter(&text) {
            frames += 1;
            // The attributes come right after (el("iframe", { ... })).
            let end = text[found.start()..].find("})").or_else(|| text[found.start()..].find('>')).map_or(text.len(), |i| found.start() + i);
            let attributes = &text[found.start()..end];
            assert!(attributes.contains(r#"sandbox: "allow-scripts allow-forms""#), "{name}: a frame without sandbox=\"allow-scripts allow-forms\":\n{attributes}");
        }
        for loose in ["allow-same-origin", "allow-top-navigation", "allow-popups", "allow-modals"] {
            assert!(!text.contains(loose), "{name} has {loose}: an add-on frame must not get it");
        }
    }
    assert!(frames >= 1, "the add-on frame wasn't found in ui/ (renamed? update this test)");
}
