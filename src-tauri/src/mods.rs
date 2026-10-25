//! Carbon / Oxide and the RustEdit extension.
//!
//! Both release archives are unpacked straight into the server folder: Carbon adds `carbon/`
//! plus a doorstop loader (`winhttp.dll`), Oxide replaces files in `RustDedicated_Data/Managed`
//! (incl. Assembly-CSharp.dll, so a server update or verify removes it again).

use crate::t;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::archive;
use crate::steamcmd::CancelToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Framework {
    None,
    Carbon,
    Oxide,
}

impl Framework {
    pub fn label(self) -> String {
        match self {
            Framework::None => t!("framework.noneLabel"),
            Framework::Carbon => t!("framework.carbon"),
            Framework::Oxide => t!("framework.oxide"),
        }
    }
}

// Always from the makers' own releases, when the user picks them; never bundled into DropLaunch or
// mirrored elsewhere: Carbon is GPL-3.0, and the RustEdit extension has no license (all rights
// reserved by its author).
#[cfg(windows)]
const CARBON_URL: &str = "https://github.com/CarbonCommunity/Carbon/releases/download/production_build/Carbon.Windows.Release.zip";
#[cfg(not(windows))]
const CARBON_URL: &str = "https://github.com/CarbonCommunity/Carbon/releases/download/production_build/Carbon.Linux.Release.tar.gz";
#[cfg(windows)]
const OXIDE_URL: &str = "https://github.com/OxideMod/Oxide.Rust/releases/latest/download/Oxide.Rust.zip";
#[cfg(not(windows))]
const OXIDE_URL: &str = "https://github.com/OxideMod/Oxide.Rust/releases/latest/download/Oxide.Rust-linux.zip";
const RUSTEDIT_URL: &str = "https://raw.githubusercontent.com/k1lly0u/Oxide.Ext.RustEdit/master/Oxide.Ext.RustEdit.dll";
const RUSTEDIT_DLL: &str = "Oxide.Ext.RustEdit.dll";

fn managed_dir(server_dir: &Path) -> PathBuf {
    server_dir.join("RustDedicated_Data").join("Managed")
}

pub fn detect(server_dir: &Path) -> Framework {
    if server_dir.join("carbon").join("managed").join("Carbon.dll").is_file() {
        Framework::Carbon
    } else if managed_dir(server_dir).join("Oxide.Rust.dll").is_file() {
        Framework::Oxide
    } else {
        Framework::None
    }
}

/// Where the RustEdit extension goes for each framework.
fn rustedit_path(server_dir: &Path, framework: Framework) -> Option<PathBuf> {
    match framework {
        Framework::Carbon => Some(server_dir.join("carbon").join("extensions").join(RUSTEDIT_DLL)),
        Framework::Oxide => Some(managed_dir(server_dir).join(RUSTEDIT_DLL)),
        Framework::None => None,
    }
}

pub fn has_rustedit(server_dir: &Path) -> bool {
    rustedit_path(server_dir, detect(server_dir)).is_some_and(|p| p.is_file())
}

/// Oxide's own DLLs survive a server update, but the update puts Facepunch's original
/// Assembly-CSharp.dll back, and only Oxide's patched one references Oxide.Core.
pub fn oxide_needs_reapply(server_dir: &Path) -> bool {
    detect(server_dir) == Framework::Oxide && !oxide_patched(server_dir)
}

fn oxide_patched(server_dir: &Path) -> bool {
    use std::sync::Mutex;
    use std::time::SystemTime;
    // The check reads a ~10 MB file, so remember the answer until the file changes.
    static CACHE: Mutex<Option<(PathBuf, u64, Option<SystemTime>, bool)>> = Mutex::new(None);
    let file = managed_dir(server_dir).join("Assembly-CSharp.dll");
    let Ok(meta) = std::fs::metadata(&file) else { return false };
    let key = (file.clone(), meta.len(), meta.modified().ok());
    let mut cache = CACHE.lock().unwrap();
    if let Some((path, len, modified, patched)) = cache.as_ref() {
        if (path, *len, *modified) == (&key.0, key.1, key.2) {
            return *patched;
        }
    }
    let patched = std::fs::read(&file).is_ok_and(|bytes| bytes.windows(OXIDE_MARKER.len()).any(|w| w == OXIDE_MARKER));
    *cache = Some((key.0, key.1, key.2, patched));
    patched
}

const OXIDE_MARKER: &[u8] = b"Oxide.Core";

/// Download the framework's latest release and unpack it into the server folder.
pub fn install(
    framework: Framework,
    server_dir: &Path,
    cancel: &CancelToken,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<()> {
    let url = match framework {
        Framework::Carbon => CARBON_URL,
        Framework::Oxide => OXIDE_URL,
        Framework::None => return Ok(()),
    };
    let current = detect(server_dir);
    if current != Framework::None && current != framework {
        bail!(t!("mods.error.switch", framework = current.label()));
    }
    let file = server_dir.join(url.rsplit('/').next().unwrap_or("framework.zip"));
    archive::download(url, &file, cancel, progress)?;
    let unpacked = archive::extract(&file, server_dir);
    let _ = std::fs::remove_file(&file);
    unpacked?;
    if detect(server_dir) != framework {
        bail!(t!("mods.error.missing", framework = framework.label()));
    }
    Ok(())
}

/// Add Oxide.Ext.RustEdit.dll (custom monuments, IO, NPC/APC paths from RustEdit maps).
pub fn install_rustedit(server_dir: &Path, cancel: &CancelToken, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<PathBuf> {
    let Some(dest) = rustedit_path(server_dir, detect(server_dir)) else {
        bail!(t!("mods.error.rusteditNeeds"));
    };
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    archive::download(RUSTEDIT_URL, &dest, cancel, progress)?;
    Ok(dest)
}

// ---------------------------------------------------------------- Carbon's profiler
// Carbon has a Mono profiler built in (carbon/config.profiler.json, read when the server
// starts): `c.profile` records what plugins, extensions (RustEdit's) and Rust's own code spend
// their time on, saved as .cprf files in carbon/profiles for Carbon's Profiler Panel.

const PROFILER_CONFIG: &str = "config.profiler.json";

fn profiler_config(server_dir: &Path) -> PathBuf {
    server_dir.join("carbon").join(PROFILER_CONFIG)
}

/// Where Carbon saves recorded profiles.
pub fn profiles_dir(server_dir: &Path) -> PathBuf {
    server_dir.join("carbon").join("profiles")
}

/// Carbon is installed and its profiler is switched on.
pub fn profiler_enabled(server_dir: &Path) -> bool {
    detect(server_dir) == Framework::Carbon
        && std::fs::read_to_string(profiler_config(server_dir))
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(text.trim_start_matches('\u{feff}')).ok())
            .is_some_and(|v| v["Enabled"].as_bool() == Some(true))
}

/// Switch Carbon's profiler on or off (used from the next server start). Switching it on also
/// picks what it watches when nothing is picked yet: Rust's own code, every plugin and extension.
/// False: no Carbon here.
pub fn set_profiler(server_dir: &Path, on: bool) -> Result<bool> {
    if detect(server_dir) != Framework::Carbon {
        return Ok(false);
    }
    let path = profiler_config(server_dir);
    let mut config = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(text.trim_start_matches('\u{feff}')).ok())
        .filter(serde_json::Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    if !on && !path.exists() {
        return Ok(true);
    }
    config["Enabled"] = serde_json::Value::Bool(on);
    if on {
        if config.get("TrackCalls").is_none() {
            config["TrackCalls"] = serde_json::Value::Bool(true);
        }
        for (key, default) in [("Assemblies", vec!["Assembly-CSharp"]), ("Plugins", vec!["*"]), ("Modules", vec![]), ("Extensions", vec!["*"]), ("Harmony", vec![])] {
            let empty = config.get(key).and_then(|v| v.as_array()).is_none_or(|a| a.is_empty());
            if empty {
                config[key] = serde_json::json!(default);
            }
        }
    }
    std::fs::write(&path, serde_json::to_string_pretty(&config)?)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A server update brings Carbon to its newest version by installing it over the old one:
    /// plugins, their configs and the profiler's settings stay. Downloads Carbon (twice).
    #[test]
    #[ignore]
    fn carbon_updates_over_itself_and_keeps_plugins() {
        let dir = std::env::temp_dir().join(format!("droplaunch-carbon-update-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cancel = CancelToken::default();
        install(Framework::Carbon, &dir, &cancel, &mut |_, _| {}).unwrap();
        assert_eq!(detect(&dir), Framework::Carbon);
        let plugin = dir.join("carbon").join("plugins").join("MyPlugin.cs");
        let config = dir.join("carbon").join("configs").join("MyPlugin.json");
        std::fs::write(&plugin, "// mine").unwrap();
        std::fs::write(&config, r#"{"mine": true}"#).unwrap();
        set_profiler(&dir, true).unwrap();
        install(Framework::Carbon, &dir, &cancel, &mut |_, _| {}).unwrap();
        assert_eq!(detect(&dir), Framework::Carbon);
        assert_eq!(std::fs::read_to_string(&plugin).unwrap(), "// mine");
        assert_eq!(std::fs::read_to_string(&config).unwrap(), r#"{"mine": true}"#);
        assert!(profiler_enabled(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_profiler_switch_keeps_other_settings() {
        let dir = std::env::temp_dir().join(format!("droplaunch-profiler-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("carbon").join("managed")).unwrap();
        assert!(!set_profiler(&dir, true).unwrap(), "no Carbon yet");
        std::fs::write(dir.join("carbon").join("managed").join("Carbon.dll"), b"x").unwrap();
        std::fs::write(profiler_config(&dir), r#"{"Enabled": false, "Plugins": ["MyPlugin"], "SourceViewer": true}"#).unwrap();
        assert!(set_profiler(&dir, true).unwrap());
        assert!(profiler_enabled(&dir));
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(profiler_config(&dir)).unwrap()).unwrap();
        assert_eq!(v["Plugins"], serde_json::json!(["MyPlugin"]), "a choice made before stays");
        assert_eq!(v["Assemblies"], serde_json::json!(["Assembly-CSharp"]));
        assert_eq!(v["SourceViewer"], serde_json::json!(true));
        set_profiler(&dir, false).unwrap();
        assert!(!profiler_enabled(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
