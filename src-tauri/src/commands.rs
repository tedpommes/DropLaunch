//! Functions the UI calls through `invoke`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::client::{self, CacheInfo, Cleaned};
use crate::concmd;
use crate::config::{BranchCache, Config, ConfigStore, Guide};
use crate::assets;
use crate::kits;
use crate::jobs::{self, JobEvent, JobSummary, Launcher, SetupOptions, SetupRequest, UpdateRequest};
use crate::maps::{self, Imported, MapEntry};
use crate::mods::{self, Framework};
use crate::owner::{self, LocalAccount, SteamUser};
use crate::platform;
use crate::playtest::{self, mapview, MapNotes, Playtest, PlaytestView};
use crate::server::{ConsoleLine, ServerManager, ServerView, StartContext, StartReply};
use crate::startfile::{self, Layout, Target};
use crate::sync::Locked;
use crate::steamcmd::manifest::{self, InstalledServer};
use crate::steamcmd::{appinfo, install, CancelToken};
use crate::toast::{self, Toast};
use crate::updater::{self, Manifest, UpdateInfo};
use crate::watcher::MapWatcher;
use crate::whitelist;
use crate::{lang, t};

type CmdResult<T> = Result<T, String>;

/// Free space a server download needs: about 6 GB of files, and a little room on top.
const SERVER_SPACE: u64 = 6_979_321_856; // 6.5 GiB

pub struct AppState {
    store: Arc<Mutex<ConfigStore>>,
    job: Mutex<Option<Job>>,
    /// The running setup's options (the UI can change some while SteamCMD downloads).
    setup_options: Mutex<Option<Arc<Mutex<SetupOptions>>>>,
    pub server: ServerManager,
    /// The active maps folder, watched for new and saved maps.
    pub watcher: MapWatcher,
    /// Bug reports from Rust's chat (/bug) on a map of the server.
    pub playtest: Playtest,
    /// Files DropLaunch was opened with (dropped on the exe), until the UI asks for them.
    pending_files: Mutex<Vec<String>>,
    /// The newer DropLaunch the last update check found.
    update: Mutex<Option<Manifest>>,
}

impl AppState {
    pub fn load(files: Vec<String>) -> Self {
        let mut config = ConfigStore::load();
        crate::log::init(&config.data);
        crate::log::write(&format!("DropLaunch {} started", env!("CARGO_PKG_VERSION")));
        // Language files the exe was opened with (dropped on it): added, the last one is used.
        let (languages, files): (Vec<String>, Vec<String>) = files.into_iter().partition(|f| lang::is_language_file(Path::new(f)));
        for file in &languages {
            if let Ok(language) = lang::import(Path::new(file), &config.data) {
                assets::record(&config.data, &config.config.folder, assets::Kind::Language, Path::new(file), language.file.iter().map(|f| f.display().to_string()).collect());
                config.config.language = Some(language.code);
            }
        }
        if !languages.is_empty() {
            let _ = config.save();
        }
        let available = lang::list(&config.base, &config.data);
        let _ = lang::activate(lang::choose(config.config.language.as_deref(), &available));
        let store = Arc::new(Mutex::new(config));
        let server = ServerManager::new(store.clone());
        let playtest = Playtest::new(store.clone(), server.clone());
        Self {
            store,
            job: Mutex::new(None),
            setup_options: Mutex::new(None),
            server,
            watcher: MapWatcher::default(),
            playtest,
            pending_files: Mutex::new(files),
            update: Mutex::new(None),
        }
    }

    pub fn maps_dir(&self) -> PathBuf {
        self.store.locked().maps_dir()
    }

    /// Claim the single SteamCMD slot (two SteamCMDs in one folder corrupt each other). `dir`:
    /// the server folder the job changes (None: none, like reading the branch list).
    fn start_job(&self, dir: Option<PathBuf>) -> CmdResult<(CancelToken, JobSlot<'_>)> {
        let mut job = self.job.locked();
        if job.is_some() {
            return Err(t!("error.busy"));
        }
        let token = CancelToken::default();
        *job = Some(Job { token: token.clone(), dir });
        Ok((token, JobSlot(&self.job)))
    }

    fn data_dir(&self) -> PathBuf {
        self.store.locked().data.clone()
    }

    fn ensure_idle(&self) -> CmdResult<()> {
        if self.job.locked().is_some() {
            Err(t!("error.busy"))
        } else {
            Ok(())
        }
    }

    /// SteamCMD isn't changing the server files in `dir` (another branch's folder can be busy:
    /// a server can start from this one meanwhile).
    fn ensure_free(&self, dir: &Path) -> CmdResult<()> {
        match self.job.locked().as_ref() {
            Some(Job { dir: Some(busy), .. }) if startfile::same_path(busy, dir) => Err(t!("error.busyHere")),
            _ => Ok(()),
        }
    }

    fn start_context(&self) -> StartContext {
        StartContext::from_store(&self.store.locked())
    }

    /// Existing installs get update_server.bat too (it's written by setup and updates).
    pub fn ensure_scripts(&self) {
        let store = self.store.locked();
        let steamcmd_dir = store.steamcmd_dir();
        if !install::is_installed(&steamcmd_dir) {
            return;
        }
        for (server_dir, server) in store.installs() {
            if !server_dir.join(startfile::UPDATE_SCRIPT).is_file() {
                let layout = Layout { base: &store.base, server_dir: &server_dir, steamcmd_dir: &steamcmd_dir };
                let _ = startfile::write_update_script(&layout, &server.branch);
            }
        }
    }
}

/// The SteamCMD job that runs.
struct Job {
    token: CancelToken,
    /// The server folder it changes (None: none).
    dir: Option<PathBuf>,
}

/// Frees the job slot when the command ends, however it ends.
struct JobSlot<'a>(&'a Mutex<Option<Job>>);

impl Drop for JobSlot<'_> {
    fn drop(&mut self) {
        *self.0.locked() = None;
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    key: &'static str,
    label: String,
    path: String,
    exists: bool,
    /// There's nothing in it (yet): an empty folder counts as one it makes, on the welcome screen.
    empty: bool,
}

/// An installed branch (each has its own folder).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallInfo {
    branch: String,
    path: String,
    buildid: String,
    fully_installed: bool,
    last_updated: Option<i64>,
    /// Servers start from this one.
    active: bool,
}

/// A DropLaunch start file next to the server.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartFileInfo {
    path: String,
    /// The map's name; None for the procedural map.
    map: Option<String>,
    /// The map's prefab test mode file.
    prefab: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    version: &'static str,
    platform: &'static str,
    can_host: bool,
    base_dir: String,
    /// DropLaunch is in a drive's root or a busy folder (platform::busy_place): it suggests a folder
    /// of its own on the welcome screen.
    busy_place: Option<&'static str>,
    config_file: String,
    config_warning: Option<String>,
    folders: Vec<Folder>,
    steamcmd_installed: bool,
    /// SteamCMD is in its folder (also when it isn't linked: the setup window says so).
    steamcmd_found: bool,
    /// The folders are in use (false after Reset everything, until setup runs).
    linked: bool,
    /// The server files are in the server folder (also when it isn't linked).
    server_found: bool,
    server: Option<InstalledServer>,
    /// Every installed branch, the active one first.
    installs: Vec<InstallInfo>,
    /// SteamCMD or the server files are missing: DropLaunch can't launch maps yet.
    setup_needed: bool,
    framework: Framework,
    rustedit: bool,
    /// Carbon's profiler is switched on (in the active server folder).
    profiler: bool,
    /// Where Carbon saves recorded profiles.
    profiles_dir: String,
    /// The bug report plugin goes into servers (playtest kit).
    playtest: bool,
    /// Oxide is installed but an update replaced its files.
    oxide_inactive: bool,
    /// The procedural map's start file, once made.
    start_file: Option<String>,
    update_script: Option<String>,
    /// Every DropLaunch start_*.bat next to the server.
    start_files: Vec<StartFileInfo>,
    connect_command: String,
    /// Free space on the server folder's drive and on SteamCMD's (the UI colours download sizes).
    free_space: Option<u64>,
    steamcmd_free_space: Option<u64>,
    branch: String,
    branch_cache: Option<BranchCache>,
    auto_join: bool,
    /// Show a notice when a map in the maps folder is saved.
    notify_saves: bool,
    /// Windows notifications while DropLaunch is in the background.
    desktop_notifications: bool,
    /// Remove the game's cached copies of a map before it starts.
    clean_client_maps: bool,
    /// Look for a new DropLaunch when it starts.
    update_check: bool,
    /// Server updates bring Carbon or Oxide up to date too.
    update_framework: bool,
    /// The new version the user doesn't want to be asked about again.
    skipped_update: Option<String>,
    /// Made the owner (admin) of every DropLaunch server.
    owner: Option<SteamUser>,
    /// Only the owner and `whitelist_ids` can join.
    whitelist: bool,
    whitelist_ids: Vec<String>,
    /// Maps removed from the game's cache are kept (in `map_backups_dir`).
    backup_client_maps: bool,
    map_backups_dir: String,
    busy: bool,
    server_running: bool,
    /// The first start's welcome was answered.
    welcomed: bool,
    /// Tips and the Getting started checklist.
    guide: Guide,
}

/// What the UI shows. Read from a copy of the settings: the disk is looked at without holding their
/// lock (a slow or disconnected drive would hold up the server monitor and the bug reports too),
/// and off the threads that answer the UI.
async fn snapshot(state: &AppState) -> CmdResult<Snapshot> {
    let busy = state.job.locked().is_some();
    let live_dir = state.server.live_dir();
    let store = state.store.locked().clone();
    tauri::async_runtime::spawn_blocking(move || read_snapshot(&store, busy, live_dir)).await.map_err(|e| e.to_string())
}

fn read_snapshot(store: &ConfigStore, busy: bool, live_dir: Option<PathBuf>) -> Snapshot {
    let folder = |key, label, path: &Path| Folder {
        key,
        label,
        path: path.display().to_string(),
        exists: path.is_dir(),
        empty: std::fs::read_dir(path).map_or(true, |mut entries| entries.next().is_none()),
    };
    let (steamcmd, server_dir, maps) = (store.steamcmd_dir(), store.server_dir(), store.maps_dir());
    // From the branch in use (another branch's server doesn't hold these files).
    let server_running = live_dir.is_some_and(|d| startfile::same_path(&d, &server_dir));
    // After Reset everything nothing is linked: what's on the disk only shows in the setup window.
    let linked = store.config.linked;
    let steamcmd_found = install::is_installed(&steamcmd);
    let steamcmd_installed = linked && steamcmd_found;
    let server = manifest::read_installed(&server_dir).filter(|_| linked);
    let procgen = startfile::read(&startfile::path_for(&server_dir, &Target::Procgen, false)).filter(|_| linked);
    let update_script = server_dir.join(startfile::UPDATE_SCRIPT);
    let space_probe = existing_ancestor(&server_dir);
    Snapshot {
        version: env!("CARGO_PKG_VERSION"),
        platform: platform::PLATFORM,
        can_host: platform::CAN_HOST_SERVER,
        base_dir: store.base.display().to_string(),
        busy_place: platform::busy_place(&store.base),
        config_file: store.file.display().to_string(),
        config_warning: store.load_error.as_ref().map(|e| e.describe()),
        folders: vec![
            folder("steamcmd", t!("setup.req.steamcmd"), &steamcmd),
            folder("server", t!("folders.server"), &server_dir),
            folder("maps", t!("folders.maps"), &maps),
        ],
        setup_needed: !steamcmd_installed || server.is_none(),
        steamcmd_installed,
        steamcmd_found,
        server_found: manifest::read_installed(&server_dir).is_some(),
        server,
        linked,
        framework: if linked { mods::detect(&server_dir) } else { Framework::None },
        rustedit: linked && mods::has_rustedit(&server_dir),
        profiler: linked && mods::profiler_enabled(&server_dir),
        profiles_dir: mods::profiles_dir(&server_dir).display().to_string(),
        playtest: store.config.playtest,
        oxide_inactive: linked && mods::oxide_needs_reapply(&server_dir),
        connect_command: procgen.as_ref().map_or_else(|| format!("client.connect 127.0.0.1:{}", startfile::DEFAULT_PORT), |s| s.connect_command()),
        start_file: procgen.map(|s| s.path.display().to_string()),
        update_script: (linked && update_script.is_file()).then(|| update_script.display().to_string()),
        start_files: startfile::list(&server_dir)
            .into_iter()
            .filter(|_| linked)
            .into_iter()
            .filter(|s| s.made_by_droplaunch())
            .map(|s| {
                let target = s.map.clone().map_or(Target::Procgen, Target::Map);
                StartFileInfo {
                    prefab: startfile::is_prefab_file(&server_dir, &target, &s.path),
                    map: s.map.as_deref().map(startfile::map_title),
                    path: s.path.display().to_string(),
                }
            })
            .collect(),
        free_space: platform::free_space(&space_probe),
        steamcmd_free_space: platform::free_space(&existing_ancestor(&steamcmd)),
        installs: {
            let mut installs: Vec<InstallInfo> = store
                .installs()
                .into_iter()
                .filter(|_| linked)
                .map(|(dir, i)| InstallInfo {
                    active: i.branch == store.config.branch,
                    branch: i.branch,
                    path: dir.display().to_string(),
                    buildid: i.buildid,
                    fully_installed: i.fully_installed,
                    last_updated: i.last_updated,
                })
                .collect();
            installs.sort_by_key(|i| !i.active);
            installs
        },
        branch: store.config.branch.clone(),
        branch_cache: store.config.branch_cache.clone(),
        auto_join: store.config.auto_join,
        notify_saves: store.config.notify_saves,
        desktop_notifications: store.config.desktop_notifications,
        clean_client_maps: store.config.clean_client_maps,
        update_check: store.config.update_check,
        update_framework: store.config.update_framework,
        skipped_update: store.config.skipped_update.clone(),
        owner: store.config.owner.clone(),
        whitelist: store.config.whitelist,
        whitelist_ids: store.config.whitelist_ids.clone(),
        backup_client_maps: store.config.backup_client_maps,
        map_backups_dir: store.data.join(client::BACKUP_DIR).display().to_string(),
        welcomed: store.config.welcomed,
        guide: store.config.guide.clone(),
        busy,
        server_running,
    }
}

fn save(store: &ConfigStore) -> CmdResult<()> {
    store.save().map_err(|e| t!("error.save", file = store.file.display(), error = format!("{e:#}")))
}

#[tauri::command]
pub async fn get_state(state: State<'_, AppState>) -> CmdResult<Snapshot> {
    snapshot(&state).await
}

#[tauri::command]
/// `branch`: the server folder of a branch being set up (otherwise the active one's).
pub async fn set_folder(state: State<'_, AppState>, key: String, path: String, branch: Option<String>) -> CmdResult<Snapshot> {
    state.ensure_idle()?;
    {
        let mut store = state.store.locked();
        let stored = store.to_stored(Path::new(&path));
        match key.as_str() {
            "steamcmd" => store.config.steamcmd_dir = stored,
            "server" => store.set_server_folder(Path::new(&path), branch.as_deref()),
            "maps" => store.config.maps_dir = stored,
            _ => return Err(t!("error.unknownFolder", key)),
        }
        save(&store)?;
    }
    if key == "maps" {
        state.watcher.watch(state.maps_dir());
    }
    snapshot(&state).await
}

#[tauri::command]
pub async fn pick_folder(app: AppHandle, title: String, start: Option<String>) -> CmdResult<Option<String>> {
    let mut dialog = app.dialog().file().set_title(title);
    if let Some(dir) = start.filter(|d| Path::new(d).is_dir()) {
        dialog = dialog.set_directory(dir);
    }
    Ok(dialog.blocking_pick_folder().and_then(|p| p.into_path().ok()).map(|p| p.display().to_string()))
}

#[tauri::command]
pub async fn open_folder(path: String) -> CmdResult<()> {
    let path = Path::new(&path);
    if !path.is_dir() {
        return Err(t!("error.notYet", path = path.display()));
    }
    platform::open_in_file_manager(path).map_err(|e| e.to_string())
}

/// Show a file in Explorer.
#[tauri::command]
pub async fn reveal(path: String) -> CmdResult<()> {
    let path = Path::new(&path);
    if !path.exists() {
        return Err(t!("error.gone", path = path.display()));
    }
    platform::reveal(path).map_err(|e| e.to_string())
}

/// Where a branch is installed, or would go (next to the others).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchFolder {
    path: String,
    exists: bool,
    installed: Option<InstalledServer>,
    /// On its drive.
    free_space: Option<u64>,
}

#[tauri::command]
pub async fn branch_folder(state: State<'_, AppState>, branch: String) -> CmdResult<BranchFolder> {
    let dir = state.store.locked().server_dir_for(&branch);
    Ok(BranchFolder {
        path: dir.display().to_string(),
        exists: dir.is_dir(),
        installed: manifest::read_installed(&dir),
        free_space: platform::free_space(&existing_ancestor(&dir)),
    })
}

/// The branch servers start with (an installed one switches right away; a new one is set up
/// with setup_all). Also while SteamCMD works on another branch: each has its own folder, and a
/// job keeps the folder it started with.
#[tauri::command]
pub async fn set_branch(state: State<'_, AppState>, branch: String) -> CmdResult<Snapshot> {
    {
        let mut store = state.store.locked();
        store.config.branch = branch;
        save(&store)?;
    }
    snapshot(&state).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionsPatch {
    auto_join: Option<bool>,
    notify_saves: Option<bool>,
    desktop_notifications: Option<bool>,
    clean_client_maps: Option<bool>,
    update_check: Option<bool>,
    update_framework: Option<bool>,
    /// "" clears it.
    skipped_update: Option<String>,
    welcomed: Option<bool>,
    /// Carbon's profiler in the active server folder (from the next server start).
    profiler: Option<bool>,
    /// The bug report plugin (the running server's folder follows right away).
    playtest: Option<bool>,
    /// The whitelist on or off, and the players it lets in besides the owner.
    whitelist: Option<bool>,
    whitelist_ids: Option<Vec<String>>,
    backup_client_maps: Option<bool>,
    guide: Option<GuidePatch>,
}

/// The guide switched on or off, tips read, checklist steps done, or all of it again.
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct GuidePatch {
    on: Option<bool>,
    seen: Vec<String>,
    done: Vec<String>,
    /// Start over: every tip and step again.
    reset: bool,
}

#[tauri::command]
pub async fn set_options(state: State<'_, AppState>, options: OptionsPatch) -> CmdResult<Snapshot> {
    let whitelist_changed = options.whitelist.is_some() || options.whitelist_ids.is_some();
    {
        let mut store = state.store.locked();
        if let Some(v) = options.auto_join {
            store.config.auto_join = v;
        }
        if let Some(v) = options.notify_saves {
            store.config.notify_saves = v;
        }
        if let Some(v) = options.desktop_notifications {
            store.config.desktop_notifications = v;
        }
        if let Some(v) = options.clean_client_maps {
            store.config.clean_client_maps = v;
        }
        if let Some(v) = options.update_check {
            store.config.update_check = v;
        }
        if let Some(v) = options.update_framework {
            store.config.update_framework = v;
        }
        if let Some(v) = options.skipped_update {
            store.config.skipped_update = Some(v).filter(|v| !v.is_empty());
        }
        if let Some(v) = options.welcomed {
            store.config.welcomed = v;
        }
        if let Some(v) = options.backup_client_maps {
            store.config.backup_client_maps = v;
        }
        if let Some(v) = options.whitelist {
            store.config.whitelist = v;
        }
        if let Some(ids) = options.whitelist_ids {
            let mut clean = Vec::new();
            for id in ids {
                let id = owner::parse_steam_id(&id)?;
                if !clean.contains(&id) {
                    clean.push(id);
                }
            }
            store.config.whitelist_ids = clean;
        }
        if let Some(patch) = options.guide {
            let guide = &mut store.config.guide;
            if patch.reset {
                guide.seen.clear();
                guide.done.clear();
            }
            if let Some(on) = patch.on {
                guide.on = on;
            }
            Guide::add(&mut guide.seen, patch.seen);
            Guide::add(&mut guide.done, patch.done);
        }
        if let Some(on) = options.profiler {
            mods::set_profiler(&store.server_dir(), on).map_err(|e| t!("error.profiler", error = e))?;
        }
        save(&store)?;
    }
    if let Some(on) = options.playtest {
        let playtest = state.playtest.clone();
        tauri::async_runtime::spawn_blocking(move || playtest.set_enabled(on)).await.map_err(|e| e.to_string())??;
    }
    if whitelist_changed {
        sync_whitelist(&state);
    }
    snapshot(&state).await
}

/// The whitelist file in every server folder (a running server's plugin reads it at the next
/// login), and the plugin that checks it in or out of the running server's folder.
fn sync_whitelist(state: &AppState) {
    let (dirs, ids) = {
        let store = state.store.locked();
        (store.installs().into_iter().map(|(dir, _)| dir).collect::<Vec<_>>(), store.config.whitelist_ids())
    };
    for dir in dirs {
        if let Err(e) = whitelist::write(&dir, ids.as_deref()) {
            crate::log::write(&format!("whitelist: {e}"));
        }
    }
    state.playtest.sync_plugin();
}

/// Settings → About → Reset everything: every setting back to its default (the welcome screen
/// shows again at the next start), the folders and installed branches kept. Nothing is deleted:
/// the server files, maps, kits and bug reports stay; a deleted built-in kit comes back.
#[tauri::command]
pub async fn reset_settings(state: State<'_, AppState>) -> CmdResult<Snapshot> {
    // The server folders in use until now: they're let go of as they were set up.
    let (old_dirs, had_owner) = {
        let store = state.store.locked();
        (store.installs().into_iter().map(|(dir, _)| dir).collect::<Vec<_>>(), store.config.owner.is_some())
    };
    for dir in &old_dirs {
        // No owner any more: out of users.cfg too (as with Make me the server owner off).
        if had_owner {
            let identities: std::collections::BTreeSet<String> =
                startfile::list(dir).into_iter().filter(|s| s.made_by_droplaunch()).map(|s| s.identity).collect();
            for identity in identities {
                let _ = owner::apply(dir, &identity, None);
            }
        }
        // The whitelist off there (a start file double-clicked later lets everyone in).
        let _ = whitelist::write(dir, None);
    }
    {
        let mut store = state.store.locked();
        let old = std::mem::take(&mut store.config);
        // Like a fresh DropLaunch: the default folders next to it, none of them linked until setup
        // runs again (it finds what's there). Only the branch list Steam gave is kept.
        store.config = Config { branch_cache: old.branch_cache, last_used: old.last_used, folder: old.folder, linked: false, legacy_drops: false, ..Config::default() };
        save(&store)?;
        let languages = lang::list(&store.base, &store.data);
        let _ = lang::activate(lang::choose(None, &languages));
        let _ = std::fs::remove_file(store.data.join(kits::BUILT_IN_FILE));
    }
    // The plugin out of a running server's folder (bug reports and the whitelist are off), and the
    // bug reports put aside: numbering starts at #1 again.
    state.playtest.sync_plugin();
    let _ = state.playtest.clear_all();
    snapshot(&state).await
}

/// Instant branch list from the steamcmd.net mirror (no SteamCMD, no job slot needed).
#[tauri::command]
pub async fn quick_branches(state: State<'_, AppState>) -> CmdResult<Snapshot> {
    let branches = tauri::async_runtime::spawn_blocking(appinfo::fetch_mirror)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("{e:#}"))?;
    {
        let mut store = state.store.locked();
        let previous = store.config.branch_cache.take();
        store.config.branch_cache = Some(BranchCache::merged(previous, branches, "steamcmd.net"));
        save(&store)?;
    }
    snapshot(&state).await
}

/// Authoritative branch list straight from Steam through SteamCMD.
#[tauri::command]
pub async fn fetch_branches(state: State<'_, AppState>, on_event: Channel<JobEvent>) -> CmdResult<Snapshot> {
    let (token, slot) = state.start_job(None)?;
    let steamcmd_dir = state.store.locked().steamcmd_dir();
    let branches = tauri::async_runtime::spawn_blocking(move || {
        jobs::fetch_branches(&steamcmd_dir, &token, &mut |e| {
            let _ = on_event.send(e);
        })
    })
    .await
    .map_err(|e| e.to_string())??;
    {
        let mut store = state.store.locked();
        store.config.branch_cache = Some(BranchCache::merged(None, branches, "Steam"));
        save(&store)?;
    }
    drop(slot);
    snapshot(&state).await
}

#[tauri::command]
pub async fn server_update(state: State<'_, AppState>, validate: bool, on_event: Channel<JobEvent>) -> CmdResult<JobSummary> {
    let server_dir = state.store.locked().server_dir();
    // A server from another branch's folder can keep running.
    if state.server.runs_from(&server_dir) {
        return Err(t!("error.serverRunning"));
    }
    if let Some(short) = space_short(&server_dir) {
        return Err(short.message());
    }
    let (token, _slot) = state.start_job(Some(server_dir.clone()))?;
    let req = {
        let mut store = state.store.locked();
        let branch = store.config.branch.clone();
        store.remember_branch_dir(&branch, &server_dir);
        save(&store)?;
        UpdateRequest {
            base_dir: store.base.clone(),
            steamcmd_dir: store.steamcmd_dir(),
            server_dir: store.server_dir(),
            branch: store.config.branch.clone(),
            validate,
            update_framework: store.config.update_framework,
        }
    };
    tauri::async_runtime::spawn_blocking(move || {
        jobs::update_server(&req, &token, &mut |e| {
            let _ = on_event.send(e);
        })
    })
    .await
    .map_err(|e| e.to_string())
}

/// Put Oxide back after an update replaced its files.
#[tauri::command]
pub async fn reapply_oxide(state: State<'_, AppState>, on_event: Channel<JobEvent>) -> CmdResult<JobSummary> {
    let server_dir = state.store.locked().server_dir();
    if state.server.runs_from(&server_dir) {
        return Err(t!("error.serverRunning"));
    }
    let (token, _slot) = state.start_job(Some(server_dir.clone()))?;
    tauri::async_runtime::spawn_blocking(move || {
        jobs::reapply_oxide(&server_dir, &token, &mut |e| {
            let _ = on_event.send(e);
        })
    })
    .await
    .map_err(|e| e.to_string())
}

/// The one-click fix from the setup window.
#[tauri::command]
pub async fn setup_all(state: State<'_, AppState>, options: SetupOptions, on_event: Channel<JobEvent>) -> CmdResult<JobSummary> {
    let server_dir = state.store.locked().server_dir_for(&options.branch);
    // A server from another branch's folder can keep running (and be used meanwhile).
    if state.server.runs_from(&server_dir) {
        return Err(t!("error.serverRunning"));
    }
    if let Some(short) = space_short(&server_dir) {
        return Err(short.message());
    }
    let (token, _slot) = state.start_job(Some(server_dir.clone()))?;
    let server = state.server.clone();
    let store_handle = state.store.clone();
    let launch_dir = server_dir.clone();
    let launcher: Launcher = Box::new(move |target: Target, prefab: bool| {
        if let Target::Map(map) = &target {
            remember(&store_handle, map);
        }
        // Built now: the owner may have been set while SteamCMD downloaded. From the folder that
        // was set up (servers keep starting from the branch in use until setup is done).
        let ctx = StartContext::with_dir(&store_handle.locked(), launch_dir.clone());
        match server.start(target, prefab, ctx, false)? {
            StartReply::Started { .. } => Ok(t!("job.launch.running")),
            StartReply::Busy { title } => Err(t!("job.launch.busy", title)),
            StartReply::Queued { .. } | StartReply::OxideInactive => Ok(t!("job.launch.starting")),
        }
    });
    let live_options = Arc::new(Mutex::new(options.clone()));
    let owner_store = state.store.clone();
    let extras_store = state.store.clone();
    let extras_dir = server_dir.clone();
    let extras: Box<dyn Fn(&SetupOptions) -> Vec<String> + Send> = Box::new(move |opts: &SetupOptions| {
        let mut lines = Vec::new();
        let (plugin, whitelist) = {
            let mut store = extras_store.locked();
            store.config.playtest = opts.playtest;
            store.config.whitelist = opts.whitelist;
            let _ = store.save();
            (store.config.wants_plugin(), store.config.whitelist_ids())
        };
        if let Err(e) = whitelist::write(&extras_dir, whitelist.as_deref()) {
            lines.push(t!("serverlog.whitelistFailed", error = e));
        } else if let Some(ids) = &whitelist {
            lines.push(if ids.len() == 1 { t!("serverlog.whitelist.one") } else { t!("serverlog.whitelist.other", n = ids.len()) });
        }
        if plugin {
            match playtest::install_plugin(&extras_dir) {
                Ok(Some(path)) => lines.push(t!("job.plugin.added", path = path.display())),
                Ok(None) => lines.push(t!("job.plugin.needs")),
                Err(e) => lines.push(t!("job.plugin.failed", error = e)),
            }
        } else {
            playtest::remove_plugin(&extras_dir);
        }
        let profiler = opts.profiler && opts.framework == Framework::Carbon;
        match mods::set_profiler(&extras_dir, profiler) {
            Ok(true) if profiler => lines.push(t!("job.profiler.on")),
            Err(e) => lines.push(t!("error.profiler", error = e)),
            _ => {}
        }
        lines
    });
    let req = {
        let mut store = state.store.locked();
        store.remember_branch_dir(&options.branch, &server_dir);
        save(&store)?;
        SetupRequest {
            base_dir: store.base.clone(),
            steamcmd_dir: store.steamcmd_dir(),
            server_dir: server_dir.clone(),
            maps_dir: store.maps_dir(),
            options: live_options.clone(),
            owner: Box::new(move || owner_store.locked().config.owner.clone()),
            extras,
            launcher,
        }
    };
    *state.setup_options.locked() = Some(live_options);
    let result = tauri::async_runtime::spawn_blocking(move || {
        jobs::setup_all(&req, &token, &mut |e| {
            let _ = on_event.send(e);
        })
    })
    .await
    .map_err(|e| e.to_string());
    *state.setup_options.locked() = None;
    // The branch that was set up is the one in use from now on.
    if result.as_ref().is_ok_and(|summary| summary.ok) {
        let mut store = state.store.locked();
        store.config.branch = options.branch.clone();
        // Set up: the folders are DropLaunch's again (after Reset everything).
        store.config.linked = true;
        save(&store)?;
    }
    result
}

/// Not enough room on the drive for the server files.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpaceShort {
    free: u64,
    needed: u64,
    /// "drive C:" (or "that drive").
    drive: String,
}

impl SpaceShort {
    fn message(&self) -> String {
        let gb = |bytes: u64| format!("{:.1} GB", bytes as f64 / 1024f64.powi(3));
        t!("error.space", needed = gb(self.needed), drive = self.drive, free = gb(self.free))
    }
}

/// The folder, or the closest parent that's there (where it will be made).
fn existing_ancestor(dir: &Path) -> PathBuf {
    dir.ancestors().find(|p| p.is_dir()).map_or_else(|| dir.to_path_buf(), Path::to_path_buf)
}

/// Checked before the server files are downloaded. What's there from a download that didn't
/// finish counts.
fn space_short(server_dir: &Path) -> Option<SpaceShort> {
    if manifest::read_installed(server_dir).is_some() {
        return None;
    }
    let probe = existing_ancestor(server_dir);
    let free = platform::free_space(&probe)?;
    let needed = SERVER_SPACE.saturating_sub(dir_size(server_dir));
    let drive = match probe.components().next() {
        Some(std::path::Component::Prefix(prefix)) => t!("error.drive", letter = prefix.as_os_str().to_string_lossy()),
        _ => t!("units.thatDrive"),
    };
    (free < needed).then_some(SpaceShort { free, needed, drive })
}

fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            Ok(_) => e.metadata().map_or(0, |m| m.len()),
            Err(_) => 0,
        })
        .sum()
}

/// Is there room for a branch's server files (the active one by default)? None: yes, or
/// they're there already.
#[tauri::command]
pub async fn server_space(state: State<'_, AppState>, branch: Option<String>) -> CmdResult<Option<SpaceShort>> {
    let server_dir = {
        let store = state.store.locked();
        branch.map_or_else(|| store.server_dir(), |b| store.server_dir_for(&b))
    };
    tauri::async_runtime::spawn_blocking(move || space_short(&server_dir)).await.map_err(|e| e.to_string())
}

/// Change the options while setup downloads (the mod framework and what comes with it, the
/// whitelist, starting the server and how). The branch and the map to start stay as they began.
#[tauri::command]
pub async fn update_setup_options(state: State<'_, AppState>, options: SetupOptions) -> CmdResult<()> {
    if let Some(live) = state.setup_options.locked().as_ref() {
        let mut live = live.locked();
        live.framework = options.framework;
        live.rustedit = options.rustedit;
        live.start_server = options.start_server;
        live.playtest = options.playtest;
        live.profiler = options.profiler;
        live.prefab = options.prefab;
        live.whitelist = options.whitelist;
    }
    Ok(())
}

#[tauri::command]
pub async fn cancel_job(state: State<'_, AppState>) -> CmdResult<()> {
    if let Some(job) = state.job.locked().as_ref() {
        job.token.cancel();
    }
    Ok(())
}

// ---------------------------------------------------------------- maps

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapList {
    maps_dir: String,
    maps_dir_exists: bool,
    maps: Vec<MapEntry>,
    /// Maps that were started, newest first.
    history: Vec<MapEntry>,
}

#[tauri::command]
pub async fn list_maps(state: State<'_, AppState>) -> CmdResult<MapList> {
    let (maps_dir, server_dir, steamcmd_dir, history, linked) = {
        let store = state.store.locked();
        (store.maps_dir(), store.server_dir(), store.steamcmd_dir(), store.config.history.clone(), store.config.linked)
    };
    // Not linked (after Reset everything): no maps folder until setup has run.
    if !linked {
        return Ok(MapList { maps_dir_exists: false, maps_dir: maps_dir.display().to_string(), maps: Vec::new(), history: Vec::new() });
    }
    tauri::async_runtime::spawn_blocking(move || {
        // The setup makes it; once set up, a removed maps folder comes back. Before that (a first
        // start) nothing is made yet.
        if install::is_installed(&steamcmd_dir) && manifest::read_installed(&server_dir).is_some() {
            let _ = std::fs::create_dir_all(&maps_dir);
        }
        let start_files = maps::StartFiles::read(&server_dir);
        let maps = maps::scan(&maps_dir, &start_files);
        let history = history
            .iter()
            .filter_map(|h| {
                let mut entry = maps::entry(Path::new(&h.path), &maps_dir, &start_files)?;
                entry.started = Some(h.started);
                Some(entry)
            })
            .collect();
        MapList { maps_dir_exists: maps_dir.is_dir(), maps_dir: maps_dir.display().to_string(), maps, history }
    })
    .await
    .map_err(|e| e.to_string())
}

/// Dropped files / files the exe was opened with -> maps that can be started.
#[tauri::command]
pub async fn import_files(state: State<'_, AppState>, paths: Vec<String>) -> CmdResult<Imported> {
    let maps_dir = state.store.locked().maps_dir();
    let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    let data = state.data_dir();
    let folder = state.store.locked().config.folder.clone();
    let dropped = paths.clone();
    let dir = maps_dir.clone();
    let mut imported = tauri::async_runtime::spawn_blocking(move || maps::import(&paths, &maps_dir)).await.map_err(|e| e.to_string())?;
    // A zip's .json files: the kits and language files go on to the UI, other files stay put.
    imported.files.retain(|f| kits::read_kit(Path::new(f)).is_ok() || lang::is_language_file(Path::new(f)));
    // Kept for Settings (Dropped assets): the maps, and each zip with the maps it held.
    for path in &dropped {
        if maps::is_map(path) && path.is_file() {
            assets::record(&data, &folder, assets::Kind::Map, path, Vec::new());
        } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")) {
            let into = maps::zip_folder(&dir, path);
            let made = imported.maps.iter().filter(|m| startfile::is_inside(Path::new(m), &into)).cloned().collect();
            assets::record(&data, &folder, assets::Kind::Zip, path, made);
        }
    }
    // Maps unpacked from a .zip land in the watched folder: that's not a save.
    state.watcher.quiet(&imported.maps);
    Ok(imported)
}

#[tauri::command]
pub async fn forget_map(state: State<'_, AppState>, path: String) -> CmdResult<()> {
    let mut store = state.store.locked();
    store.config.history.retain(|h| !h.path.eq_ignore_ascii_case(&path));
    save(&store)
}

#[tauri::command]
pub async fn take_pending_files(state: State<'_, AppState>) -> CmdResult<Vec<String>> {
    Ok(std::mem::take(&mut *state.pending_files.locked()))
}

fn remember(store: &Mutex<ConfigStore>, map: &Path) {
    let mut store = store.locked();
    store.remember_map(map);
    let _ = store.save();
}

// ---------------------------------------------------------------- server

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TargetArg {
    Procgen,
    Map { path: String },
}

impl From<TargetArg> for Target {
    fn from(arg: TargetArg) -> Self {
        match arg {
            TargetArg::Procgen => Target::Procgen,
            TargetArg::Map { path } => Target::Map(PathBuf::from(path)),
        }
    }
}

#[tauri::command]
pub async fn start_server(
    state: State<'_, AppState>,
    target: TargetArg,
    prefab: bool,
    replace: bool,
    ignore_oxide: bool,
) -> CmdResult<StartReply> {
    let target = Target::from(target);
    // After Reset everything nothing starts until setup has linked the folders again.
    if !state.store.locked().config.linked {
        return Err(t!("error.installFirst"));
    }
    let ctx = state.start_context();
    // SteamCMD may be busy with another branch's folder meanwhile.
    state.ensure_free(&ctx.server_dir)?;
    if manifest::read_installed(&ctx.server_dir).is_none() {
        return Err(t!("error.installFirst"));
    }
    if !ignore_oxide && mods::oxide_needs_reapply(&ctx.server_dir) {
        return Ok(StartReply::OxideInactive);
    }
    // Only maps that exist go in the history (a typo'd path doesn't).
    if let Target::Map(map) = target.clone() {
        let reply = state.server.start(target, prefab, ctx, replace)?;
        remember(&state.store, &map);
        return Ok(reply);
    }
    state.server.start(target, prefab, ctx, replace)
}

#[tauri::command]
pub async fn stop_server(state: State<'_, AppState>, force: bool) -> CmdResult<()> {
    state.server.stop(force)
}

/// Stop and start the running server again (from its own folder).
#[tauri::command]
pub async fn restart_server(state: State<'_, AppState>) -> CmdResult<()> {
    if let Some(dir) = state.server.live_dir() {
        state.ensure_free(&dir)?;
    }
    let ctx = state.start_context();
    state.server.restart(ctx)
}

#[tauri::command]
pub async fn server_command(state: State<'_, AppState>, text: String) -> CmdResult<()> {
    state.server.command(&text)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatus {
    server: Option<ServerView>,
    lines: Vec<ConsoleLine>,
}

#[tauri::command]
pub async fn server_status(state: State<'_, AppState>) -> CmdResult<ServerStatus> {
    let (server, lines) = state.server.status();
    Ok(ServerStatus { server, lines })
}

/// Write a map's start files (normal + prefab test mode) without starting anything.
#[tauri::command]
pub async fn create_start_file(state: State<'_, AppState>, target: TargetArg) -> CmdResult<Vec<String>> {
    let ctx = state.start_context();
    if manifest::read_installed(&ctx.server_dir).is_none() {
        return Err(t!("error.installFirst"));
    }
    let target = Target::from(target);
    let layout = Layout { base: &ctx.base, server_dir: &ctx.server_dir, steamcmd_dir: &ctx.steamcmd_dir };
    let error = |e| t!("error.startFile", error = e);
    let (normal, _) = startfile::ensure(&layout, &target, false).map_err(error)?;
    let _ = owner::apply(&ctx.server_dir, &normal.identity, ctx.owner.as_ref());
    let mut paths = vec![normal.path.display().to_string()];
    if matches!(target, Target::Map(_)) {
        paths.push(startfile::path_for(&ctx.server_dir, &target, true).display().to_string());
    }
    Ok(paths)
}

/// Ask the running server something over RCON without it showing in the console (e.g. whether
/// Carbon's profiler still records); its reply.
#[tauri::command]
pub async fn server_ask(state: State<'_, AppState>, text: String) -> CmdResult<String> {
    let server = state.server.clone();
    tauri::async_runtime::spawn_blocking(move || server.rcon_call(&text, std::time::Duration::from_secs(5))).await.map_err(|e| e.to_string())?
}

/// An account's picture, as Steam keeps it on this PC (PNG bytes).
#[tauri::command]
pub async fn steam_avatar(steam_id: String) -> CmdResult<tauri::ipc::Response> {
    owner::avatar(&steam_id).map(tauri::ipc::Response::new).ok_or_else(|| t!("error.noAvatar"))
}

/// Steam accounts that signed in on this PC (read from Steam's files), the signed-in one first.
#[tauri::command]
pub async fn steam_accounts() -> CmdResult<Vec<LocalAccount>> {
    Ok(tauri::async_runtime::spawn_blocking(owner::local_accounts).await.unwrap_or_default())
}

/// Make a player the owner (admin) of every DropLaunch server, or stop doing that (None).
#[tauri::command]
pub async fn set_owner(state: State<'_, AppState>, steam_id: Option<String>, name: Option<String>) -> CmdResult<Snapshot> {
    let owner = match steam_id.filter(|id| !id.trim().is_empty()) {
        Some(id) => Some(SteamUser { steam_id: owner::parse_steam_id(&id)?, name: owner::clean_name(name.as_deref().unwrap_or("")) }),
        None => None,
    };
    let (server_dir, previous) = {
        let mut store = state.store.locked();
        let previous = std::mem::replace(&mut store.config.owner, owner.clone());
        save(&store)?;
        (store.server_dir(), previous)
    };
    // Every server DropLaunch made, also for when its start file is double-clicked later.
    let identities: std::collections::BTreeSet<String> =
        startfile::list(&server_dir).into_iter().filter(|s| s.made_by_droplaunch()).map(|s| s.identity).collect();
    for identity in identities {
        owner::apply(&server_dir, &identity, owner.as_ref()).map_err(|e| t!("error.usersCfg", identity, error = e))?;
    }
    // A running server takes it right away (it reads users.cfg only when it starts).
    if previous != owner {
        if let Some(old) = previous.filter(|old| owner.as_ref().is_none_or(|o| o.steam_id != old.steam_id)) {
            let _ = state.server.command(&format!("removeowner {}", old.steam_id));
        }
        if let Some(o) = &owner {
            let _ = state.server.command(&format!("ownerid {} \"{}\" \"{}\"", o.steam_id, o.name, owner::NOTE));
        }
    }
    // The whitelist lets the (new) owner in.
    sync_whitelist(&state);
    snapshot(&state).await
}

/// Open a web page in the browser (https only).
#[tauri::command]
pub async fn open_link(url: String) -> CmdResult<()> {
    if !url.starts_with("https://") {
        return Err(t!("error.httpsOnly"));
    }
    platform::open_url(&url).map_err(|e| e.to_string())
}

/// Start the Rust game the running server needs (Rust, or Rust - Staging Branch for staging) and
/// join it. A Rust that's open already, or a game that isn't installed: the UI asks.
#[tauri::command]
pub async fn join_server(state: State<'_, AppState>) -> CmdResult<client::JoinPlan> {
    let (port, branch) = state.server.join_target();
    tauri::async_runtime::spawn_blocking(move || client::plan_join(&branch, port, true)).await.map_err(|e| e.to_string())?
}

/// Bring the Rust game for `branch` to the front (for F1 + paste).
#[tauri::command]
pub async fn show_rust(branch: String) -> CmdResult<bool> {
    tauri::async_runtime::spawn_blocking(move || client::show(&branch)).await.map_err(|e| e.to_string())
}

/// Close the Rust that's open (like its X button) and start the game for `branch` straight into
/// the server on this PC at `port`. Rust only takes a server to join when it starts.
#[tauri::command]
pub async fn restart_rust_and_join(port: u16, branch: String) -> CmdResult<()> {
    if port < 1024 {
        return Err(t!("error.badPort", port));
    }
    tauri::async_runtime::spawn_blocking(move || client::restart_into(&branch, port)).await.map_err(|e| e.to_string())?
}

/// Steam's install window for the Rust game `branch` needs (Rust - Staging Branch is free for
/// Rust owners).
#[tauri::command]
pub async fn install_rust_game(branch: String) -> CmdResult<()> {
    client::install_game(&branch)
}

// ---------------------------------------------------------------- console: autocomplete, history

/// The running (or active) server's console commands, for the console's autocomplete. The
/// running server's own folder: its branch's build and mods.
#[tauri::command]
pub async fn console_commands(state: State<'_, AppState>) -> CmdResult<Vec<concmd::Command>> {
    let server_dir = state.server.live_dir().unwrap_or_else(|| state.store.locked().server_dir());
    tauri::async_runtime::spawn_blocking(move || concmd::list(&server_dir)).await.map_err(|e| e.to_string())
}

const HISTORY_FILE: &str = "console-history.txt";
const HISTORY_KEEP: usize = 200;

/// Commands typed in the console before, oldest first (kept in the app data).
#[tauri::command]
pub async fn console_history(state: State<'_, AppState>) -> CmdResult<Vec<String>> {
    let file = state.data_dir().join(HISTORY_FILE);
    Ok(std::fs::read_to_string(file).map(|t| t.lines().filter(|l| !l.trim().is_empty()).map(String::from).collect()).unwrap_or_default())
}

/// Add a command to the console's history (newest last; a repeat moves to the end).
#[tauri::command]
pub async fn console_history_add(state: State<'_, AppState>, text: String) -> CmdResult<()> {
    let text = text.replace(['\r', '\n'], " ").trim().to_string();
    if text.is_empty() {
        return Ok(());
    }
    let data = state.data_dir();
    let file = data.join(HISTORY_FILE);
    let mut lines: Vec<String> = std::fs::read_to_string(&file).map(|t| t.lines().map(String::from).collect()).unwrap_or_default();
    lines.retain(|l| *l != text && !l.trim().is_empty());
    lines.push(text);
    let skip = lines.len().saturating_sub(HISTORY_KEEP);
    let _ = std::fs::create_dir_all(&data);
    std::fs::write(&file, lines[skip..].join("\n") + "\n").map_err(|e| t!("error.history", error = e))
}

// ---------------------------------------------------------------- languages

/// The language in use (every text, English where it has none) and the ones there are.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LanguageState {
    code: String,
    name: String,
    strings: std::collections::HashMap<String, String>,
    languages: Vec<lang::Language>,
    /// Where dropped language files are kept.
    folder: String,
}

#[tauri::command]
pub async fn language_state(state: State<'_, AppState>) -> CmdResult<LanguageState> {
    let (base, data) = {
        let store = state.store.locked();
        (store.base.clone(), store.data.clone())
    };
    let current = lang::current();
    Ok(LanguageState {
        code: current.code,
        name: current.name,
        strings: lang::strings(),
        languages: lang::list(&base, &data),
        folder: data.join(lang::FOLDER).display().to_string(),
    })
}

/// Use another language (the UI reloads to show it).
#[tauri::command]
pub async fn set_language(state: State<'_, AppState>, code: String) -> CmdResult<()> {
    let mut store = state.store.locked();
    let available = lang::list(&store.base, &store.data);
    let language = available.iter().find(|l| l.code.eq_ignore_ascii_case(&code)).ok_or_else(|| t!("lang.error.unknown", code))?;
    lang::activate(language)?;
    store.config.language = Some(language.code.clone());
    save(&store)
}

#[derive(Serialize)]
pub struct AddedLanguages {
    added: Vec<lang::Language>,
    problems: Vec<String>,
}

/// Language files dropped on the window (or on the exe while DropLaunch runs).
#[tauri::command]
pub async fn add_languages(state: State<'_, AppState>, paths: Vec<String>) -> CmdResult<AddedLanguages> {
    let data = state.data_dir();
    let folder = state.store.locked().config.folder.clone();
    let mut out = AddedLanguages { added: Vec::new(), problems: Vec::new() };
    for path in paths {
        match lang::import(Path::new(&path), &data) {
            Ok(language) => {
                assets::record(&data, &folder, assets::Kind::Language, Path::new(&path), language.file.iter().map(|f| f.display().to_string()).collect());
                out.added.push(language);
            }
            Err(e) => out.problems.push(e),
        }
    }
    Ok(out)
}

/// The folder dropped language files are kept in (made when it isn't there yet).
#[tauri::command]
pub async fn open_languages_folder(state: State<'_, AppState>) -> CmdResult<()> {
    let dir = state.data_dir().join(lang::FOLDER);
    std::fs::create_dir_all(&dir).map_err(|e| t!("lang.error.save", error = e))?;
    platform::open_in_file_manager(&dir).map_err(|e| e.to_string())
}

/// DropLaunch's English texts as a language file, to start a translation from. None: cancelled.
#[tauri::command]
pub async fn export_language(app: AppHandle) -> CmdResult<Option<String>> {
    let Some(path) = app
        .dialog()
        .file()
        .set_title(t!("lang.export.dialog"))
        .set_file_name("DropLaunch-English.json")
        .add_filter(t!("lang.export.filter"), &["json"])
        .blocking_save_file()
        .and_then(|p| p.into_path().ok())
    else {
        return Ok(None);
    };
    std::fs::write(&path, lang::builtin_text()).map_err(|e| t!("error.save", file = path.display(), error = e))?;
    Ok(Some(path.display().to_string()))
}

// ---------------------------------------------------------------- the Rust game's map cache

#[tauri::command]
pub async fn client_cache() -> CmdResult<CacheInfo> {
    tauri::async_runtime::spawn_blocking(client::cache_info).await.map_err(|e| e.to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearReply {
    cleaned: Cleaned,
    cache: CacheInfo,
}

/// Remove every map from the game's cache (it downloads or makes them again when needed).
#[tauri::command]
pub async fn clear_client_cache(state: State<'_, AppState>) -> CmdResult<ClearReply> {
    let backups = {
        let store = state.store.locked();
        store.config.backup_client_maps.then(|| store.data.join(client::BACKUP_DIR))
    };
    tauri::async_runtime::spawn_blocking(move || {
        let cleaned = client::clear_cache(backups.as_deref());
        ClearReply { cleaned, cache: client::cache_info() }
    })
    .await
    .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- notifications and updates

/// A Windows notification (when they're switched on; the UI only asks while it's in the
/// background). A click comes back as a "notification-clicked" event.
#[tauri::command]
pub async fn notify(app: AppHandle, state: State<'_, AppState>, toast: Toast) -> CmdResult<bool> {
    if !state.store.locked().config.desktop_notifications {
        return Ok(false);
    }
    toast::show(&app, toast).map(|()| true)
}

/// Is there a newer DropLaunch? (None: this is the newest.)
#[tauri::command]
pub async fn check_update(state: State<'_, AppState>) -> CmdResult<Option<UpdateInfo>> {
    let found = tauri::async_runtime::spawn_blocking(updater::check).await.map_err(|e| e.to_string())??;
    let (manifest, info) = found.unzip();
    *state.update.locked() = manifest;
    Ok(info)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProgress {
    done: u64,
    total: u64,
}

/// Download the update the last check found, put it in place and restart DropLaunch with it.
/// A running server keeps running (the new DropLaunch finds it again).
#[tauri::command]
pub async fn install_update(app: AppHandle, state: State<'_, AppState>, on_event: Channel<UpdateProgress>) -> CmdResult<()> {
    let Some(manifest) = state.update.locked().clone() else {
        return Err(t!("error.checkFirst"));
    };
    tauri::async_runtime::spawn_blocking(move || {
        let mut last = std::time::Instant::now();
        let file = updater::download(&manifest, &mut |done, total| {
            if done == total || last.elapsed() >= std::time::Duration::from_millis(150) {
                last = std::time::Instant::now();
                let _ = on_event.send(UpdateProgress { done, total });
            }
        })?;
        let exe = updater::install(&file)?;
        updater::launch(&exe)
    })
    .await
    .map_err(|e| e.to_string())??;
    // Close after the reply went out; the new DropLaunch waits for this one.
    let handle = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        handle.exit(0);
    });
    Ok(())
}

// ---------------------------------------------------------------- playtest kit (/bug)

#[tauri::command]
pub async fn playtest_status(state: State<'_, AppState>) -> CmdResult<PlaytestView> {
    Ok(state.playtest.view())
}

#[tauri::command]
pub async fn playtest_set_fixed(state: State<'_, AppState>, id: String, fixed: bool) -> CmdResult<PlaytestView> {
    state.playtest.set_fixed(&id, fixed)
}

/// Settings → Servers: how many bug reports are kept, over how many maps.
#[tauri::command]
pub async fn bug_reports_summary(state: State<'_, AppState>) -> CmdResult<playtest::ReportsSummary> {
    Ok(state.playtest.summary())
}

/// Settings → Servers → Clear bug reports: every map's reports put aside, numbering from #1.
#[tauri::command]
pub async fn bug_reports_clear(state: State<'_, AppState>) -> CmdResult<usize> {
    state.playtest.clear_all()
}

#[tauri::command]
pub async fn playtest_delete(state: State<'_, AppState>, id: String) -> CmdResult<PlaytestView> {
    state.playtest.delete(&id)
}

/// Teleport the host in the game: to a report's spot, or to a tester.
#[tauri::command]
pub async fn playtest_teleport(state: State<'_, AppState>, report: Option<String>, player: Option<String>) -> CmdResult<()> {
    let playtest = state.playtest.clone();
    tauri::async_runtime::spawn_blocking(move || playtest.teleport(report.as_deref(), player.as_deref())).await.map_err(|e| e.to_string())?
}

/// Teleport one tester (Steam id) or everyone (None) to a spot on the map.
#[tauri::command]
pub async fn playtest_teleport_to(state: State<'_, AppState>, x: f32, z: f32, player: Option<String>) -> CmdResult<u32> {
    let playtest = state.playtest.clone();
    tauri::async_runtime::spawn_blocking(move || playtest.teleport_to(x, z, player.as_deref())).await.map_err(|e| e.to_string())?
}

/// A map file from the map list (None: the procedural map, once a server made it).
fn map_file(state: &AppState, path: Option<String>) -> CmdResult<PathBuf> {
    match path {
        Some(path) => Ok(PathBuf::from(path)),
        None => {
            let dir = state.store.locked().server_dir().join("server").join(startfile::PROCGEN_IDENTITY);
            mapview::newest_map(&dir).ok_or_else(|| t!("error.procgenNotYet"))
        }
    }
}

/// A map's icon for the list (header + RGBA, see mapview::HEADER), made once per version.
#[tauri::command]
pub async fn map_thumbnail(state: State<'_, AppState>, path: Option<String>) -> CmdResult<tauri::ipc::Response> {
    let file = map_file(&state, path)?;
    let cache = state.store.locked().data.join("Thumbs");
    let bytes = tauri::async_runtime::spawn_blocking(move || mapview::thumbnail(&file, &cache)).await.map_err(|e| e.to_string())??;
    Ok(tauri::ipc::Response::new(bytes))
}

/// A map's full picture (the map viewer).
#[tauri::command]
pub async fn map_picture(state: State<'_, AppState>, path: Option<String>) -> CmdResult<tauri::ipc::Response> {
    let file = map_file(&state, path)?;
    let picture = tauri::async_runtime::spawn_blocking(move || mapview::picture(&file)).await.map_err(|e| e.to_string())??;
    Ok(tauri::ipc::Response::new(picture.to_bytes()))
}

/// The bug reports kept for a map and its monuments' names (the map viewer shows them).
#[tauri::command]
pub async fn map_notes(state: State<'_, AppState>, path: Option<String>) -> CmdResult<MapNotes> {
    Ok(state.playtest.notes_for(path.as_deref().map(Path::new)))
}

/// The map's picture: a 16-byte header (see mapview::HEADER) and RGBA pixels.
#[tauri::command]
pub async fn playtest_picture(state: State<'_, AppState>) -> CmdResult<tauri::ipc::Response> {
    let playtest = state.playtest.clone();
    let picture = tauri::async_runtime::spawn_blocking(move || playtest.picture()).await.map_err(|e| e.to_string())??;
    Ok(tauri::ipc::Response::new(picture.to_bytes()))
}

#[tauri::command]
pub async fn playtest_set_enabled(state: State<'_, AppState>, on: bool) -> CmdResult<PlaytestView> {
    let playtest = state.playtest.clone();
    tauri::async_runtime::spawn_blocking(move || playtest.set_enabled(on)).await.map_err(|e| e.to_string())?
}

/// Save the report page the UI made (one HTML file with everything in it). None: cancelled.
#[tauri::command]
pub async fn playtest_save_report(app: AppHandle, html: String, name: String) -> CmdResult<Option<String>> {
    let name: String = name.chars().map(|c| if r#"<>:"/\|?*"#.contains(c) || c.is_control() { '_' } else { c }).collect();
    let Some(path) = app
        .dialog()
        .file()
        .set_title(t!("playtest.export.dialog"))
        .set_file_name(format!("{name}.html"))
        .add_filter(t!("playtest.export.filter"), &["html"])
        .blocking_save_file()
        .and_then(|p| p.into_path().ok())
    else {
        return Ok(None);
    };
    std::fs::write(&path, html).map_err(|e| t!("error.save", file = path.display(), error = e))?;
    Ok(Some(path.display().to_string()))
}

// ---------------------------------------------------------------- kits

#[tauri::command]
pub async fn kits_load(state: State<'_, AppState>) -> CmdResult<Vec<kits::Kit>> {
    Ok(kits::load(&state.data_dir()))
}

/// The whole list as it is now (saved, made givable); what was saved.
#[tauri::command]
pub async fn kits_save(state: State<'_, AppState>, kits: Vec<kits::Kit>) -> CmdResult<Vec<kits::Kit>> {
    let kits: Vec<kits::Kit> = kits.into_iter().map(kits::clean).collect();
    kits::save(&state.data_dir(), &kits).map_err(|e| format!("{e:#}"))?;
    Ok(kits)
}

/// Rust's item list: from the running server when the DropLaunch plugin answers (and kept), else
/// the one it told last; None: the UI uses the list built into DropLaunch.
#[tauri::command]
pub async fn kit_items(state: State<'_, AppState>) -> CmdResult<Option<Vec<kits::CatalogItem>>> {
    let server = state.server.clone();
    let data = state.data_dir();
    tauri::async_runtime::spawn_blocking(move || {
        if let Some(items) = server.rcon_call("droplaunch.items", std::time::Duration::from_secs(6)).ok().and_then(|reply| kits::parse_items(&reply)) {
            let _ = kits::save_items(&data, &items);
            return Ok(Some(items));
        }
        Ok(kits::load_items(&data))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// An item's picture (PNG bytes), downloaded the first time and kept.
#[tauri::command]
pub async fn item_icon(state: State<'_, AppState>, item: String) -> CmdResult<tauri::ipc::Response> {
    let data = state.data_dir();
    tauri::async_runtime::spawn_blocking(move || kits::icon(&data, &item))
        .await
        .map_err(|e| e.to_string())?
        .map(tauri::ipc::Response::new)
        .map_err(|e| format!("{e:#}"))
}

/// Who's on the running server (Rust's own `playerlist`).
#[tauri::command]
pub async fn kit_players(state: State<'_, AppState>) -> CmdResult<Vec<kits::Player>> {
    let server = state.server.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let reply = server.rcon_call("playerlist", std::time::Duration::from_secs(4))?;
        kits::parse_playerlist(&reply).ok_or_else(|| t!("error.kits.noPlayerList", error = reply.trim().chars().take(120).collect::<String>()))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KitGiven {
    /// Players that got it.
    players: usize,
    /// Each item in its own slot (the DropLaunch plugin), or where Rust put it.
    exact: bool,
}

/// Give a kit to these players (Steam IDs) on the running server.
#[tauri::command]
pub async fn kit_give(state: State<'_, AppState>, kit: kits::Kit, players: Vec<String>) -> CmdResult<KitGiven> {
    let kit = kits::clean(kit);
    if kits::item_count(&kit) == 0 {
        return Err(t!("error.kits.empty"));
    }
    let players: Vec<String> = players.into_iter().filter(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())).collect();
    if players.is_empty() {
        return Err(t!("error.kits.noPlayers"));
    }
    let server = state.server.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let wait = std::time::Duration::from_secs(4);
        let spec = kits::spec(&kit);
        // A player it didn't work for doesn't stop the others; with nobody given, the last
        // problem is the answer (else: none of them was found).
        let mut problem = None;
        let none_given = |problem: Option<String>| Err(problem.unwrap_or_else(|| t!("error.kits.notFound")));
        // The plugin puts every item in its own slot; "DLKIT" says it's there.
        // (No answer or "not found": the plugin isn't loaded.)
        let first = server.rcon_call(&format!("droplaunch.givekit {} {spec}", players[0]), wait).unwrap_or_default();
        if first.trim_start().starts_with("DLKIT") {
            let mut given = usize::from(first.contains("DLKIT ok"));
            for id in &players[1..] {
                match server.rcon_call(&format!("droplaunch.givekit {id} {spec}"), wait) {
                    Ok(reply) if reply.contains("DLKIT ok") => given += 1,
                    Ok(_) => {}
                    Err(e) => problem = Some(e),
                }
            }
            if given == 0 {
                return none_given(problem);
            }
            return Ok(KitGiven { players: given, exact: true });
        }
        // Without it: Rust's own command, item by item (the first one answers whether the player
        // is there: "Couldn't find player!").
        let mut given = 0;
        for id in &players {
            let mut commands = kits::vanilla_commands(&kit, id).into_iter();
            let Some(first) = commands.next() else { continue };
            match server.rcon_call(&first, wait) {
                Ok(reply) if reply.contains("Couldn't find") => continue,
                Ok(_) => {}
                Err(e) => {
                    problem = Some(e);
                    continue;
                }
            }
            given += 1;
            for command in commands {
                server.rcon_send(&command);
            }
        }
        if given == 0 {
            return none_given(problem);
        }
        Ok(KitGiven { players: given, exact: false })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The kits that come with DropLaunch, by id.
#[tauri::command]
pub async fn kits_built_in() -> CmdResult<Vec<String>> {
    Ok(kits::built_in_ids())
}

// ---------------------------------------------------------------- dropped assets

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KitsImported {
    imported: Vec<kits::Kit>,
    /// The files that aren't kits (language files, most likely).
    rest: Vec<String>,
}

/// Kit files dropped on DropLaunch: added to the kits (the other files are handed back).
#[tauri::command]
pub async fn kits_import(state: State<'_, AppState>, paths: Vec<String>) -> CmdResult<KitsImported> {
    let data = state.data_dir();
    let folder = state.store.locked().config.folder.clone();
    let mut out = KitsImported { imported: Vec::new(), rest: Vec::new() };
    for path in paths {
        let file = Path::new(&path);
        if kits::read_kit(file).is_err() {
            out.rest.push(path);
            continue;
        }
        let kit = kits::import(&data, file).map_err(|e| format!("{e:#}"))?;
        let copy = kits::load_files(&data).into_iter().find(|(k, _)| k.id == kit.id).map(|(_, p)| p.display().to_string());
        assets::record(&data, &folder, assets::Kind::Kit, file, copy.into_iter().collect());
        out.imported.push(kit);
    }
    Ok(out)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetFile {
    path: String,
    name: String,
    /// When it was dropped (or last started, for maps from the history), seconds since 1970.
    added: i64,
    modified: i64,
    size: u64,
    exists: bool,
    /// A zip's maps; a language's or kit's own copy.
    made: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetLanguage {
    code: String,
    name: String,
    source: lang::Source,
    path: Option<String>,
    added: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetStartFile {
    path: String,
    name: String,
    /// The map it starts (None: the procedural map).
    map: Option<String>,
    map_name: String,
    prefab: bool,
    modified: i64,
    branch: String,
    /// DropLaunch's own file for its map (the one Start uses).
    own: bool,
    /// Its settings (the arguments that aren't about the map): files with the same ones are
    /// one start file in Settings, whatever map they start.
    settings: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetKit {
    id: String,
    /// Comes with DropLaunch.
    built_in: bool,
    name: String,
    path: String,
    items: usize,
    modified: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Assets {
    maps: Vec<AssetFile>,
    zips: Vec<AssetFile>,
    languages: Vec<AssetLanguage>,
    start_files: Vec<AssetStartFile>,
    kits: Vec<AssetKit>,
    kits_dir: String,
}

fn modified_secs(meta: &std::fs::Metadata) -> i64 {
    meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Everything dropped on DropLaunch by kind, with the start files and the kits (Settings).
#[tauri::command]
pub async fn assets_list(state: State<'_, AppState>) -> CmdResult<Assets> {
    let (data, base, maps_dir, history, installs, folder, legacy) = {
        let store = state.store.locked();
        (store.data.clone(), store.base.clone(), store.maps_dir(), store.config.history.clone(), store.installs(), store.config.folder.clone(), store.config.legacy_drops)
    };
    tauri::async_runtime::spawn_blocking(move || {
        // What was dropped on this DropLaunch folder (not on a copy somewhere else).
        let dropped = assets::load_for(&data, &folder, legacy);
        let file = |path: &str, added: i64, made: Vec<String>| {
            let p = Path::new(path);
            let meta = std::fs::metadata(p).ok();
            AssetFile {
                path: path.to_string(),
                name: p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string()),
                added,
                modified: meta.as_ref().map(modified_secs).unwrap_or(0),
                size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
                exists: meta.is_some(),
                made: made.into_iter().filter(|m| Path::new(m).is_file()).collect(),
            }
        };
        let of = |kind| dropped.iter().filter(move |d| d.kind == kind);
        let mut maps: Vec<AssetFile> = of(assets::Kind::Map).map(|d| file(&d.path, d.added, Vec::new())).collect();
        // Maps started from outside the Maps folder were dropped (or opened) too.
        for h in &history {
            let outside = !startfile::is_inside(Path::new(&h.path), &maps_dir);
            if outside && !maps.iter().any(|m| m.path.eq_ignore_ascii_case(&h.path)) {
                maps.push(file(&h.path, h.started, Vec::new()));
            }
        }
        maps.sort_by(|a, b| b.added.cmp(&a.added));
        let zips = of(assets::Kind::Zip).map(|d| file(&d.path, d.added, d.made.clone())).collect();
        let added_at = |path: &Option<PathBuf>| {
            path.as_ref()
                .and_then(|p| dropped.iter().find(|d| d.kind == assets::Kind::Language && d.made.iter().any(|m| Path::new(m) == p.as_path())))
                .map(|d| d.added)
                .unwrap_or(0)
        };
        let languages = lang::list(&base, &data)
            .into_iter()
            .map(|l| AssetLanguage { added: added_at(&l.file), code: l.code, name: l.name, source: l.source, path: l.file.map(|p| p.display().to_string()) })
            .collect();
        let start_files = installs
            .iter()
            .flat_map(|(dir, server)| startfile::list(dir).into_iter().map(move |sf| (sf, server.branch.clone(), dir.clone())))
            .map(|(sf, branch, dir)| AssetStartFile {
                // The file Start uses for its map (others, like an older start_rust_server.bat, aren't).
                own: {
                    let target = sf.map.clone().map_or(Target::Procgen, Target::Map);
                    startfile::path_for(&dir, &target, sf.prefab_mode) == sf.path
                },
                settings: std::fs::read_to_string(&sf.path).map(|text| startfile::settings_of(&text)).unwrap_or_default(),
                name: sf.file_name(),
                map_name: sf.map.as_deref().map(startfile::map_title).unwrap_or_else(|| t!("maps.procgen.name")),
                map: sf.map.as_ref().map(|p| p.display().to_string()),
                prefab: sf.prefab_mode,
                modified: std::fs::metadata(&sf.path).as_ref().map(modified_secs).unwrap_or(0),
                path: sf.path.display().to_string(),
                branch,
            })
            .collect();
        let kits = kits::load_files(&data)
            .into_iter()
            .map(|(k, p)| AssetKit {
                built_in: kits::built_in_ids().contains(&k.id),
                items: kits::item_count(&k),
                modified: std::fs::metadata(&p).as_ref().map(modified_secs).unwrap_or(0),
                id: k.id,
                name: k.name,
                path: p.display().to_string(),
            })
            .collect();
        Ok(Assets { maps, zips, languages, start_files, kits, kits_dir: data.join(kits::KITS_DIR).display().to_string() })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Off the dropped list (the file stays where it is).
#[tauri::command]
pub async fn assets_forget(state: State<'_, AppState>, path: String) -> CmdResult<()> {
    assets::forget(&state.data_dir(), &path);
    let mut store = state.store.locked();
    store.config.history.retain(|h| !h.path.eq_ignore_ascii_case(&path));
    save(&store)
}
