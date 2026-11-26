//! Add-ons: pages with scripts that someone made for DropLaunch (an RCON tool, a player list…).
//!
//! Each page runs in a sandboxed frame in the window (ui/addons.js), served from the add-on's
//! folder over the `addon` address (`serve`). The frame can't reach DropLaunch, Tauri or the PC:
//! only messages to the window, which pass on the calls in `commands::addon_call`, each checked
//! against the permissions its addon.json asks for (shown when it's added). Its pages get a strict
//! Content-Security-Policy too: scripts only from its own folder and the SDK, no connections
//! anywhere but its own folder (and the web addresses it lists, with the "network" permission).
//!
//! - A `.dladdon` file (a zip with addon.json) is unpacked into the data folder's Add-ons\<id>.
//! - A folder with addon.json (someone making an add-on) is used where it is: a change shows
//!   after Reload, nothing to pack first.
//! - What an add-on saves: Add-on data\<id>.json (`storage_*`).
//!
//! addons/README.md is the guide for making one; addons/_sdk/ the script and styles they include.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::sync::Locked;
use crate::t;

/// Unpacked add-ons, in the data folder.
pub const FOLDER: &str = "Add-ons";
/// What add-ons save, one file each.
pub const DATA_FOLDER: &str = "Add-on data";
pub const EXTENSION: &str = "dladdon";
/// The API this DropLaunch has (addon.json's "api" can't be newer).
pub const API_VERSION: u32 = 1;
const MANIFEST: &str = "addon.json";
/// Folders with an add-on being made, used where they are (absolute paths).
const LINKED_FILE: &str = "linked.json";
/// Add-ons that are turned off (ids).
const OFF_FILE: &str = "off.json";
/// A .dladdon: at most this many files and bytes unpacked.
const MAX_FILES: usize = 2000;
const MAX_BYTES: u64 = 64 * 1024 * 1024;
/// What one add-on may save.
const MAX_STORAGE: usize = 4 * 1024 * 1024;
/// One file served: at most this big.
const MAX_SERVED: u64 = 32 * 1024 * 1024;
/// The window's own address: the only page that may show an add-on's pages.
const APP_ORIGINS: &str = "http://tauri.localhost tauri://localhost";

/// Changes to the add-on list (installing, linking, turning on and off) one at a time.
static CHANGING: Mutex<()> = Mutex::new(());
/// One storage change at a time per add-on (each reads its file, changes it and writes it back):
/// a big save of one doesn't hold up the others.
static STORING: Mutex<std::collections::BTreeMap<String, std::sync::Arc<Mutex<()>>>> = Mutex::new(std::collections::BTreeMap::new());
/// DropLaunch was started with `--no-addons`: none is used for this run (the way out when one
/// misbehaves: Settings → Add-ons still lists them, to remove or turn off).
static PAUSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// No add-on is used for the rest of this run (`--no-addons`).
pub fn pause() {
    PAUSED.store(true, std::sync::atomic::Ordering::Relaxed);
}

fn paused() -> bool {
    PAUSED.load(std::sync::atomic::Ordering::Relaxed)
}

const SDK_JS: &str = include_str!("../../addons/_sdk/droplaunch.js");
const SDK_CSS: &str = include_str!("../../addons/_sdk/droplaunch.css");

/// What an add-on can ask for. Anything else in addon.json is refused (a typo shouldn't pass
/// silently, and a permission a newer DropLaunch knows means it needs that one).
pub const PERMISSIONS: [&str; 11] = ["server:read", "server:control", "console:read", "rcon", "kits", "maps:read", "reports:read", "reports:write", "configs:read", "configs:write", "network"];
/// Icons a tab or card may have (ui/index.html's symbols); anything else gets the puzzle piece.
const ICONS: [&str; 18] = [
    "terminal", "users", "wrench", "server", "map", "star", "shield", "bell", "sliders", "gift", "bug", "globe", "flask", "history", "search", "info", "puzzle", "bulb",
];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum UiKind {
    /// A page of its own: a tab next to Maps and Server files (More options).
    Tab,
    /// A card in More options' side column, under Bug reports.
    Card,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ui {
    pub kind: UiKind,
    pub title: String,
    /// The page in the add-on's folder ("index.html").
    pub page: String,
    #[serde(default)]
    pub icon: Option<String>,
    /// A card's height in pixels.
    #[serde(default)]
    pub height: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub homepage: Option<String>,
    /// The oldest DropLaunch it works with ("2.1.0").
    #[serde(default)]
    pub droplaunch: Option<String>,
    #[serde(default = "first_api")]
    pub api: u32,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub ui: Vec<Ui>,
    /// Web addresses its pages may connect to and show pictures from ("https://example.com"),
    /// with the "network" permission.
    #[serde(default)]
    pub network: Vec<String>,
    /// Sites its links open on without asking first ("steamcommunity.com").
    #[serde(default)]
    pub links: Vec<String>,
}

fn first_api() -> u32 {
    1
}

/// An add-on as Settings and the window know it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Addon {
    #[serde(flatten)]
    pub manifest: Manifest,
    /// Where its files are.
    pub folder: String,
    /// Used where it is (a folder being worked on), not unpacked into Add-ons.
    pub linked: bool,
    pub on: bool,
    /// Its pages' address: base + page.
    pub base: String,
    /// Why it can't be used (a broken or too new addon.json); it's listed so it can be removed.
    pub problem: Option<String>,
}

/// The `addon` address's root, as the window sees it.
pub fn base_url() -> &'static str {
    if cfg!(windows) {
        "http://addon.localhost/"
    } else {
        "addon://localhost/"
    }
}

// ---------------------------------------------------------------- addon.json
fn valid_id(id: &str) -> bool {
    (2..=40).contains(&id.len())
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-')
        && !id.ends_with('-')
}

/// A path inside the add-on's folder, written with forward slashes: no "..", no drive, no root.
fn valid_relative(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 200
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains(':')
        && path.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
}

fn version_parts(version: &str) -> Option<(u32, u32, u32)> {
    let mut parts = version.trim().trim_start_matches(">=").trim().split('.').map(|p| p.parse::<u32>().ok());
    Some((parts.next()??, parts.next().flatten().unwrap_or(0), parts.next().flatten().unwrap_or(0)))
}

/// "https://example.com" or "wss://example.com:8080": a scheme and a host, nothing after it.
fn valid_origin(origin: &str) -> bool {
    let Some(rest) = origin.strip_prefix("https://").or_else(|| origin.strip_prefix("wss://")) else { return false };
    let host = rest.split(':').next().unwrap_or("");
    let port_ok = rest.split_once(':').is_none_or(|(_, port)| port.parse::<u16>().is_ok());
    valid_host(host.strip_prefix("*.").unwrap_or(host)) && port_ok
}

fn valid_host(host: &str) -> bool {
    !host.is_empty() && host.len() <= 253 && host.contains('.') && host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
}

/// addon.json read and checked; what's off is said in plain words.
pub fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let mut m: Manifest = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| t!("addons.error.json", error = e))?;
    m.id = m.id.trim().to_string();
    m.name = m.name.trim().to_string();
    m.version = m.version.trim().to_string();
    if !valid_id(&m.id) {
        return Err(t!("addons.error.id", id = m.id));
    }
    if m.name.is_empty() || m.name.chars().count() > 60 {
        return Err(t!("addons.error.name"));
    }
    if m.version.is_empty() || m.version.len() > 20 {
        return Err(t!("addons.error.version"));
    }
    if m.api == 0 || m.api > API_VERSION {
        return Err(t!("addons.error.newer", name = m.name));
    }
    if let Some(needed) = m.droplaunch.as_deref().filter(|v| !v.trim().is_empty()) {
        let have = version_parts(env!("CARGO_PKG_VERSION"));
        match version_parts(needed) {
            Some(needed_parts) if have.is_some_and(|have| have >= needed_parts) => {}
            Some(_) => return Err(t!("addons.error.droplaunch", name = m.name, version = needed.trim_start_matches(">=").trim())),
            None => return Err(t!("addons.error.version")),
        }
    }
    m.author = m.author.trim().chars().take(60).collect();
    m.description = m.description.trim().chars().take(400).collect();
    if m.homepage.as_deref().is_some_and(|h| !h.starts_with("https://")) {
        m.homepage = None;
    }
    let mut permissions = Vec::new();
    for p in &m.permissions {
        if !PERMISSIONS.contains(&p.as_str()) {
            return Err(t!("addons.error.permission", permission = p));
        }
        if !permissions.contains(p) {
            permissions.push(p.clone());
        }
    }
    m.permissions = permissions;
    if m.ui.len() > 4 {
        return Err(t!("addons.error.ui"));
    }
    for ui in &mut m.ui {
        ui.title = ui.title.trim().chars().take(40).collect();
        if ui.title.is_empty() || !valid_relative(&ui.page) || !(ui.page.ends_with(".html") || ui.page.ends_with(".htm")) {
            return Err(t!("addons.error.ui"));
        }
        if ui.icon.as_deref().is_some_and(|i| !ICONS.contains(&i)) {
            ui.icon = None;
        }
        ui.height = Some(ui.height.unwrap_or(260).clamp(80, 900));
    }
    if m.network.len() > 20 || m.network.iter().any(|o| !valid_origin(o)) {
        return Err(t!("addons.error.network"));
    }
    if !m.network.is_empty() && !m.permissions.iter().any(|p| p == "network") {
        return Err(t!("addons.error.networkPermission"));
    }
    m.links.retain(|h| valid_host(h.trim_start_matches("*.")));
    m.links.truncate(20);
    Ok(m)
}

fn read_manifest(folder: &Path) -> Result<Manifest, String> {
    let text = std::fs::read_to_string(folder.join(MANIFEST)).map_err(|_| t!("addons.error.noManifest", path = folder.display()))?;
    parse_manifest(&text)
}

// ---------------------------------------------------------------- the list
fn root(data: &Path) -> PathBuf {
    data.join(FOLDER)
}

fn load_strings(file: &Path) -> Vec<String> {
    std::fs::read_to_string(file).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

fn save_strings(file: &Path, list: &[String]) -> Result<(), String> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(list).map_err(|e| e.to_string())?;
    let partial = file.with_extension("json.part");
    std::fs::write(&partial, text).map_err(|e| e.to_string())?;
    std::fs::rename(&partial, file).map_err(|e| e.to_string())
}

/// The list as last read: `find` (every call, every file a page loads) uses it instead of reading
/// every addon.json again. `list` reads it fresh (Settings, the window's refresh, Reload), and
/// every change here drops it.
static KNOWN: Mutex<Option<(PathBuf, Vec<Addon>)>> = Mutex::new(None);
/// Counts the changes: a list read while one was made isn't kept.
static CHANGES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Something in Add-ons changed: the next `find` reads the list again.
fn changed() {
    CHANGES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    *KNOWN.locked() = None;
}

/// Held while Add-ons is being changed: the list as last read is dropped when it starts and
/// again when it's done (also when it went wrong halfway).
struct Changing;

impl Changing {
    fn now() -> Self {
        changed();
        Changing
    }
}

impl Drop for Changing {
    fn drop(&mut self) {
        changed();
    }
}

/// Every add-on: unpacked ones, then folders being worked on (one with the same id as an
/// unpacked one wins), by name.
pub fn list(data: &Path) -> Vec<Addon> {
    let at = CHANGES.load(std::sync::atomic::Ordering::SeqCst);
    let all = read_list(data);
    let mut known = KNOWN.locked();
    if CHANGES.load(std::sync::atomic::Ordering::SeqCst) == at {
        *known = Some((data.to_path_buf(), all.clone()));
    }
    all
}

fn read_list(data: &Path) -> Vec<Addon> {
    let off = load_strings(&root(data).join(OFF_FILE));
    let entry = |folder: PathBuf, linked: bool, fallback_id: String| {
        let (manifest, problem) = match read_manifest(&folder) {
            Ok(m) if !linked && m.id != fallback_id => (empty_manifest(&fallback_id), Some(t!("addons.error.folderName", id = m.id))),
            Ok(m) => (m, None),
            Err(e) => (empty_manifest(&fallback_id), Some(e)),
        };
        // Started without add-ons: listed, with that as the reason none of them runs.
        let problem = problem.or_else(|| paused().then(|| t!("addons.error.paused")));
        let on = problem.is_none() && !off.contains(&manifest.id);
        Addon { base: format!("{}{}/", base_url(), manifest.id), manifest, folder: folder.display().to_string(), linked, on, problem }
    };
    let mut linked: Vec<Addon> = load_strings(&root(data).join(LINKED_FILE))
        .into_iter()
        .map(PathBuf::from)
        .map(|folder| {
            let name = folder.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            entry(folder, true, name)
        })
        .collect();
    let mut all: Vec<Addon> = std::fs::read_dir(root(data))
        .map(|dir| {
            dir.flatten()
                .filter(|e| e.path().is_dir())
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    (!name.starts_with('.')).then(|| entry(e.path(), false, name))
                })
                .filter(|a| !linked.iter().any(|l| l.manifest.id == a.manifest.id))
                .collect()
        })
        .unwrap_or_default();
    all.append(&mut linked);
    all.sort_by_key(|a| a.manifest.name.to_lowercase());
    all
}

fn empty_manifest(id: &str) -> Manifest {
    Manifest {
        id: id.to_string(),
        name: id.to_string(),
        version: String::new(),
        author: String::new(),
        description: String::new(),
        homepage: None,
        droplaunch: None,
        api: 1,
        permissions: Vec::new(),
        ui: Vec::new(),
        network: Vec::new(),
        links: Vec::new(),
    }
}

/// An add-on that's on and can be used (from the list as last read).
pub fn find(data: &Path, id: &str) -> Option<Addon> {
    let wanted = |a: &&Addon| a.manifest.id == id && a.on;
    let known = KNOWN.locked().as_ref().filter(|(dir, _)| dir == data).map(|(_, all)| all.iter().find(wanted).cloned());
    match known {
        Some(found) => found,
        None => list(data).iter().find(wanted).cloned(),
    }
}

/// Is `path` an add-on to add (a .dladdon file, or a folder with addon.json)?
pub fn is_addon(path: &Path) -> bool {
    if path.is_dir() {
        return path.join(MANIFEST).is_file();
    }
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION))
}

/// A .dladdon's addon.json and where in the zip the add-on is ("" or "name/": a zipped folder).
fn zip_manifest(file: &Path) -> Result<(Manifest, String), String> {
    let bad = || t!("addons.error.zip", file = file.display());
    let mut zip = zip::ZipArchive::new(std::fs::File::open(file).map_err(|e| e.to_string())?).map_err(|_| bad())?;
    if zip.len() > MAX_FILES {
        return Err(t!("addons.error.tooBig"));
    }
    let names: Vec<String> = zip.file_names().map(str::to_string).collect();
    let manifest = names
        .iter()
        .filter(|n| n.ends_with(MANIFEST) && (n.as_str() == MANIFEST || n.matches('/').count() == 1 && n.ends_with(&format!("/{MANIFEST}"))))
        .min_by_key(|n| n.len())
        .cloned()
        .ok_or_else(|| t!("addons.error.noManifest", path = file.display()))?;
    let prefix = manifest.trim_end_matches(MANIFEST).to_string();
    let mut text = String::new();
    zip.by_name(&manifest).map_err(|_| bad())?.take(1024 * 1024).read_to_string(&mut text).map_err(|_| bad())?;
    Ok((parse_manifest(&text)?, prefix))
}

/// What's being added, before it's added: the install dialog shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inspected {
    pub path: String,
    #[serde(flatten)]
    pub manifest: Manifest,
    /// A folder being worked on (used where it is).
    pub linked: bool,
    /// The version that's there now (an update), if any.
    pub replaces: Option<String>,
    /// The one that's there now (an update): the dialog marks what the new one adds, and says so
    /// when its name or maker isn't the same (it gets what the old one saved).
    pub before: Option<Before>,
}

/// An installed add-on, as an update dialog compares it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Before {
    pub name: String,
    pub author: String,
    pub permissions: Vec<String>,
    pub network: Vec<String>,
    pub links: Vec<String>,
}

pub fn inspect(data: &Path, path: &Path) -> Result<Inspected, String> {
    let (manifest, linked) = if path.is_dir() { (read_manifest(path)?, true) } else { (zip_manifest(path)?.0, false) };
    Ok(inspected(path, manifest, linked, &list(data)))
}

fn inspected(path: &Path, manifest: Manifest, linked: bool, all: &[Addon]) -> Inspected {
    let installed = all.iter().find(|a| a.manifest.id == manifest.id && a.problem.is_none()).map(|a| a.manifest.clone());
    let replaces = installed.as_ref().map(|m| m.version.clone());
    let before = installed.map(|m| Before { name: m.name, author: m.author, permissions: m.permissions, network: m.network, links: m.links });
    Inspected { path: path.display().to_string(), manifest, linked, replaces, before }
}

// ---------------------------------------------------------------- found, not added yet
/// .dladdon files hidden from the found list ("<path>|<size>|<modified>": the same file again, not
/// a new one with that name).
const HIDDEN_FILE: &str = "hidden.json";
/// At most this many .dladdon files looked at per folder.
const MAX_FOUND: usize = 50;

/// A .dladdon lying next to DropLaunch.exe or in the Add-ons folder, not added yet (or newer than
/// the one added): Settings → Add-ons lists it to add, with the same dialog as a dropped one.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    #[serde(flatten)]
    pub addon: Inspected,
    /// "app" (next to DropLaunch.exe) or "folder" (the Add-ons folder).
    pub place: &'static str,
}

/// A .dladdon there that can't be added, and why (a broken zip, made for a newer DropLaunch…).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Unreadable {
    pub path: String,
    pub place: &'static str,
    pub problem: String,
}

fn hidden_key(file: &Path) -> Option<String> {
    let meta = std::fs::metadata(file).ok()?;
    let modified = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    let path = file.display().to_string();
    let path = if cfg!(windows) { path.to_lowercase() } else { path };
    Some(format!("{path}|{}|{modified}", meta.len()))
}

/// Is `found` (from a file) worth offering over what's there: not there yet, or a newer version.
/// Versions that aren't numbers: offered when they differ.
fn worth_offering(found: &str, installed: &str) -> bool {
    match (version_parts(found), version_parts(installed)) {
        (Some(new), Some(old)) => new > old,
        _ => found != installed,
    }
}

/// The .dladdon files in `app_dir` (DropLaunch's folder) and Add-ons, not in subfolders, that
/// aren't added yet or are newer than the one added. One hidden with `hide` isn't listed (until
/// the file changes).
pub fn found(data: &Path, app_dir: &Path) -> (Vec<Found>, Vec<Unreadable>) {
    let all = list(data);
    let hidden = load_strings(&root(data).join(HIDDEN_FILE));
    let mut found = Vec::new();
    let mut unreadable = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();
    for (place, dir) in [("app", app_dir.to_path_buf()), ("folder", root(data))] {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_file() && is_addon(p)).collect();
        files.sort();
        files.truncate(MAX_FOUND);
        for file in files {
            let real = std::fs::canonicalize(&file).unwrap_or_else(|_| file.clone());
            if seen.contains(&real) {
                continue;
            }
            seen.push(real);
            if hidden_key(&file).is_some_and(|key| hidden.contains(&key)) {
                continue;
            }
            match zip_manifest(&file) {
                Ok((manifest, _)) => {
                    // One with this id that's broken (no version) can be replaced by this one.
                    let there = all.iter().find(|a| a.manifest.id == manifest.id && !a.manifest.version.is_empty());
                    if there.is_some_and(|a| !worth_offering(&manifest.version, &a.manifest.version)) {
                        continue;
                    }
                    found.push(Found { addon: inspected(&file, manifest, false, &all), place });
                }
                Err(problem) => {
                    // The row already says where it is: its name is enough in the reason.
                    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let problem = problem.replace(&file.display().to_string(), &name);
                    unreadable.push(Unreadable { path: file.display().to_string(), place, problem });
                }
            }
        }
    }
    (found, unreadable)
}

/// Not listed as found any more (until the file changes). Only a .dladdon file; the list keeps
/// the files that are still there.
pub fn hide(data: &Path, file: &Path) -> Result<(), String> {
    if !file.is_file() || !is_addon(file) {
        return Err(t!("addons.notAnAddon"));
    }
    let _one = CHANGING.locked();
    let path = root(data).join(HIDDEN_FILE);
    let mut hidden: Vec<String> = load_strings(&path).into_iter().filter(|key| key.split('|').next().is_some_and(|p| Path::new(p).is_file())).collect();
    let key = hidden_key(file).ok_or_else(|| t!("addons.notAnAddon"))?;
    if !hidden.contains(&key) {
        hidden.push(key);
    }
    let keep = hidden.len().saturating_sub(200);
    save_strings(&path, &hidden[keep..])
}

/// Add it (an update replaces the add-on with the same id; what it saved stays) and turn it on.
pub fn install(data: &Path, path: &Path) -> Result<Addon, String> {
    let _one = CHANGING.locked();
    let _changing = Changing::now();
    let root = root(data);
    std::fs::create_dir_all(&root).map_err(|e| t!("addons.error.write", error = e))?;
    let linked_file = root.join(LINKED_FILE);
    let mut linked = load_strings(&linked_file);
    let id = if path.is_dir() {
        let manifest = read_manifest(path)?;
        // A copy unpacked before goes: the folder is used from now on.
        let _ = std::fs::remove_dir_all(root.join(&manifest.id));
        let folder = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let folder = strip_verbatim(&folder);
        linked.retain(|l| read_manifest(Path::new(l)).map_or(true, |m| m.id != manifest.id) && !Path::new(l).eq(&folder));
        linked.push(folder.display().to_string());
        save_strings(&linked_file, &linked)?;
        manifest.id
    } else {
        let (manifest, prefix) = zip_manifest(path)?;
        let part = root.join(format!(".{}.part", manifest.id));
        let _ = std::fs::remove_dir_all(&part);
        unpack(path, &prefix, &part)?;
        let dest = root.join(&manifest.id);
        if dest.exists() {
            let old = root.join(format!(".{}.old", manifest.id));
            let _ = std::fs::remove_dir_all(&old);
            std::fs::rename(&dest, &old).map_err(|e| t!("addons.error.write", error = e))?;
            let _ = std::fs::remove_dir_all(&old);
        }
        std::fs::rename(&part, &dest).map_err(|e| t!("addons.error.write", error = e))?;
        // A folder with this add-on being worked on isn't used any more.
        let before = linked.len();
        linked.retain(|l| read_manifest(Path::new(l)).map_or(true, |m| m.id != manifest.id));
        if linked.len() != before {
            save_strings(&linked_file, &linked)?;
        }
        manifest.id
    };
    let off_file = root.join(OFF_FILE);
    let mut off = load_strings(&off_file);
    if off.contains(&id) {
        off.retain(|o| o != &id);
        save_strings(&off_file, &off)?;
    }
    list(data).into_iter().find(|a| a.manifest.id == id).ok_or_else(|| t!("addons.error.unknown", id = id))
}

/// `\\?\C:\…` (canonicalize's form on Windows) as `C:\…`.
fn strip_verbatim(path: &Path) -> PathBuf {
    let text = path.display().to_string();
    PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text))
}

/// The files under `prefix` in the zip into `dest`, checked: no paths out of it, not too many,
/// not too big.
fn unpack(file: &Path, prefix: &str, dest: &Path) -> Result<(), String> {
    let bad = || t!("addons.error.zip", file = file.display());
    let mut zip = zip::ZipArchive::new(std::fs::File::open(file).map_err(|e| e.to_string())?).map_err(|_| bad())?;
    std::fs::create_dir_all(dest).map_err(|e| t!("addons.error.write", error = e))?;
    let mut total = 0u64;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|_| bad())?;
        let Some(inner) = entry.enclosed_name() else { continue }; // refuses "../" paths
        let name = inner.to_string_lossy().replace('\\', "/");
        let Some(relative) = name.strip_prefix(prefix) else { continue };
        if relative.is_empty() || entry.is_dir() {
            continue;
        }
        if !valid_relative(relative) {
            continue;
        }
        total += entry.size();
        if total > MAX_BYTES {
            let _ = std::fs::remove_dir_all(dest);
            return Err(t!("addons.error.tooBig"));
        }
        let out = dest.join(relative);
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir).map_err(|e| t!("addons.error.write", error = e))?;
        }
        let mut target = std::fs::File::create(&out).map_err(|e| t!("addons.error.write", error = e))?;
        // At most what the zip says it is (a lying header doesn't fill the disk).
        let size = entry.size();
        std::io::copy(&mut (&mut entry).take(size), &mut target).map_err(|_| bad())?;
    }
    Ok(())
}

/// Taken out: its folder (or the link to a folder being worked on) and what it saved.
pub fn remove(data: &Path, id: &str) -> Result<(), String> {
    let _one = CHANGING.locked();
    let _changing = Changing::now();
    let addon = list(data).into_iter().find(|a| a.manifest.id == id).ok_or_else(|| t!("addons.error.unknown", id = id))?;
    let root = root(data);
    if addon.linked {
        let mut linked = load_strings(&root.join(LINKED_FILE));
        linked.retain(|l| !Path::new(l).eq(Path::new(&addon.folder)));
        save_strings(&root.join(LINKED_FILE), &linked)?;
    } else {
        // The folder that was listed, also one whose name isn't its add-on's id (unzipped there by
        // hand): only ever a folder right in Add-ons.
        let folder = PathBuf::from(&addon.folder);
        if folder.parent() != Some(root.as_path()) || folder.file_name().is_none_or(|n| n.to_string_lossy().starts_with('.')) {
            return Err(t!("addons.error.unknown", id = id));
        }
        std::fs::remove_dir_all(&folder).map_err(|e| t!("addons.error.write", error = e))?;
    }
    if valid_id(id) {
        let _ = std::fs::remove_file(storage_file(data, id));
    }
    let mut off = load_strings(&root.join(OFF_FILE));
    if off.contains(&id.to_string()) {
        off.retain(|o| o != id);
        save_strings(&root.join(OFF_FILE), &off)?;
    }
    Ok(())
}

pub fn set_on(data: &Path, id: &str, on: bool) -> Result<(), String> {
    let _one = CHANGING.locked();
    let _changing = Changing::now();
    let file = root(data).join(OFF_FILE);
    let mut off = load_strings(&file);
    off.retain(|o| o != id);
    if !on {
        off.push(id.to_string());
    }
    save_strings(&file, &off)
}

// ---------------------------------------------------------------- storage
fn storage_file(data: &Path, id: &str) -> PathBuf {
    data.join(DATA_FOLDER).join(format!("{id}.json"))
}

fn storage_load(data: &Path, id: &str) -> Map<String, Value> {
    std::fs::read_to_string(storage_file(data, id)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

pub fn storage_get(data: &Path, id: &str, key: &str) -> Value {
    storage_load(data, id).remove(key).unwrap_or(Value::Null)
}

pub fn storage_keys(data: &Path, id: &str) -> Vec<String> {
    storage_load(data, id).into_iter().map(|(k, _)| k).collect()
}

/// Saves `value` under `key` (null takes it out).
pub fn storage_set(data: &Path, id: &str, key: &str, value: Value) -> Result<(), String> {
    let mut one = Map::new();
    one.insert(key.to_string(), value);
    storage_set_many(data, id, one)
}

/// Saves every `key: value` in one write (null takes the key out); none of them when a key isn't
/// valid or it would be too much.
pub fn storage_set_many(data: &Path, id: &str, entries: Map<String, Value>) -> Result<(), String> {
    if entries.keys().any(|key| key.is_empty() || key.len() > 200) {
        return Err(t!("addons.error.storageKey"));
    }
    if entries.is_empty() {
        return Ok(());
    }
    let lock = STORING.locked().entry(id.to_string()).or_default().clone();
    let _one = lock.locked();
    let mut all = storage_load(data, id);
    for (key, value) in entries {
        if value.is_null() {
            all.remove(&key);
        } else {
            all.insert(key, value);
        }
    }
    let text = serde_json::to_string(&all).map_err(|e| e.to_string())?;
    if text.len() > MAX_STORAGE {
        return Err(t!("addons.error.storageFull"));
    }
    let file = storage_file(data, id);
    std::fs::create_dir_all(file.parent().unwrap_or(data)).map_err(|e| e.to_string())?;
    let partial = file.with_extension("json.part");
    std::fs::write(&partial, text).map_err(|e| e.to_string())?;
    std::fs::rename(&partial, &file).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- the addon address
/// What an add-on's pages may load and connect to.
fn content_policy(m: &Manifest) -> String {
    let base = base_url();
    let own = format!("{base}{}/", m.id);
    let sdk = format!("{base}_sdk/");
    let web = if m.permissions.iter().any(|p| p == "network") { m.network.join(" ") } else { String::new() };
    // "sandbox" first: the window's frames are sandboxed already (ui/addons.js), and this keeps an
    // add-on's page without an origin however it's loaded. Tauri takes every page of a registered
    // address (this one too) for the app's own, with every command: the missing origin is what
    // keeps an add-on to the calls it's allowed.
    format!(
        "sandbox allow-scripts allow-forms; default-src 'none'; script-src {own} {sdk}; style-src 'unsafe-inline' {own} {sdk}; img-src {own} data: blob: {web}; font-src {own} {sdk} data:; \
         media-src {own} data: blob:; connect-src {own} {web}; frame-src 'none'; worker-src 'none'; form-action 'none'; base-uri 'none'; frame-ancestors {APP_ORIGINS}"
    )
}

fn content_type(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        "txt" | "md" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// "%20" and the like back to what they stand for (None: not valid UTF-8 after that).
fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok())?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// A response to a page of the `addon` address: status, headers, body.
pub struct Served {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
}

/// GET http://addon.localhost/<id>/<file>: the file from an add-on that's on, with its policy;
/// /_sdk/droplaunch.js and .css: the SDK.
pub fn serve(data: &Path, method: &str, path: &str) -> Served {
    let plain = |status: u16, text: &str| Served {
        status,
        headers: vec![("Content-Type", "text/plain; charset=utf-8".into()), ("Content-Security-Policy", "default-src 'none'".into())],
        body: text.as_bytes().to_vec(),
    };
    if method != "GET" && method != "HEAD" {
        return plain(405, "Method not allowed");
    }
    let Some(path) = percent_decode(path) else { return plain(400, "Bad address") };
    let path = path.trim_start_matches('/');
    let (id, file) = path.split_once('/').unwrap_or((path, ""));
    let file = if file.is_empty() { "index.html" } else { file };
    let common = |kind: &str, policy: String| {
        vec![
            ("Content-Type", kind.to_string()),
            ("Content-Security-Policy", policy),
            // The frame's origin is "null" (sandboxed): its scripts as modules and its own files
            // through fetch need this.
            ("Access-Control-Allow-Origin", "*".to_string()),
            // The window's frames require a policy of every page in them (<iframe csp>, addons.js):
            // add-on pages take it; any other page the frame goes to (a website) is blocked.
            ("Allow-CSP-From", "*".to_string()),
            ("X-Content-Type-Options", "nosniff".to_string()),
            ("Referrer-Policy", "no-referrer".to_string()),
            ("Cache-Control", "no-store".to_string()),
        ]
    };
    if id == "_sdk" {
        let body = match file {
            "droplaunch.js" => SDK_JS,
            "droplaunch.css" => SDK_CSS,
            _ => return plain(404, "Not found"),
        };
        return Served { status: 200, headers: common(content_type(file), "default-src 'none'".into()), body: body.as_bytes().to_vec() };
    }
    if !valid_id(id) || !valid_relative(file) {
        return plain(404, "Not found");
    }
    // Turned off (or broken): nothing of it is served.
    let Some(addon) = find(data, id).filter(|a| a.on && a.problem.is_none()) else { return plain(404, "Not found") };
    let folder = PathBuf::from(&addon.folder);
    let wanted = folder.join(file);
    // Still inside its folder once links and the like are followed.
    let (Ok(folder), Ok(wanted)) = (std::fs::canonicalize(&folder), std::fs::canonicalize(&wanted)) else { return plain(404, "Not found") };
    if !wanted.starts_with(&folder) || !wanted.is_file() {
        return plain(404, "Not found");
    }
    if std::fs::metadata(&wanted).map(|m| m.len()).unwrap_or(u64::MAX) > MAX_SERVED {
        return plain(413, "Too big");
    }
    match std::fs::read(&wanted) {
        Ok(body) => Served { status: 200, headers: common(content_type(file), content_policy(&addon.manifest)), body: if method == "HEAD" { Vec::new() } else { body } },
        Err(_) => plain(404, "Not found"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("droplaunch-addons-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const GOOD: &str = r#"{"id":"my-addon","name":"My add-on","version":"1.0.0","permissions":["rcon","server:read"],"ui":[{"kind":"tab","title":"My tab","page":"index.html","icon":"terminal"}]}"#;

    #[test]
    fn manifests_are_checked() {
        let m = parse_manifest(GOOD).unwrap();
        assert_eq!(m.id, "my-addon");
        assert_eq!(m.ui[0].height, Some(260));
        assert!(parse_manifest(&GOOD.replace("my-addon", "Bad Id")).is_err());
        assert!(parse_manifest(&GOOD.replace("\"rcon\"", "\"files:write\"")).is_err());
        assert!(parse_manifest(&GOOD.replace("index.html", "../x.html")).is_err());
        assert!(parse_manifest(&GOOD.replace("index.html", "C:/x.html")).is_err());
        assert!(parse_manifest(&GOOD.replace("\"version\"", "\"api\":99,\"version\"")).is_err());
        assert!(parse_manifest(&GOOD.replace("\"version\"", "\"droplaunch\":\"99.0.0\",\"version\"")).is_err());
        // Web addresses need the network permission, and only origins count.
        let web = GOOD.replace("\"permissions\"", "\"network\":[\"https://api.example.com\"],\"permissions\"");
        assert!(parse_manifest(&web).is_err());
        assert!(parse_manifest(&web.replace("\"rcon\"", "\"rcon\",\"network\"")).is_ok());
        assert!(parse_manifest(&web.replace("https://api.example.com", "https://api.example.com/path").replace("\"rcon\"", "\"rcon\",\"network\"")).is_err());
    }

    fn zip_with(path: &Path, files: &[(&str, &str)]) {
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, text) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)).unwrap();
            std::io::Write::write_all(&mut zip, text.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn install_update_turn_off_remove() {
        let dir = temp("install");
        let data = dir.join("data");
        let file = dir.join("rcon.dladdon");
        zip_with(&file, &[("addon.json", GOOD), ("index.html", "<p>one</p>"), ("../evil.txt", "no")]);
        let added = install(&data, &file).unwrap();
        assert!(added.on && !added.linked);
        assert_eq!(std::fs::read_to_string(data.join(FOLDER).join("my-addon").join("index.html")).unwrap(), "<p>one</p>");
        assert!(!dir.join("evil.txt").exists() && !data.join("evil.txt").exists());
        storage_set(&data, "my-addon", "seen", serde_json::json!({"a": 1})).unwrap();
        // An update (a zipped folder this time) replaces the files, keeps what it saved.
        zip_with(&file, &[("rcon/addon.json", &GOOD.replace("1.0.0", "1.1.0")), ("rcon/index.html", "<p>two</p>")]);
        let update = inspect(&data, &file).unwrap();
        assert_eq!(update.replaces.as_deref(), Some("1.0.0"));
        // What the one there now may do, so the dialog can mark what's new (S11).
        let before = update.before.expect("an update says what it replaces");
        assert_eq!((before.name.as_str(), before.permissions.len()), ("My add-on", 2));
        assert_eq!(install(&data, &file).unwrap().manifest.version, "1.1.0");
        assert_eq!(std::fs::read_to_string(data.join(FOLDER).join("my-addon").join("index.html")).unwrap(), "<p>two</p>");
        assert_eq!(storage_get(&data, "my-addon", "seen"), serde_json::json!({"a": 1}));
        set_on(&data, "my-addon", false).unwrap();
        assert!(find(&data, "my-addon").is_none());
        assert_eq!(serve(&data, "GET", "/my-addon/index.html").status, 404);
        set_on(&data, "my-addon", true).unwrap();
        remove(&data, "my-addon").unwrap();
        assert!(list(&data).is_empty());
        assert_eq!(storage_get(&data, "my-addon", "seen"), Value::Null);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn files_lying_there_are_found_until_added_or_hidden() {
        let dir = temp("found");
        let data = dir.join("data");
        let app = dir.join("app");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::create_dir_all(data.join(FOLDER)).unwrap();
        let next_to_exe = app.join("my-addon-1.0.0.dladdon");
        zip_with(&next_to_exe, &[("addon.json", GOOD), ("index.html", "<p>one</p>")]);
        let in_folder = data.join(FOLDER).join("other.dladdon");
        zip_with(&in_folder, &[("addon.json", &GOOD.replace("my-addon", "other-tool").replace("My add-on", "Other"))]);
        std::fs::write(app.join("broken.dladdon"), b"not a zip").unwrap();
        // In a subfolder: not looked at.
        std::fs::create_dir_all(app.join("sub")).unwrap();
        zip_with(&app.join("sub").join("deep.dladdon"), &[("addon.json", &GOOD.replace("my-addon", "deep-tool"))]);

        let (found, unreadable) = super::found(&data, &app);
        let ids: Vec<(&str, &str)> = found.iter().map(|f| (f.addon.manifest.id.as_str(), f.place)).collect();
        assert_eq!(ids, [("my-addon", "app"), ("other-tool", "folder")]);
        assert!(found.iter().all(|f| f.addon.replaces.is_none()));
        assert_eq!(unreadable.len(), 1);
        assert!(unreadable[0].path.ends_with("broken.dladdon"));

        // Added: the same version isn't offered again, a newer one is (as an update), an older one not.
        install(&data, &next_to_exe).unwrap();
        let ids = |data: &Path| super::found(data, &app).0.into_iter().map(|f| (f.addon.manifest.id, f.addon.replaces)).collect::<Vec<_>>();
        assert_eq!(ids(&data), [("other-tool".to_string(), None)]);
        zip_with(&next_to_exe, &[("addon.json", &GOOD.replace("1.0.0", "1.2.0"))]);
        assert_eq!(ids(&data)[0], ("my-addon".to_string(), Some("1.0.0".to_string())));
        zip_with(&next_to_exe, &[("addon.json", &GOOD.replace("1.0.0", "0.9.0"))]);
        assert_eq!(ids(&data).len(), 1);

        // Hidden: gone from the list until the file changes.
        hide(&data, &in_folder).unwrap();
        hide(&data, &app.join("broken.dladdon")).unwrap();
        let (found, unreadable) = super::found(&data, &app);
        assert!(found.is_empty() && unreadable.is_empty());
        assert!(hide(&data, &app).is_err(), "only a .dladdon file");
        std::thread::sleep(std::time::Duration::from_millis(1100));
        zip_with(&in_folder, &[("addon.json", &GOOD.replace("my-addon", "other-tool").replace("1.0.0", "2.0.0"))]);
        assert_eq!(super::found(&data, &app).0.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn worth_offering_compares_versions() {
        assert!(worth_offering("1.10.0", "1.9.0"));
        assert!(!worth_offering("1.0", "1.0.0"));
        assert!(!worth_offering("1.0.0", "1.0.1"));
        assert!(worth_offering("beta", "alpha"));
        assert!(!worth_offering("beta", "beta"));
    }

    #[test]
    fn a_folder_is_used_where_it_is() {
        let dir = temp("linked");
        let data = dir.join("data");
        let work = dir.join("my-addon");
        std::fs::create_dir_all(work.join("lang")).unwrap();
        std::fs::write(work.join(MANIFEST), GOOD).unwrap();
        std::fs::write(work.join("index.html"), "<p>hi</p>").unwrap();
        std::fs::write(work.join("lang").join("en.json"), "{}").unwrap();
        std::fs::write(dir.join("secret.txt"), "no").unwrap();
        assert!(is_addon(&work));
        let added = install(&data, &work).unwrap();
        assert!(added.linked);
        let page = serve(&data, "GET", "/my-addon/index.html");
        assert_eq!((page.status, page.body.as_slice()), (200, b"<p>hi</p>".as_slice()));
        let policy = &page.headers.iter().find(|(k, _)| *k == "Content-Security-Policy").unwrap().1;
        assert!(policy.contains("connect-src http://addon.localhost/my-addon/ ;"), "{policy}");
        assert!(policy.contains("frame-ancestors http://tauri.localhost"));
        assert!(policy.starts_with("sandbox allow-scripts allow-forms;"), "no origin, however the page is loaded: {policy}");
        assert!(page.headers.iter().any(|(k, v)| *k == "Allow-CSP-From" && v == "*"), "takes the frame's policy");
        assert_eq!(serve(&data, "GET", "/my-addon/lang/en.json").status, 200);
        // Nothing outside its folder, however it's written.
        for path in ["/my-addon/../secret.txt", "/my-addon/%2e%2e/secret.txt", "/my-addon/..%5Csecret.txt", "/my-addon/C:/x", "/other/index.html"] {
            assert_eq!(serve(&data, "GET", path).status, 404, "{path}");
        }
        assert_eq!(serve(&data, "POST", "/my-addon/index.html").status, 405);
        assert_eq!(serve(&data, "GET", "/_sdk/droplaunch.js").status, 200);
        assert_eq!(serve(&data, "GET", "/_sdk/../my-addon/index.html").status, 404);
        assert_eq!(serve(&data, "GET", "/_sdk/other.js").status, 404);
        // Turned off: not served.
        set_on(&data, "my-addon", false).unwrap();
        assert_eq!(serve(&data, "GET", "/my-addon/index.html").status, 404);
        set_on(&data, "my-addon", true).unwrap();
        assert_eq!(serve(&data, "GET", "/my-addon/index.html").status, 200);
        remove(&data, "my-addon").unwrap();
        assert!(work.join("index.html").exists(), "a linked folder stays where it is");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Third sweep, D2: a folder unzipped by hand under another name is listed with a problem "so
    /// it can be removed", and Remove takes it away (it used to say so and leave it).
    #[test]
    fn a_folder_with_another_name_can_be_removed() {
        let data = temp("other-name");
        let folder = data.join(FOLDER).join("My Addon");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(MANIFEST), r#"{"id":"my-addon","name":"Mine","version":"1.0.0"}"#).unwrap();
        let listed = list(&data);
        assert_eq!(listed.len(), 1);
        assert!(listed[0].problem.is_some());
        remove(&data, &listed[0].manifest.id).unwrap();
        assert!(!folder.exists(), "the folder that was listed is gone");
        assert!(data.join(FOLDER).is_dir(), "and nothing else");
        assert!(remove(&data, "..").is_err());
        let _ = std::fs::remove_dir_all(&data);
    }

    /// An add-on folder being made next to the SDK (there may be none) is valid.
    #[test]
    fn addon_folders_next_to_the_sdk_are_valid() {
        let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../addons");
        let asked = regex::Regex::new(r#"\bt\("([^"]+)"|data-t(?:-title|-placeholder)?="([^"]+)""#).unwrap();
        for entry in std::fs::read_dir(&examples).unwrap().flatten() {
            let folder = entry.path();
            if !folder.join(MANIFEST).is_file() {
                continue;
            }
            let m = parse_manifest(&std::fs::read_to_string(folder.join(MANIFEST)).unwrap()).unwrap_or_else(|e| panic!("{}: {e}", folder.display()));
            assert_eq!(folder.file_name().unwrap().to_str(), Some(m.id.as_str()), "the folder is named like the id");
            for ui in &m.ui {
                assert!(folder.join(&ui.page).is_file(), "{}: {}", m.id, ui.page);
            }
            assert!(folder.join("lang").join("en.json").is_file(), "{}: lang/en.json", m.id);
            // Every text its pages and scripts ask for by name is in lang/en.json.
            let english: serde_json::Map<String, Value> = serde_json::from_str(&std::fs::read_to_string(folder.join("lang").join("en.json")).unwrap()).unwrap();
            for file in std::fs::read_dir(&folder).unwrap().flatten().map(|e| e.path()) {
                if !file.extension().is_some_and(|e| e == "js" || e == "html") {
                    continue;
                }
                let text = std::fs::read_to_string(&file).unwrap();
                for found in asked.captures_iter(&text) {
                    let key = found.get(1).or_else(|| found.get(2)).unwrap().as_str();
                    assert!(english.contains_key(key), "{}: \"{key}\" isn't in lang/en.json", file.display());
                }
            }
        }
    }

    #[test]
    fn storage_limits() {
        let dir = temp("storage");
        assert!(storage_set(&dir, "x1", "", Value::Bool(true)).is_err());
        storage_set(&dir, "x1", "k", Value::String("v".into())).unwrap();
        assert_eq!(storage_keys(&dir, "x1"), ["k"]);
        storage_set(&dir, "x1", "k", Value::Null).unwrap();
        assert!(storage_keys(&dir, "x1").is_empty());
        assert!(storage_set(&dir, "x1", "big", Value::String("x".repeat(MAX_STORAGE + 1))).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The SDK's gathered sets: all in one write, null takes a key out, and a bad key means none.
    #[test]
    fn several_sets_go_in_one_write() {
        let dir = temp("storage-many");
        storage_set(&dir, "x1", "old", Value::Bool(true)).unwrap();
        let entries = |pairs: &[(&str, Value)]| pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect::<Map<String, Value>>();
        storage_set_many(&dir, "x1", entries(&[("a", 1.into()), ("b", "two".into()), ("old", Value::Null)])).unwrap();
        let mut keys = storage_keys(&dir, "x1");
        keys.sort();
        assert_eq!(keys, ["a", "b"]);
        assert!(storage_set_many(&dir, "x1", entries(&[("c", 3.into()), ("", 4.into())])).is_err());
        assert_eq!(storage_get(&dir, "x1", "c"), Value::Null);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
