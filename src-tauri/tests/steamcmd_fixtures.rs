//! Replays SteamCMD output captured on 2026-10-07 through the real parsing code.
//! *_pty.json hold the ConPTY chunks exactly as they arrived (with timestamps);
//! info_pipe.txt is the same app_info_print query captured through a plain pipe.

use droplaunch_lib::steamcmd::parser::Signal;
use droplaunch_lib::steamcmd::runner::{Output, StreamParser};
use droplaunch_lib::steamcmd::vt::LineAssembler;
use droplaunch_lib::steamcmd::{appinfo, manifest};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

/// The PTY chunks of a capture, in arrival order.
fn pty_chunks(name: &str) -> Vec<String> {
    let json: serde_json::Value = serde_json::from_slice(&fixture(name)).unwrap();
    json["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["src"] == "PTY")
        .map(|e| e["raw"].as_str().unwrap().to_string())
        .collect()
}

fn signals(chunks: &[String]) -> Vec<(Signal, bool)> {
    let mut out = Vec::new();
    let mut parser = StreamParser::new();
    let mut on = |o: Output| {
        if let Output::Signal { signal, partial } = o {
            out.push((signal, partial));
        }
    };
    for chunk in chunks {
        parser.push(chunk.as_bytes(), &mut on);
    }
    parser.finish(&mut on);
    out
}

fn assemble(raw: &[u8], chunk: usize) -> Vec<String> {
    let mut vt = LineAssembler::new(50);
    let mut lines = Vec::new();
    for part in raw.chunks(chunk) {
        lines.extend(vt.feed(part));
    }
    lines.extend(vt.finish());
    lines
}

#[test]
fn burst_output_rebuilds_the_same_lines_as_a_pipe() {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    // Pipe output arrives out of order (bootstrap lines last), so compare as sorted sets.
    // The two captures were taken a few seconds apart; only the "change number" line differs.
    let keep = |l: &String| !l.is_empty() && !l.starts_with("AppID : 258550");
    let mut pty: Vec<String> = assemble(&fixture("info_pty.raw"), usize::MAX).iter().map(|l| norm(l)).filter(keep).collect();
    let text = String::from_utf8(fixture("info_pipe.txt")).unwrap();
    let mut pipe: Vec<String> = text.lines().map(norm).filter(keep).collect();
    pty.sort();
    pipe.sort();
    assert_eq!(pty.len(), 223);
    assert_eq!(pty, pipe);
}

#[test]
fn chunk_boundaries_do_not_change_the_result() {
    let raw = fixture("info_pty.raw");
    let whole = assemble(&raw, usize::MAX);
    for size in [1, 2, 3, 7, 64, 4096] {
        assert_eq!(assemble(&raw, size), whole, "chunk size {size}");
    }
}

#[test]
fn validate_run_fires_every_stage_in_order() {
    let got = signals(&pty_chunks("validate_pty.json"));
    let names: Vec<String> = got
        .iter()
        .map(|(s, partial)| {
            let name = match s {
                Signal::SelfUpdate { text, .. } => format!("self:{text}"),
                Signal::Progress { label, percent, .. } => format!("progress:{label}:{percent}"),
                other => format!("{other:?}"),
            };
            if *partial { format!("{name} (partial)") } else { name }
        })
        .collect();
    assert_eq!(
        names,
        [
            "self:Checking for available updates...",
            "self:Verifying installation...",
            "SteamApi",
            "Connecting (partial)",
            "Connected",
            "LoggedIn",
            "progress:verifying install:30.93",
            "progress:verifying install:66.3",
            "Success { up_to_date: false }",
        ]
    );
}

#[test]
fn up_to_date_run_is_recognised() {
    let got = signals(&pty_chunks("uptodate_pty.json"));
    assert_eq!(got.last().map(|(s, _)| s.clone()), Some(Signal::Success { up_to_date: true }));
}

#[test]
fn branches_from_app_info() {
    let text = String::from_utf8(fixture("info_pipe.txt")).unwrap();
    let branches = appinfo::parse_branches(&text).unwrap();
    let names: Vec<&str> = branches.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names[0], "public");
    for expected in ["staging", "release", "aux02", "last-month", "debug"] {
        assert!(names.contains(&expected), "{expected} missing from {names:?}");
    }
    let public = &branches[0];
    assert_eq!(public.buildid, "25681086");
}

#[test]
fn installed_manifest() {
    let text = String::from_utf8(fixture("appmanifest_258550.acf")).unwrap();
    let inst = manifest::parse_manifest(&text, true).unwrap();
    assert_eq!(inst.buildid, "25681086");
    assert_eq!(inst.branch, "public");
    assert!(inst.fully_installed);
    assert_eq!(inst.size_on_disk, 6103829589);
}

#[test]
fn branches_from_the_steamcmd_net_mirror() {
    let json: serde_json::Value = serde_json::from_slice(&fixture("mirror_258550.json")).unwrap();
    let branches = appinfo::branches_from_json(&json).unwrap();
    assert_eq!(branches[0].name, "public");
    assert_eq!(branches[0].buildid, "25681086");
    assert_eq!(branches.len(), 6);
}

#[test]
fn lagging_mirror_never_downgrades_a_build_steam_reported() {
    use droplaunch_lib::config::BranchCache;
    let branch = |name: &str, build: &str| appinfo::Branch {
        name: name.into(),
        buildid: build.into(),
        time_updated: None,
        description: String::new(),
    };
    let from_steam = BranchCache::merged(None, vec![branch("public", "100"), branch("staging", "205")], "Steam");
    let from_mirror = BranchCache::merged(Some(from_steam), vec![branch("public", "101"), branch("staging", "200")], "steamcmd.net");
    let build = |name: &str| from_mirror.branches.iter().find(|b| b.name == name).unwrap().buildid.clone();
    assert_eq!(build("public"), "101"); // newer from the mirror: taken
    assert_eq!(build("staging"), "205"); // mirror is behind: Steam's build kept
}
