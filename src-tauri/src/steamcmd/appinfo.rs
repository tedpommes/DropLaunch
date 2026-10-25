//! Branch list for the Rust dedicated server: straight from Steam (`app_info_print 258550`)
//! or, instantly, from the steamcmd.net mirror of that same data.

use crate::t;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

use super::vdf::{self, Vdf};
use super::RUST_SERVER_APP_ID;

/// Public mirror of Steam's app info. Answers in ~0.2 s without SteamCMD, but can lag Steam
/// by a few hours, so SteamCMD stays the authority ("Check for updates").
const MIRROR_URL: &str = "https://api.steamcmd.net/v1/info/258550";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Branch {
    pub name: String,
    pub buildid: String,
    pub time_updated: Option<i64>,
    pub description: String,
}

pub fn app_info_args() -> Vec<String> {
    [
        "+@ShutdownOnFailedCommand",
        "1",
        "+@NoPromptForPassword",
        "1",
        "+login",
        "anonymous",
        "+app_info_update",
        "1",
        "+app_info_print",
        RUST_SERVER_APP_ID,
        "+quit",
    ]
    .map(String::from)
    .to_vec()
}

/// Lines that belong to the KeyValues dump (kept out of the live log, it's 200+ lines).
pub fn is_dump_line(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with('"') || t.starts_with('{') || t.starts_with('}') || t.starts_with("AppID :")
}

/// Public (not password protected) branches, `public` first, then newest builds.
pub fn parse_branches(output: &str) -> Option<Vec<Branch>> {
    let start = output.find(&format!("\"{RUST_SERVER_APP_ID}\""))?;
    let root = vdf::parse(&output[start..]);
    let branches = root.get(RUST_SERVER_APP_ID)?.get("depots")?.get("branches")?;
    let mut list: Vec<Branch> = branches
        .entries()
        .iter()
        .filter_map(|(name, b)| {
            let get = |key: &str| b.get(key).and_then(Vdf::as_str);
            if get("pwdrequired") == Some("1") {
                return None;
            }
            Some(Branch {
                name: name.clone(),
                buildid: get("buildid")?.to_string(),
                time_updated: get("timeupdated").and_then(|t| t.parse().ok()),
                description: get("description").unwrap_or("").to_string(),
            })
        })
        .collect();
    sort_branches(&mut list);
    Some(list)
}

pub fn fetch_mirror() -> anyhow::Result<Vec<Branch>> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(8))
        .timeout_read(Duration::from_secs(15))
        .user_agent(concat!("DropLaunch/", env!("CARGO_PKG_VERSION")))
        .build();
    let body = agent.get(MIRROR_URL).call().context(t!("error.mirror.noAnswer"))?.into_string()?;
    let json: Value = serde_json::from_str(&body).context(t!("error.mirror.unexpected"))?;
    branches_from_json(&json).filter(|b| !b.is_empty()).context(t!("error.mirror.noList"))
}

pub fn branches_from_json(json: &Value) -> Option<Vec<Branch>> {
    let text = |v: &Value| v.as_str().map(str::to_string).or_else(|| v.as_u64().map(|n| n.to_string()));
    let branches = json.get("data")?.get(RUST_SERVER_APP_ID)?.get("depots")?.get("branches")?.as_object()?;
    let mut list: Vec<Branch> = branches
        .iter()
        .filter_map(|(name, b)| {
            if b.get("pwdrequired").and_then(text).as_deref() == Some("1") {
                return None;
            }
            Some(Branch {
                name: name.clone(),
                buildid: b.get("buildid").and_then(text)?,
                time_updated: b.get("timeupdated").and_then(text).and_then(|t| t.parse().ok()),
                description: b.get("description").and_then(text).unwrap_or_default(),
            })
        })
        .collect();
    sort_branches(&mut list);
    Some(list)
}

/// `public` first, then the most recently updated branches.
fn sort_branches(list: &mut [Branch]) {
    list.sort_by(|a, b| (a.name != "public").cmp(&(b.name != "public")).then(b.time_updated.cmp(&a.time_updated)));
}

/// Build ids only ever go up, so they compare as numbers.
pub fn build_number(buildid: &str) -> u64 {
    buildid.trim().parse().unwrap_or(0)
}
