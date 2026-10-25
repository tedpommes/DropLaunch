//! What is installed right now, read from Steam's own bookkeeping instead of console text.

use serde::Serialize;
use std::path::Path;

use super::vdf::{self, Vdf};
use super::RUST_SERVER_APP_ID;

#[cfg(windows)]
pub const SERVER_EXECUTABLE: &str = "RustDedicated.exe";
#[cfg(not(windows))]
pub const SERVER_EXECUTABLE: &str = "RustDedicated";

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InstalledServer {
    pub buildid: String,
    pub branch: String,
    pub state_flags: u32,
    /// StateFlags 4 = "Fully Installed" with no update or validation pending.
    pub fully_installed: bool,
    pub size_on_disk: u64,
    pub last_updated: Option<i64>,
    pub executable_found: bool,
}

/// Reads `<server>/steamapps/appmanifest_258550.acf`.
pub fn read_installed(server_dir: &Path) -> Option<InstalledServer> {
    let path = server_dir.join("steamapps").join(format!("appmanifest_{RUST_SERVER_APP_ID}.acf"));
    let text = std::fs::read_to_string(path).ok()?;
    parse_manifest(&text, server_dir.join(SERVER_EXECUTABLE).is_file())
}

pub fn parse_manifest(text: &str, executable_found: bool) -> Option<InstalledServer> {
    let root = vdf::parse(text);
    let app = root.get("AppState")?;
    let get = |key: &str| app.get(key).and_then(Vdf::as_str).unwrap_or("");
    // A beta shows up as BetaKey; the public branch leaves both sections empty.
    let beta = ["UserConfig", "MountedConfig"].iter().find_map(|section| {
        let key = app.get(section)?.get("BetaKey")?.as_str()?;
        (!key.is_empty()).then(|| key.to_string())
    });
    let state_flags = get("StateFlags").parse().unwrap_or(0);
    Some(InstalledServer {
        buildid: get("buildid").to_string(),
        branch: beta.unwrap_or_else(|| "public".to_string()),
        state_flags,
        fully_installed: state_flags == 4,
        size_on_disk: get("SizeOnDisk").parse().unwrap_or(0),
        last_updated: get("LastUpdated").parse().ok(),
        executable_found,
    })
}
