//! Run the SteamCMD jobs from a terminal and print every event with its arrival time.
//!
//!   cargo run --release --example steamcmd_live -- <steamcmd_dir> <server_dir> [branch] [--validate]
//!   cargo run --release --example steamcmd_live -- <steamcmd_dir> --branches

use droplaunch_lib::jobs::{self, JobEvent, UpdateRequest};
use droplaunch_lib::steamcmd::CancelToken;
use std::path::PathBuf;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let positional: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let Some(steamcmd_dir) = positional.first().map(PathBuf::from) else {
        eprintln!("usage: steamcmd_live <steamcmd_dir> <server_dir> [branch] [--validate] | <steamcmd_dir> --branches");
        std::process::exit(2);
    };

    let started = Instant::now();
    let mut print = |event: JobEvent| {
        let t = started.elapsed().as_secs_f64();
        match event {
            JobEvent::Line { text, source } => println!("{t:7.2}s  {source:?}: {text}"),
            other => println!("{t:7.2}s  >> {}", serde_json::to_string(&other).unwrap()),
        }
    };
    let cancel = CancelToken::default();

    if flag("--branches") {
        match jobs::fetch_branches(&steamcmd_dir, &cancel, &mut print) {
            Ok(list) => {
                for b in list {
                    println!("  {:<12} build {:>9}  updated {:?}", b.name, b.buildid, b.time_updated);
                }
            }
            Err(e) => println!("error: {e}"),
        }
        return;
    }

    let Some(server_dir) = positional.get(1).map(PathBuf::from) else {
        eprintln!("missing <server_dir>");
        std::process::exit(2);
    };
    let branch = positional.get(2).map(|s| s.to_string()).unwrap_or_else(|| "public".into());
    let base_dir = server_dir.parent().map(PathBuf::from).unwrap_or_else(|| server_dir.clone());
    let req = UpdateRequest { base_dir, steamcmd_dir, server_dir, branch, validate: flag("--validate"), update_framework: false };
    let summary = jobs::update_server(&req, &cancel, &mut print);
    println!("{}", serde_json::to_string_pretty(&summary).unwrap());
}
