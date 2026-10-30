//! Third-party notices: every library compiled into DropLaunch's Windows exe, with its license
//! and the license files it ships, written to `ui/licenses.json`. The app shows it under About
//! (the version tag in the top bar), so the single exe carries its own notices.
//!
//!     cargo run --example notices
//!
//! Run it after a dependency changes: the `third_party_notices_are_up_to_date` test fails until
//! it's done (the file records the Cargo.lock it was made from).

use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::process::Command;

/// What's shipped: the Windows build.
const TARGET: &str = "x86_64-pc-windows-msvc";

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--locked", "--filter-platform", TARGET])
        .current_dir(root)
        .output()
        .expect("run cargo metadata");
    assert!(out.status.success(), "cargo metadata failed: {}", String::from_utf8_lossy(&out.stderr));
    let meta: Value = serde_json::from_slice(&out.stdout).expect("cargo metadata's JSON");

    let packages: HashMap<&str, &Value> = meta["packages"].as_array().unwrap().iter().map(|p| (p["id"].as_str().unwrap(), p)).collect();
    let nodes: HashMap<&str, &Value> = meta["resolve"]["nodes"].as_array().unwrap().iter().map(|n| (n["id"].as_str().unwrap(), n)).collect();
    let me = meta["resolve"]["root"].as_str().expect("the droplaunch package");

    // Linked into the exe: normal dependencies, not build scripts' or proc macros' (those only
    // run while compiling).
    let is_proc_macro = |id: &str| packages[id]["targets"].as_array().unwrap().iter().any(|t| t["kind"].as_array().unwrap().iter().any(|k| k == "proc-macro"));
    let mut shipped: HashSet<&str> = HashSet::new();
    let mut todo = vec![me];
    while let Some(id) = todo.pop() {
        for dep in nodes[id]["deps"].as_array().unwrap() {
            let normal = dep["dep_kinds"].as_array().unwrap().iter().any(|k| k["kind"].is_null());
            let pkg = dep["pkg"].as_str().unwrap();
            if normal && !is_proc_macro(pkg) && shipped.insert(pkg) {
                todo.push(pkg);
            }
        }
    }
    // `cargo metadata` takes the features of every platform together (ureq's rustls is for Linux
    // only, Windows uses its own TLS): what's really built for Windows is what `cargo tree` lists.
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let tree = Command::new(cargo)
        .args(["tree", "--locked", "--target", TARGET, "-e", "normal", "--prefix", "none", "--format", "{p}"])
        .current_dir(root)
        .output()
        .expect("run cargo tree");
    assert!(tree.status.success(), "cargo tree failed: {}", String::from_utf8_lossy(&tree.stderr));
    let built: HashSet<String> =
        String::from_utf8_lossy(&tree.stdout).lines().map(|l| l.split_whitespace().take(2).collect::<Vec<_>>().join(" ")).collect();
    shipped.retain(|id| built.contains(&format!("{} v{}", packages[id]["name"].as_str().unwrap(), packages[id]["version"].as_str().unwrap())));

    // Each package's own files, plus `notices-extra/<name>/` for what a published package leaves
    // out (taken from the project's repository). With neither, a package from the same
    // repository (same project, same authors) lends its files.
    let own: HashMap<&str, Vec<(String, String)>> = shipped
        .iter()
        .map(|id| {
            let p = packages[id];
            let dir = Path::new(p["manifest_path"].as_str().unwrap()).parent().unwrap();
            (*id, license_files(dir, p["license_file"].as_str()))
        })
        .collect();
    let repo = |id: &str| packages[id]["repository"].as_str().map(|r| r.trim_end_matches('/').trim_end_matches(".git").to_ascii_lowercase());
    let files_of = |id: &str| -> Vec<(String, String)> {
        let mut files = own[id].clone();
        for (name, text) in license_files(&root.join("notices-extra").join(packages[id]["name"].as_str().unwrap()), None) {
            if !files.iter().any(|(n, _)| *n == name) {
                files.push((name, text));
            }
        }
        if !files.is_empty() {
            return files;
        }
        repo(id)
            .and_then(|r| shipped.iter().filter(|other| repo(other).as_deref() == Some(r.as_str()) && !own[*other].is_empty()).min())
            .map(|other| own[other].clone())
            .unwrap_or_default()
    };

    // License texts are kept once (many crates ship the same Apache-2.0 text).
    let mut texts: BTreeMap<String, String> = BTreeMap::new();
    let mut crates: Vec<Value> = shipped
        .iter()
        .map(|id| {
            let p = packages[id];
            let files: Vec<Value> = files_of(id)
                .into_iter()
                .map(|(name, text)| {
                    let key = blake3::hash(text.as_bytes()).to_hex()[..12].to_string();
                    texts.entry(key.clone()).or_insert(text);
                    json!({ "name": name, "text": key })
                })
                .collect();
            json!({
                "name": p["name"],
                "version": p["version"],
                "license": p["license"].as_str().unwrap_or("see its files"),
                "repository": p["repository"],
                "authors": p["authors"],
                "files": files,
            })
        })
        .collect();
    crates.sort_by_key(|c| (c["name"].as_str().unwrap().to_lowercase(), c["version"].as_str().unwrap().to_string()));

    let lock = std::fs::read_to_string(root.join("Cargo.lock")).expect("Cargo.lock").replace("\r\n", "\n");
    let notices = json!({
        "about": "Libraries compiled into DropLaunch (Windows build), made by `cargo run --example notices`.",
        "target": TARGET,
        "cargoLock": blake3::hash(lock.as_bytes()).to_hex().as_str(),
        "crates": crates,
        "texts": texts,
    });
    let path = root.join("..").join("ui").join("licenses.json");
    std::fs::write(&path, serde_json::to_string_pretty(&notices).unwrap() + "\n").expect("write ui/licenses.json");
    let without: Vec<&str> = crates.iter().filter(|c| c["files"].as_array().unwrap().is_empty()).map(|c| c["name"].as_str().unwrap()).collect();
    println!("{} libraries, {} distinct license texts -> {}", crates.len(), texts.len(), path.display());
    if !without.is_empty() {
        println!("without license files of their own ({}): {}", without.len(), without.join(", "));
    }
}

/// The license and notice files a package ships (`LICENSE-MIT`, `COPYING`, `LICENSES/x.txt`...).
fn license_files(dir: &Path, declared: Option<&str>) -> Vec<(String, String)> {
    let wanted = |name: &str| {
        let lower = name.to_ascii_lowercase();
        ["license", "licence", "copying", "notice", "copyright", "unlicense"].iter().any(|w| lower.starts_with(w))
    };
    let mut found: Vec<(String, String)> = Vec::new();
    let mut add = |name: String, path: &Path| {
        if found.iter().any(|(n, _)| *n == name) {
            return;
        }
        if let Ok(bytes) = std::fs::read(path) {
            let text = String::from_utf8_lossy(&bytes).replace("\r\n", "\n").trim_end().to_string();
            if !text.is_empty() {
                found.push((name, text));
            }
        }
    };
    if let Some(file) = declared {
        add(file.replace('\\', "/"), &dir.join(file));
    }
    let mut entries: Vec<_> = std::fs::read_dir(dir).map(|e| e.flatten().collect()).unwrap_or_default();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !wanted(&name) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            // REUSE style: LICENSES/MIT.txt
            let mut inner: Vec<_> = std::fs::read_dir(&path).map(|e| e.flatten().collect()).unwrap_or_default();
            inner.sort_by_key(|e| e.file_name());
            for file in inner.into_iter().filter(|e| e.path().is_file()) {
                add(format!("{name}/{}", file.file_name().to_string_lossy()), &file.path());
            }
        } else {
            add(name, &path);
        }
    }
    found
}
