//! Settings, kept in the user's app data (`%LOCALAPPDATA%\DropLaunch\settings.json`) so the
//! DropLaunch folder holds nothing but the exe and what it downloads. Each folder DropLaunch runs
//! from has its own entry there (paths inside it are stored relative to it); a DropLaunch that
//! runs from a folder it never ran from starts like a new DropLaunch: default settings, the
//! folders it finds next to itself, nothing from another folder (an exe sent to someone or
//! copied elsewhere doesn't bring old settings along). Older versions kept `droplaunch.json` next to the exe: it's moved
//! in once and removed.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::owner::SteamUser;
use crate::steamcmd::appinfo::{build_number, Branch};
use crate::steamcmd::manifest::InstalledServer;
use crate::steamcmd::{install, manifest};

/// The settings file in the app data folder.
pub const SETTINGS_FILE: &str = "settings.json";
/// Older versions: the settings next to the exe.
pub const LEGACY_FILE: &str = "droplaunch.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    /// Stored relative to the DropLaunch folder when inside it, absolute otherwise.
    pub steamcmd_dir: String,
    /// The server folder: it holds the branch that was installed first.
    pub server_dir: String,
    pub maps_dir: String,
    /// The branch servers start with (each installed branch has its own folder).
    pub branch: String,
    /// Branches installed next to the server folder's own: branch -> folder (stored like
    /// server_dir, normally `Server_Files_<branch>`).
    pub branch_dirs: BTreeMap<String, String>,
    pub branch_cache: Option<BranchCache>,
    /// Open Rust and join (steam://connect) as soon as a started server is ready.
    pub auto_join: bool,
    /// Show a notice when a map in the maps folder is saved (the list follows it either way).
    pub notify_saves: bool,
    /// Windows notifications (map saved, server ready) while DropLaunch is in the background.
    pub desktop_notifications: bool,
    /// Before a map starts, remove the Rust game's cached copies of it (like v1's cleanup).
    pub clean_client_maps: bool,
    /// The playtest kit: a small plugin in servers with Carbon or Oxide, so testers can send bug
    /// reports with /bug in Rust's chat. Only when chosen (setup, the Mods card or its page).
    pub playtest: bool,
    /// Look for a new DropLaunch on GitHub when it starts.
    pub update_check: bool,
    /// A server update also brings Carbon (or Oxide) to its newest version.
    pub update_framework: bool,
    /// "Don't show this again" on the new version's window (that version; the top bar's button stays).
    pub skipped_update: Option<String>,
    /// Made the owner (admin) of every DropLaunch server.
    pub owner: Option<SteamUser>,
    /// Only the owner and `whitelist_ids` can join DropLaunch's servers (the DropLaunch plugin
    /// checks each login, so it needs Carbon or Oxide).
    pub whitelist: bool,
    /// Players let in besides the owner (Steam64 IDs).
    pub whitelist_ids: Vec<String>,
    /// Maps removed from the Rust game's cache are moved to ClientMapBackups in the data folder
    /// instead of deleted.
    pub backup_client_maps: bool,
    /// Maps that were started, newest first.
    pub history: Vec<HistoryEntry>,
    /// The first start's welcome was answered (it's shown once, before setup).
    pub welcomed: bool,
    /// The folders are in use. False after Reset everything: like a fresh DropLaunch, nothing is
    /// linked (no server, no maps folder) until setup has run again, whatever is on the disk.
    pub linked: bool,
    /// Dropped assets older versions recorded without their folder are listed here (an entry
    /// from back then); a new folder's entry or a reset one doesn't list them.
    pub legacy_drops: bool,
    /// The language picked in Settings (its code). None: a language file next to the exe, else
    /// English.
    pub language: Option<String>,
    /// The guide for people new to hosting and map making.
    pub guide: Guide,
    /// Last time DropLaunch ran from this folder, seconds since 1970 (a moved or copied
    /// DropLaunch starts from the newest entry).
    pub last_used: i64,
    /// The folder this entry is for, as it's written (the entry's key is lowercase on Windows).
    pub folder: String,
    /// Older versions: maps started from outside the Maps folder (read once into `history`).
    #[serde(skip_serializing)]
    pub recent_maps: Vec<String>,
}

/// The settings file: one entry per folder DropLaunch ran from.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Settings {
    version: u32,
    /// Folder (lowercase on Windows) -> its Config, kept as JSON so other folders' entries are
    /// written back as they were.
    folders: BTreeMap<String, serde_json::Value>,
}

/// The guide: tips that explain a step when it comes up, and the Getting started checklist.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Guide {
    /// Shown (the first start's "Show me around", or Guide in the top bar).
    pub on: bool,
    /// Tips already read ("Got it"), by id.
    pub seen: Vec<String>,
    /// Checklist steps done, by id (they stay ticked).
    pub done: Vec<String>,
}

impl Guide {
    /// Ids for `seen` or `done`: short names, each once.
    pub fn add(list: &mut Vec<String>, ids: Vec<String>) {
        for id in ids {
            let ok = !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
            if ok && !list.contains(&id) && list.len() < 200 {
                list.push(id);
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub path: String,
    /// Last start, seconds since 1970 (0: unknown, from an older version).
    pub started: i64,
}

const HISTORY_LEN: usize = 25;

impl Default for Config {
    fn default() -> Self {
        // Same layout as v1's one-click setup, so an old DropLaunch folder just works.
        Self {
            steamcmd_dir: "SteamCMD".into(),
            server_dir: "Server_Files".into(),
            maps_dir: "Maps".into(),
            branch: "public".into(),
            branch_dirs: BTreeMap::new(),
            branch_cache: None,
            auto_join: false,
            notify_saves: true,
            desktop_notifications: true,
            clean_client_maps: true,
            playtest: false,
            update_check: true,
            update_framework: true,
            skipped_update: None,
            owner: None,
            whitelist: false,
            whitelist_ids: Vec::new(),
            backup_client_maps: true,
            history: Vec::new(),
            welcomed: false,
            linked: true,
            legacy_drops: true,
            language: None,
            guide: Guide::default(),
            last_used: 0,
            folder: String::new(),
            recent_maps: Vec::new(),
        }
    }
}

impl Config {
    /// Who the whitelist lets in: the owner first, then the players added. None: it's off (or
    /// there's no owner to let in, which would lock everyone out).
    pub fn whitelist_ids(&self) -> Option<Vec<String>> {
        let owner = self.owner.as_ref().filter(|_| self.whitelist)?;
        let mut ids = vec![owner.steam_id.clone()];
        for id in &self.whitelist_ids {
            if !ids.contains(id) {
                ids.push(id.clone());
            }
        }
        Some(ids)
    }

    /// The DropLaunch plugin goes into servers: for bug reports or for the whitelist.
    pub fn wants_plugin(&self) -> bool {
        self.playtest || self.whitelist_ids().is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchCache {
    pub checked_at: i64,
    /// "Steam" (SteamCMD) or "steamcmd.net" (mirror, may lag a little).
    #[serde(default)]
    pub source: String,
    pub branches: Vec<Branch>,
}

impl BranchCache {
    /// New list from `source`; a build Steam already told us about is never replaced by an
    /// older one from the lagging mirror.
    pub fn merged(previous: Option<BranchCache>, mut branches: Vec<Branch>, source: &str) -> Self {
        if let Some(prev) = previous {
            for branch in &mut branches {
                if let Some(old) = prev.branches.iter().find(|b| b.name == branch.name) {
                    if build_number(&old.buildid) > build_number(&branch.buildid) {
                        *branch = old.clone();
                    }
                }
            }
        }
        let checked_at = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        Self { checked_at, source: source.to_string(), branches }
    }
}

/// The settings couldn't be read (said once DropLaunch's language is known).
#[derive(Debug, Clone)]
pub enum LoadError {
    /// The settings file isn't JSON: it's kept as .json.bad and DropLaunch starts fresh.
    Corrupt { error: String, kept: PathBuf },
    /// This folder's entry (or the old droplaunch.json) couldn't be used.
    Entry { legacy: bool, error: String },
}

impl LoadError {
    pub fn describe(&self) -> String {
        match self {
            Self::Corrupt { error, kept } => crate::t!("config.error.corrupt", error, kept = kept.display()),
            Self::Entry { legacy: false, error } => crate::t!("config.error.entry", error),
            Self::Entry { legacy: true, error } => crate::t!("config.error.legacy", file = LEGACY_FILE, error),
        }
    }
}

pub struct ConfigStore {
    /// The folder DropLaunch runs from: relative paths start here.
    pub base: PathBuf,
    /// DropLaunch's app data folder (settings, map pictures, bug reports).
    pub data: PathBuf,
    /// The settings file.
    pub file: PathBuf,
    pub config: Config,
    pub load_error: Option<LoadError>,
    /// This folder's entry in the file.
    key: String,
    /// The other folders' entries, written back unchanged.
    others: BTreeMap<String, serde_json::Value>,
}

impl ConfigStore {
    pub fn load() -> Self {
        Self::load_from(base_dir(), data_dir())
    }

    fn load_from(base: PathBuf, data: PathBuf) -> Self {
        let file = data.join(SETTINGS_FILE);
        let key = folder_key(&base);
        let mut load_error = None;
        // A byte order mark (an editor may add one) isn't JSON.
        let read = |path: &Path| std::fs::read_to_string(path).map(|t| t.trim_start_matches('\u{feff}').to_string());
        let mut folders = match read(&file) {
            Ok(text) => match serde_json::from_str::<Settings>(&text) {
                Ok(settings) => settings.folders,
                Err(e) => {
                    // Kept for a look, not overwritten.
                    let bad = file.with_extension("json.bad");
                    let _ = std::fs::rename(&file, &bad);
                    load_error = Some(LoadError::Corrupt { error: e.to_string(), kept: bad });
                    BTreeMap::new()
                }
            },
            Err(_) => BTreeMap::new(),
        };

        let legacy = base.join(LEGACY_FILE);
        let legacy_text = read(&legacy).ok();
        let parse = |value: serde_json::Value, legacy: bool, load_error: &mut Option<LoadError>| match serde_json::from_value::<Config>(value) {
            Ok(config) => config,
            Err(e) => {
                *load_error = Some(LoadError::Entry { legacy, error: e.to_string() });
                detect_layout(&base)
            }
        };
        let mut config = match (folders.remove(&key), &legacy_text) {
            (Some(entry), _) => parse(entry, false, &mut load_error),
            (None, Some(text)) => match serde_json::from_str::<serde_json::Value>(text) {
                Ok(value) => parse(value, true, &mut load_error),
                Err(e) => {
                    load_error = Some(LoadError::Entry { legacy: true, error: e.to_string() });
                    detect_layout(&base)
                }
            },
            // A folder DropLaunch never ran from (new, moved or copied): a new DropLaunch, with only
            // what it finds next to itself. What older versions dropped isn't listed here either.
            (None, None) => Config { legacy_drops: false, ..detect_layout(&base) },
        };
        if config.history.is_empty() {
            let old = std::mem::take(&mut config.recent_maps);
            config.history = old.into_iter().map(|path| HistoryEntry { path, started: 0 }).collect();
        }
        config.last_used = now();
        config.folder = base.to_string_lossy().into_owned();
        // Folders that are gone don't need their settings any more (but a drive that isn't
        // plugged in right now, like a USB stick, comes back with its folders).
        folders.retain(|folder, _| still_there(Path::new(folder)));

        let store = Self { base, data, file, config, load_error, key, others: folders };
        if store.save().is_ok() && legacy_text.is_some() {
            let _ = std::fs::remove_file(&legacy);
            let _ = std::fs::remove_file(legacy.with_extension("json.tmp"));
        }
        store
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let mut folders = self.others.clone();
        folders.insert(self.key.clone(), serde_json::to_value(&self.config)?);
        std::fs::create_dir_all(&self.data)?;
        let tmp = self.file.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(&Settings { version: 1, folders })?)?;
        std::fs::rename(&tmp, &self.file)?;
        Ok(())
    }

    pub fn resolve(&self, stored: &str) -> PathBuf {
        let path = Path::new(stored);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.base.join(path)
        }
    }

    pub fn to_stored(&self, path: &Path) -> String {
        match path.strip_prefix(&self.base) {
            Ok(rel) if rel.as_os_str().is_empty() => ".".into(),
            Ok(rel) => rel.to_string_lossy().into_owned(),
            Err(_) => path.to_string_lossy().into_owned(),
        }
    }

    pub fn steamcmd_dir(&self) -> PathBuf {
        self.resolve(&self.config.steamcmd_dir)
    }

    /// The active branch's server folder (what servers start from).
    pub fn server_dir(&self) -> PathBuf {
        self.server_dir_for(&self.config.branch)
    }

    /// The server folder setup made (it holds the branch installed first).
    pub fn main_server_dir(&self) -> PathBuf {
        self.resolve(&self.config.server_dir)
    }

    /// Where a branch is, or goes: every branch has its own folder, so switching between them
    /// needs no download. The server folder holds the branch that went in first, others go next
    /// to it (`Server_Files_staging`).
    pub fn server_dir_for(&self, branch: &str) -> PathBuf {
        if let Some(dir) = self.config.branch_dirs.get(branch) {
            return self.resolve(dir);
        }
        let main = self.main_server_dir();
        match manifest::read_installed(&main) {
            None => main,
            Some(installed) if installed.branch == branch => main,
            Some(_) => self
                .installs()
                .into_iter()
                .find(|(_, i)| i.branch == branch)
                .map_or_else(|| branch_folder(&main, branch), |(dir, _)| dir),
        }
    }

    /// Every installed branch and its folder: the server folder, the recorded ones and folders
    /// next to it named like it. One folder per branch (the first one found).
    pub fn installs(&self) -> Vec<(PathBuf, InstalledServer)> {
        let main = self.main_server_dir();
        let mut dirs = vec![main.clone()];
        dirs.extend(self.config.branch_dirs.values().map(|d| self.resolve(d)));
        if let (Some(parent), Some(name)) = (main.parent(), main.file_name().and_then(|n| n.to_str())) {
            let prefix = format!("{name}_").to_lowercase();
            let mut beside: Vec<PathBuf> = std::fs::read_dir(parent)
                .map(|entries| {
                    entries
                        .flatten()
                        .map(|e| e.path())
                        .filter(|p| p.is_dir() && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.to_lowercase().starts_with(&prefix)))
                        .collect()
                })
                .unwrap_or_default();
            beside.sort();
            dirs.extend(beside);
        }
        let mut seen: Vec<PathBuf> = Vec::new();
        let mut out: Vec<(PathBuf, InstalledServer)> = Vec::new();
        for dir in dirs {
            if seen.iter().any(|d| same_dir(d, &dir)) {
                continue;
            }
            seen.push(dir.clone());
            if let Some(installed) = manifest::read_installed(&dir) {
                if !out.iter().any(|(_, i)| i.branch == installed.branch) {
                    out.push((dir, installed));
                }
            }
        }
        out
    }

    /// Remember where a branch was installed (the server folder needs no entry).
    pub fn remember_branch_dir(&mut self, branch: &str, dir: &Path) {
        if same_dir(dir, &self.main_server_dir()) {
            self.config.branch_dirs.remove(branch);
        } else {
            let stored = self.to_stored(dir);
            self.config.branch_dirs.insert(branch.to_string(), stored);
        }
    }

    /// Another folder for a branch's server files (`for_branch`: one being set up; None: the
    /// active one). A folder with a server in it decides the branch; with no branch given, that
    /// one becomes the active one.
    pub fn set_server_folder(&mut self, dir: &Path, for_branch: Option<&str>) {
        let installed = manifest::read_installed(dir).map(|i| i.branch);
        let branch = installed.clone().or_else(|| for_branch.map(String::from)).unwrap_or_else(|| self.config.branch.clone());
        if for_branch.is_none() {
            self.config.branch = branch.clone();
        }
        let main = self.main_server_dir();
        if same_dir(&self.server_dir_for(&branch), &main) && !self.config.branch_dirs.contains_key(&branch) {
            // The server folder itself moves: the other branches stay where they are (found next
            // to the old one until now, so they're written down first).
            for (other_dir, other) in self.installs() {
                if other.branch != branch && !same_dir(&other_dir, &main) {
                    let stored = self.to_stored(&other_dir);
                    self.config.branch_dirs.entry(other.branch).or_insert(stored);
                }
            }
            self.config.server_dir = self.to_stored(dir);
        } else {
            let stored = self.to_stored(dir);
            self.config.branch_dirs.insert(branch, stored);
        }
    }

    pub fn maps_dir(&self) -> PathBuf {
        self.resolve(&self.config.maps_dir)
    }

    /// Put a map that was just started at the top of the history.
    pub fn remember_map(&mut self, path: &Path) {
        let text = path.display().to_string();
        let started = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        self.config.history.retain(|h| !h.path.eq_ignore_ascii_case(&text));
        self.config.history.insert(0, HistoryEntry { path: text, started });
        self.config.history.truncate(HISTORY_LEN);
    }
}

/// First start without settings: find what is already there. Expected setups are a fresh
/// folder, or DropLaunch.exe next to (or inside) an existing SteamCMD folder.
fn detect_layout(base: &Path) -> Config {
    let mut config = Config::default();
    let relative = |path: &Path| match path.strip_prefix(base) {
        Ok(rel) if rel.as_os_str().is_empty() => ".".to_string(),
        Ok(rel) => rel.to_string_lossy().into_owned(),
        Err(_) => path.to_string_lossy().into_owned(),
    };

    if !install::is_installed(&base.join(&config.steamcmd_dir)) && install::is_installed(base) {
        config.steamcmd_dir = ".".into(); // the exe sits inside the SteamCMD folder
    }
    let steamcmd = base.join(&config.steamcmd_dir);

    if manifest::read_installed(&base.join(&config.server_dir)).is_none() {
        // SteamCMD's own default install folder, then any folder next to the exe holding a server.
        let mut candidates = vec![steamcmd.join("steamapps").join("common").join("rust_dedicated")];
        if let Ok(entries) = std::fs::read_dir(base) {
            candidates.extend(entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()));
        }
        let is_server = |dir: &Path| {
            dir.join("steamapps").join("appmanifest_258550.acf").is_file() || dir.join(manifest::SERVER_EXECUTABLE).is_file()
        };
        if let Some(found) = candidates.iter().find(|d| is_server(d.as_path())) {
            config.server_dir = relative(found);
        }
    }
    config
}

/// `Server_Files` -> `Server_Files_staging`.
fn branch_folder(main: &Path, branch: &str) -> PathBuf {
    let safe: String = branch.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '_' }).collect();
    match main.file_name() {
        Some(name) => main.with_file_name(format!("{}_{safe}", name.to_string_lossy())),
        None => main.join(format!("Server_Files_{safe}")),
    }
}

/// A settings entry's folder is there, or might be again: its drive (or network share) isn't
/// available right now.
fn still_there(folder: &Path) -> bool {
    folder.is_dir() || folder.ancestors().last().is_some_and(|root| !root.is_dir())
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let text = |p: &Path| p.to_string_lossy().trim_end_matches(['\\', '/']).to_string();
    if cfg!(windows) {
        text(a).eq_ignore_ascii_case(&text(b))
    } else {
        text(a) == text(b)
    }
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// A folder's entry in the settings file (Windows paths don't care about case).
fn folder_key(base: &Path) -> String {
    let text = base.to_string_lossy().trim_end_matches(['\\', '/']).to_string();
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    }
}

/// DropLaunch's app data: DROPLAUNCH_DATA, else DROPLAUNCH_HOME (a dev build keeps everything
/// in its own folder), else the user's app data folder.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("DROPLAUNCH_DATA").or_else(|| std::env::var_os("DROPLAUNCH_HOME")) {
        return PathBuf::from(dir);
    }
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    let dir = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("DropLaunch"))
    } else if cfg!(target_os = "macos") {
        home().map(|h| h.join("Library").join("Application Support").join("DropLaunch"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| home().map(|h| h.join(".config"))).map(|d| d.join("droplaunch"))
    };
    // Nowhere else to go: next to the exe, like older versions.
    dir.unwrap_or_else(base_dir)
}

/// DropLaunch's home: the exe's folder, or DROPLAUNCH_HOME (handy while developing).
fn base_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("DROPLAUNCH_HOME") {
        return PathBuf::from(home);
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::detect_layout;
    use std::fs;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("droplaunch-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: PathBuf) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"").unwrap();
    }

    #[test]
    fn minimal_settings_with_an_absolute_server_folder_parse() {
        let text = r#"{
  "steamcmdDir": "SteamCMD",
  "serverDir": "C:\\Users\\TP\\Desktop\\Droplaaauc\\Server_Files",
  "mapsDir": "Maps",
  "branch": "public"
}"#;
        let config: super::Config = serde_json::from_str(text).unwrap();
        assert_eq!(config.server_dir, r"C:\Users\TP\Desktop\Droplaaauc\Server_Files");
    }

    #[test]
    fn old_settings_next_to_the_exe_move_into_the_app_data() {
        let base = temp_dir("legacy");
        let data = temp_dir("legacy-data");
        // With a byte order mark, as some editors save it.
        fs::write(base.join(super::LEGACY_FILE), "\u{feff}{\"branch\":\"staging\",\"skippedUpdate\":\"2.0.1\"}").unwrap();
        let store = super::ConfigStore::load_from(base.clone(), data.clone());
        assert_eq!(store.config.branch, "staging");
        assert!(!base.join(super::LEGACY_FILE).exists(), "the old file is gone");
        assert!(data.join(super::SETTINGS_FILE).is_file());
        // Read back from the app data next time.
        let again = super::ConfigStore::load_from(base.clone(), data.clone());
        assert_eq!(again.config.skipped_update.as_deref(), Some("2.0.1"));
        let _ = fs::remove_dir_all(base);
        let _ = fs::remove_dir_all(data);
    }

    #[test]
    fn each_folder_keeps_its_own_settings_and_a_new_one_starts_fresh() {
        let one = temp_dir("folder-one");
        let two = temp_dir("folder-two");
        let data = temp_dir("folders-data");
        touch(one.join("SteamCMD").join(crate::steamcmd::install::EXECUTABLE));
        let mut first = super::ConfigStore::load_from(one.clone(), data.clone());
        first.config.branch = "staging".into();
        first.config.skipped_update = Some("2.0.1".into());
        first.save().unwrap();
        // A copy somewhere else: a new DropLaunch (nothing from the first folder, not its SteamCMD).
        let second = super::ConfigStore::load_from(two.clone(), data.clone());
        assert_eq!(second.config.branch, "public");
        assert_eq!(second.config.skipped_update, None);
        assert!(!second.config.legacy_drops);
        assert_eq!(second.config.steamcmd_dir, "SteamCMD");
        assert_eq!(second.steamcmd_dir(), two.join("SteamCMD"));
        assert_eq!(second.config.server_dir, "Server_Files");
        // Both are kept, and the first one still reads its own.
        let text = fs::read_to_string(data.join(super::SETTINGS_FILE)).unwrap();
        let settings: super::Settings = serde_json::from_str(&text).unwrap();
        assert_eq!(settings.folders.len(), 2);
        let back = super::ConfigStore::load_from(one.clone(), data.clone());
        assert_eq!(back.config.steamcmd_dir, "SteamCMD");
        for dir in [one, two, data] {
            let _ = fs::remove_dir_all(dir);
        }
    }

    fn fake_server(dir: &std::path::Path, branch: &str) {
        let beta = if branch == "public" { String::new() } else { format!("\"BetaKey\" \"{branch}\"") };
        let acf = format!(
            "\"AppState\"\n{{\n\"appid\" \"258550\"\n\"StateFlags\" \"4\"\n\"buildid\" \"123\"\n\"UserConfig\"\n{{\n{beta}\n}}\n}}\n"
        );
        fs::create_dir_all(dir.join("steamapps")).unwrap();
        fs::write(dir.join("steamapps").join("appmanifest_258550.acf"), acf).unwrap();
    }

    #[test]
    fn each_branch_gets_its_own_folder() {
        let base = temp_dir("branches");
        let data = temp_dir("branches-data");
        let mut store = super::ConfigStore::load_from(base.clone(), data.clone());
        // Nothing installed: the first branch goes into the server folder.
        assert_eq!(store.server_dir(), base.join("Server_Files"));
        fake_server(&base.join("Server_Files"), "public");
        // Another branch: next to it, and public stays where it is.
        store.config.branch = "staging".into();
        assert_eq!(store.server_dir(), base.join("Server_Files_staging"));
        assert_eq!(store.server_dir_for("public"), base.join("Server_Files"));
        fake_server(&base.join("Server_Files_staging"), "staging");
        let installs: Vec<String> = store.installs().into_iter().map(|(_, i)| i.branch).collect();
        assert_eq!(installs, ["public", "staging"]);
        // A folder picked for staging elsewhere is remembered for it.
        let elsewhere = base.join("Other");
        fake_server(&elsewhere, "staging");
        store.set_server_folder(&elsewhere, None);
        assert_eq!(store.config.branch, "staging");
        assert_eq!(store.server_dir(), elsewhere);
        assert_eq!(store.server_dir_for("public"), base.join("Server_Files"));
        // Public's folder moves somewhere else: staging stays where it is.
        store.config.branch_dirs.clear();
        let moved = base.join("Moved");
        fs::create_dir_all(&moved).unwrap();
        store.set_server_folder(&moved, Some("public"));
        assert_eq!(store.server_dir_for("public"), moved);
        assert_eq!(store.server_dir_for("staging"), base.join("Server_Files_staging"));
        let _ = fs::remove_dir_all(base);
        let _ = fs::remove_dir_all(data);
    }

    #[test]
    fn settings_of_gone_folders_go_unless_their_drive_is_away() {
        let here = temp_dir("still-there");
        assert!(super::still_there(&here));
        // The drive is there, the folder isn't: gone for good.
        assert!(!super::still_there(&here.join("removed")));
        let _ = fs::remove_dir_all(here);
    }

    #[test]
    fn fresh_folder_uses_v1_layout() {
        let base = temp_dir("fresh");
        let config = detect_layout(&base);
        assert_eq!((config.steamcmd_dir.as_str(), config.server_dir.as_str()), ("SteamCMD", "Server_Files"));
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn exe_next_to_steamcmd_folder_finds_a_server_in_another_folder() {
        let base = temp_dir("beside");
        touch(base.join("SteamCMD").join(crate::steamcmd::install::EXECUTABLE));
        touch(base.join("MyRustServer").join("steamapps").join("appmanifest_258550.acf"));
        let config = detect_layout(&base);
        assert_eq!((config.steamcmd_dir.as_str(), config.server_dir.as_str()), ("SteamCMD", "MyRustServer"));
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn exe_inside_steamcmd_folder_finds_steamcmds_default_server_folder() {
        let base = temp_dir("inside");
        touch(base.join(crate::steamcmd::install::EXECUTABLE));
        let server = base.join("steamapps").join("common").join("rust_dedicated");
        touch(server.join(crate::steamcmd::manifest::SERVER_EXECUTABLE));
        let config = detect_layout(&base);
        assert_eq!(config.steamcmd_dir, ".");
        assert_eq!(PathBuf::from(&config.server_dir), PathBuf::from("steamapps").join("common").join("rust_dedicated"));
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn guide_ids_are_kept_once_and_short() {
        let mut seen = vec!["setup-files".to_string()];
        super::Guide::add(&mut seen, vec!["setup-files".into(), "start".into(), "".into(), "no spaces".into(), "x".repeat(65)]);
        assert_eq!(seen, ["setup-files", "start"]);
        let old: super::Config = serde_json::from_str(r#"{"branch":"public"}"#).unwrap();
        assert_eq!(old.guide, super::Guide::default());
    }
}
