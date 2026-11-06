//! Start files: DropLaunch writes a .bat next to RustDedicated.exe for every server it starts
//! (the procedural map; two per custom map: a normal one and one in prefab test mode) plus
//! `update_server.bat` (and `update_rust_server.bat` in the SteamCMD folder, for every server
//! folder), so everything keeps working without DropLaunch. Before launching it
//! reads the file back: the file is the source of truth, so edits to ports, password, max
//! players etc. are respected. A file is only rewritten when the map moved, it was made by an
//! older DropLaunch, or the start file of yours it takes its settings from changed.
//!
//! Start files dropped on DropLaunch are kept (a copy in the data folder): one can be chosen for
//! the procedural map and one for custom maps, and DropLaunch's own files then take its settings
//! (the procedural map keeps its seed and size, a custom map its map). A procedural map's start
//! file is also in the map list: started from there it gets a file of its own with all its
//! settings.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use crate::steamcmd::{install, RUST_SERVER_APP_ID};

#[cfg(windows)]
const EXT: &str = "bat";
#[cfg(not(windows))]
const EXT: &str = "sh";

pub const PROCGEN_KEY: &str = "procgen_server";
/// A procedural map start file's own file is `start_procgen_<file>.bat`.
const PRESET_PREFIX: &str = "procgen_";
/// A map's prefab test mode file is `start_<map>_prefab.bat`.
const PREFAB_SUFFIX: &str = "_prefab";
pub const DEFAULT_PORT: u16 = 28015;
pub const DEFAULT_RCON_PORT: u16 = 28016;
pub const SEED: u32 = 1184645116;
pub const WORLD_SIZE: u32 = 3000;
pub const MAX_PLAYERS: u32 = 10;
/// Set by DropLaunch when it starts a file, so the window closes instead of waiting for a key.
pub const LAUNCH_ENV: &str = "DROPLAUNCH_SERVER";
/// In every file this DropLaunch writes; DropLaunch files without it are from an older version.
const TEMPLATE_MARK: &str = "Made with DropLaunch.gg";
/// Preview versions let DropLaunch hand the map's URL over in this variable: their files are
/// rewritten once.
const RETIRED_LEVELURL_ENV: &str = "DROPLAUNCH_LEVELURL";
/// Start files dropped on DropLaunch: a copy of each is kept in this folder of the data folder,
/// as text (`start_server.bat.txt`). Only their settings are used, and an app that writes scripts
/// to its data folder looks like malware to virus scanners.
pub const DROPPED_DIR: &str = "StartFiles";
const DROPPED_EXT: &str = ".txt";
/// Bigger files aren't start files.
const MAX_SCRIPT: u64 = 256 * 1024;
/// In a file that takes the settings of one of yours: `rem Uses the settings of <file>, your start file.`
const CHOSEN_MARK: &str = "Uses the settings of";
const CHOSEN_END: &str = ", your start file.";

/// What a start file runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Procgen,
    Map(PathBuf),
    /// A procedural map's start file dropped on DropLaunch (its name, e.g. start_server.bat): in the
    /// map list, it starts with all its settings, its own seed and size too.
    Preset(String),
}

impl Target {
    pub fn title(&self) -> String {
        match self {
            Target::Procgen => crate::t!("maps.procgen.name"),
            Target::Map(path) => map_title(path),
            Target::Preset(name) => map_title(Path::new(name)),
        }
    }
}

pub fn map_title(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "map".into())
}

/// Everything DropLaunch needs to know about a start file.
#[derive(Debug, Clone, PartialEq)]
pub struct StartFile {
    pub path: PathBuf,
    /// None for the procedural map.
    pub map: Option<PathBuf>,
    pub identity: String,
    pub server_port: u16,
    pub rcon_port: u16,
    pub rcon_password: Option<String>,
    pub log_file: Option<PathBuf>,
    /// Has the prefab test mode arguments.
    pub prefab_mode: bool,
    /// Written by this version of DropLaunch.
    pub current_template: bool,
    /// The start file of yours it took its settings from (its name).
    pub chosen: Option<String>,
}

impl StartFile {
    pub fn connect_command(&self) -> String {
        format!("client.connect 127.0.0.1:{}", self.server_port)
    }

    pub fn file_name(&self) -> String {
        self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }
}

/// Where DropLaunch's folders are, for writing paths the scripts can resolve themselves.
pub struct Layout<'a> {
    pub base: &'a Path,
    pub server_dir: &'a Path,
    pub steamcmd_dir: &'a Path,
}

// ---------------------------------------------------------------- names

/// Safe for a file name and a server identity: letters, digits, '-' and '_'.
pub fn sanitize(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '-' {
            out.push(c);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let out: String = out.trim_matches('_').chars().take(48).collect();
    if out.is_empty() {
        "map".into()
    } else {
        out
    }
}

pub fn key_for(target: &Target) -> String {
    match target {
        Target::Procgen => PROCGEN_KEY.into(),
        Target::Map(path) => {
            let key = sanitize(&map_title(path));
            let lower = key.to_ascii_lowercase();
            // Never collide with the procedural map's files, or with another map's prefab file.
            if lower.starts_with(PRESET_PREFIX) || lower.ends_with(PREFAB_SUFFIX) {
                format!("{key}_map")
            } else {
                key
            }
        }
        Target::Preset(name) => {
            let key = format!("{PRESET_PREFIX}{}", sanitize(&map_title(Path::new(name))));
            let lower = key.to_ascii_lowercase();
            if lower == PROCGEN_KEY || lower.ends_with(PREFAB_SUFFIX) {
                format!("{key}_file")
            } else {
                key
            }
        }
    }
}

/// Only custom maps have a prefab test mode file.
fn is_prefab(target: &Target, prefab: bool) -> bool {
    prefab && matches!(target, Target::Map(_))
}

/// `start_<map>.bat`, or `start_<map>_prefab.bat` for the map's prefab test mode.
pub fn path_for(server_dir: &Path, target: &Target, prefab: bool) -> PathBuf {
    let suffix = if is_prefab(target, prefab) { PREFAB_SUFFIX } else { "" };
    server_dir.join(format!("start_{}{suffix}.{EXT}", key_for(target)))
}

/// Is `path` the prefab test mode file of `target`?
pub fn is_prefab_file(server_dir: &Path, target: &Target, path: &Path) -> bool {
    matches!(target, Target::Map(_)) && same_path(path, &path_for(server_dir, target, true))
}

/// The procedural map's server identity (its world, and the map file Rust makes, are in
/// server/<identity>).
pub const PROCGEN_IDENTITY: &str = "droplaunch_procgen";

/// Both files of a map share the identity (one server folder, one fresh world per start).
fn identity_for(target: &Target) -> String {
    match target {
        Target::Procgen => PROCGEN_IDENTITY.into(),
        Target::Map(_) | Target::Preset(_) => format!("droplaunch_{}", key_for(target)),
    }
}

/// ...but have their own log, so DropLaunch can tell which one a running server came from.
fn log_name(target: &Target, prefab: bool) -> String {
    let identity = identity_for(target);
    if is_prefab(target, prefab) {
        format!("{identity}{PREFAB_SUFFIX}")
    } else {
        identity
    }
}

// ---------------------------------------------------------------- paths

/// Lexically clean a path (drop `.`, resolve `..`) without touching the disk.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

pub fn same_path(a: &Path, b: &Path) -> bool {
    let (a, b) = (normalize(a), normalize(b));
    if cfg!(windows) {
        a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy())
    } else {
        a == b
    }
}

pub fn is_inside(path: &Path, dir: &Path) -> bool {
    let (path, dir) = (normalize(path), normalize(dir));
    let mut p = path.components();
    for d in dir.components() {
        match p.next() {
            Some(c) if eq_component(c.as_os_str(), d.as_os_str()) => {}
            _ => return false,
        }
    }
    true
}

fn eq_component(a: &std::ffi::OsStr, b: &std::ffi::OsStr) -> bool {
    if cfg!(windows) {
        a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy())
    } else {
        a == b
    }
}

/// `to` relative to the folder `from` (both absolute, same drive), e.g. `..\Maps\x.map`.
fn relative(from: &Path, to: &Path) -> Option<PathBuf> {
    let (from, to) = (normalize(from), normalize(to));
    let from: Vec<_> = from.components().collect();
    let to: Vec<_> = to.components().collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| eq_component(a.as_os_str(), b.as_os_str())).count();
    if common == 0 {
        return None;
    }
    let mut out = PathBuf::new();
    for _ in common..from.len() {
        out.push("..");
    }
    for comp in &to[common..] {
        out.push(comp.as_os_str());
    }
    Some(out)
}

/// How a script refers to `path`: relative to its own folder when both live in the DropLaunch
/// folder (the folder can then be moved or shared), absolute otherwise. Escaped for the script.
fn script_path(layout: &Layout, path: &Path) -> String {
    script_path_from(layout.base, layout.server_dir, path)
}

/// How a script in the folder `from` refers to `path` (see script_path).
fn script_path_from(base: &Path, from: &Path, path: &Path) -> String {
    let portable = is_inside(path, base) && is_inside(from, base);
    match relative(from, path).filter(|_| portable) {
        #[cfg(windows)]
        Some(rel) => format!("%~dp0{}", bat_escape(&rel.display().to_string())),
        #[cfg(not(windows))]
        Some(rel) => format!("$HERE/{}", rel.display()),
        #[cfg(windows)]
        None => bat_escape(&normalize(path).display().to_string()),
        #[cfg(not(windows))]
        None => normalize(path).display().to_string(),
    }
}

// ---------------------------------------------------------------- writing

/// v1's "prefab start file": quiets everything that gets in the way of testing unfinished maps.
const PREFAB_ARGS: [&str; 5] = [
    "+cargoship.event_enabled false",
    "+baseboat.generate_paths false",
    "+ai.ocean_patrol_path_iterations 0",
    "+server.events false",
    "+antihack.terrain_protection 0",
];

/// Arguments DropLaunch sets itself: a start file that came with a map can't change these (the
/// map, the name, the ports and RCON DropLaunch talks to, the log it reads).
const MANAGED_ARGS: [&str; 15] = [
    "server.port",
    "server.level",
    "server.seed",
    "server.worldsize",
    "server.levelurl",
    "server.identity",
    "server.hostname",
    "server.description",
    "server.queryport",
    "app.port",
    "rcon.ip",
    "rcon.port",
    "rcon.password",
    "rcon.web",
    "logfile",
];

/// Arguments that only say which map and what it's called: start files that differ only in
/// these have the same settings.
const MAP_ARGS: [&str; 5] = ["server.levelurl", "server.identity", "server.hostname", "server.description", "rcon.password"];

/// Every `+name value` argument in a start file, in order.
pub fn args_of(text: &str) -> Vec<(String, String)> {
    static ARG: OnceLock<Regex> = OnceLock::new();
    re(&ARG, r#"(?:^|\s)\+([A-Za-z][A-Za-z0-9_.]*)[ \t]+(?:"([^"\r\n]*)"|([^\s"^]+))"#)
        .captures_iter(text)
        .filter_map(|c| {
            let name = c.get(1)?.as_str().to_ascii_lowercase();
            let value = c.get(2).or_else(|| c.get(3))?.as_str().to_string();
            Some((name, value))
        })
        .collect()
}

/// A start file's settings: its arguments without the map's own (sorted, so files with the same
/// settings compare equal whatever map they start).
pub fn settings_of(text: &str) -> Vec<String> {
    let mut out: Vec<String> = args_of(text)
        .into_iter()
        .filter(|(name, _)| !MAP_ARGS.contains(&name.as_str()))
        .map(|(name, value)| format!("+{name} {value}"))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// A start file that came with a map (in the map's folder, like one unpacked from the same .zip):
/// the settings it adds to DropLaunch's start files for that map.
#[derive(Debug, Clone, PartialEq)]
pub struct Bundled {
    pub path: PathBuf,
    pub args: Vec<(String, String)>,
}

/// Safe to write into a start file: no quotes, no variables, nothing the shell would run.
fn plain_value(value: &str) -> bool {
    value.len() <= 200 && !value.chars().any(|c| matches!(c, '"' | '%' | '$' | '`' | '^' | '&' | '|' | '<' | '>' | '\r' | '\n' | '!'))
}

/// The start file next to `map` that isn't DropLaunch's own: one named after the map, else the
/// only one there. Its arguments DropLaunch doesn't set itself, and only plain ones.
pub fn bundled(map: &Path) -> Option<Bundled> {
    let dir = map.parent()?;
    let stem = map_title(map).to_ascii_lowercase();
    let candidates: Vec<(PathBuf, String)> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("bat") || e.eq_ignore_ascii_case("sh")))
        .filter(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.len() < 256 * 1024))
        .filter_map(|p| std::fs::read_to_string(&p).ok().map(|text| (p, text)))
        .filter(|(_, text)| text.contains("RustDedicated") && !text.contains(TEMPLATE_MARK))
        .collect();
    let named = candidates.iter().find(|(p, _)| map_title(p).to_ascii_lowercase().contains(&stem));
    let (path, text) = named.or_else(|| (candidates.len() == 1).then(|| &candidates[0]))?;
    let mut args: Vec<(String, String)> = Vec::new();
    for (name, value) in args_of(text) {
        if MANAGED_ARGS.contains(&name.as_str()) || !plain_value(&value) {
            continue;
        }
        args.retain(|(n, _)| *n != name);
        args.push((name, value));
    }
    (!args.is_empty()).then(|| Bundled { path: path.clone(), args })
}

/// What a start file of yours is for: custom maps (it loads a map with server.levelurl), or the
/// procedural map (any other).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Use {
    Procgen,
    Maps,
}

/// Arguments DropLaunch sets itself also with a start file of yours: which map, its world folder,
/// RCON as DropLaunch talks to it, and the log it reads.
const OWN_ARGS: [&str; 6] = ["server.levelurl", "server.identity", "rcon.ip", "rcon.password", "rcon.web", "logfile"];
/// ...and which procedural map: the procedural map keeps its own (only a start file started from
/// the map list brings its own).
const PROCGEN_ARGS: [&str; 3] = ["server.level", "server.seed", "server.worldsize"];

/// A start file's text (a batch file needn't be UTF-8: what isn't is replaced). None: it can't be
/// read, or it's too big to be one.
pub fn read_script(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_SCRIPT {
        return None;
    }
    std::fs::read(path).ok().map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

/// A .bat, .cmd or .sh file (it may be a start file).
pub fn is_script(path: &Path) -> bool {
    path.extension().is_some_and(|e| ["bat", "cmd", "sh"].iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// What a Rust server start file is for. None: it isn't one (it doesn't run RustDedicated with
/// server settings, like update_server.bat).
pub fn use_of(text: &str) -> Option<Use> {
    let args = args_of(text);
    if !text.to_ascii_lowercase().contains("rustdedicated") || !args.iter().any(|(name, _)| name.starts_with("server.")) {
        return None;
    }
    Some(if args.iter().any(|(name, _)| name == "server.levelurl") { Use::Maps } else { Use::Procgen })
}

/// The settings DropLaunch's start files take from a start file of yours: its plain arguments but
/// DropLaunch's own and the procedural map's level, seed and size (the last one counts when one is
/// there twice). `with_map`: those three too (a start file started from the map list).
pub fn settings_for(text: &str, with_map: bool) -> Vec<(String, String)> {
    let mut args: Vec<(String, String)> = Vec::new();
    for (name, value) in args_of(text) {
        if OWN_ARGS.contains(&name.as_str()) || (!with_map && PROCGEN_ARGS.contains(&name.as_str())) || !plain_value(&value) {
            continue;
        }
        args.retain(|(n, _)| *n != name);
        args.push((name, value));
    }
    args
}

/// A start file of yours, chosen for the procedural map or for custom maps (DropLaunch's own start
/// files for those take its settings), or started from the map list.
#[derive(Debug, Clone, PartialEq)]
pub struct Chosen {
    /// DropLaunch's copy of it (in the data folder's StartFiles).
    pub path: PathBuf,
    pub args: Vec<(String, String)>,
}

impl Chosen {
    /// None: it's gone, or it isn't a start file for `usage`.
    pub fn read(path: &Path, usage: Use) -> Option<Self> {
        let text = read_script(path)?;
        (use_of(&text)? == usage).then(|| Chosen { path: path.to_path_buf(), args: settings_for(&text, false) })
    }

    /// A procedural map's start file started from the map list: all its settings, its level, seed
    /// and size too.
    pub fn preset(path: &Path) -> Option<Self> {
        let text = read_script(path)?;
        (use_of(&text)? == Use::Procgen).then(|| Chosen { path: path.to_path_buf(), args: settings_for(&text, true) })
    }

    /// The value of `+name` in its settings.
    pub fn value(&self, name: &str) -> Option<&str> {
        self.args.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    pub fn file_name(&self) -> String {
        dropped_name(&self.path).unwrap_or_else(|| self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
    }
}

/// A start file dropped on DropLaunch that it kept a copy of.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Kept {
    /// Its copy's name.
    pub name: String,
    pub usage: Use,
    /// It's the one chosen for `usage` already (dropped again).
    pub in_use: bool,
}

/// DropLaunch's copy of the dropped start file `name` (e.g. start_server.bat).
pub fn dropped_copy(data: &Path, name: &str) -> PathBuf {
    data.join(DROPPED_DIR).join(format!("{name}{DROPPED_EXT}"))
}

/// The name of the start file `copy` is DropLaunch's copy of (None: it isn't one).
pub fn dropped_name(copy: &Path) -> Option<String> {
    let name = copy.file_name()?.to_str()?.strip_suffix(DROPPED_EXT)?;
    is_script(Path::new(name)).then(|| name.to_string())
}

/// Keep a copy of a start file dropped on DropLaunch: the copy, and what it's for. Its name is
/// made safe (only letters, digits, '-' and '_'): the same name dropped again replaces it.
pub fn add_dropped(data: &Path, file: &Path) -> Result<(PathBuf, Use), String> {
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| file.display().to_string());
    let text = read_script(file).ok_or_else(|| crate::t!("startFiles.cantRead", name))?;
    keep_dropped(data, &name, &text)
}

/// The same for a start file `name` that came in a .zip (its text).
pub fn keep_dropped(data: &Path, name: &str, text: &str) -> Result<(PathBuf, Use), String> {
    let usage = use_of(text).ok_or_else(|| crate::t!("startFiles.notStartFile", name))?;
    let file = Path::new(name);
    let ext = file.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_else(|| EXT.into());
    let copy = dropped_copy(data, &format!("{}.{ext}", sanitize(&map_title(file))));
    // Written, not copied: the copy is newer than the start files made with the last one, so
    // they're made again.
    std::fs::create_dir_all(data.join(DROPPED_DIR))
        .and_then(|_| std::fs::write(&copy, text))
        .map_err(|e| crate::t!("startFiles.cantKeep", name, error = e))?;
    Ok((copy, usage))
}

/// A procedural map's start file: no custom map, and a level, seed or size of its own (one that
/// only sets players or the like, in a .zip with a map, is that map's).
pub fn makes_its_own_map(text: &str) -> bool {
    use_of(text) == Some(Use::Procgen) && args_of(text).iter().any(|(name, _)| PROCGEN_ARGS.contains(&name.as_str()))
}

fn server_args(target: &Target, identity: &str, password: &str, prefab: bool, extras: &[(String, String)]) -> Vec<String> {
    let log = log_name(target, prefab);
    let mut args = vec![format!("+server.port {DEFAULT_PORT}")];
    match target {
        Target::Procgen | Target::Preset(_) => {
            args.push("+server.level \"Procedural Map\"".into());
            args.push(format!("+server.seed {SEED}"));
            args.push(format!("+server.worldsize {WORLD_SIZE}"));
        }
        Target::Map(_) => {
            #[cfg(windows)]
            args.push("+server.levelurl \"%LEVEL_URL%\"".into());
            #[cfg(not(windows))]
            args.push("+server.levelurl \"$LEVEL_URL\"".into());
        }
    }
    args.push(format!("+server.maxplayers {MAX_PLAYERS}"));
    let (hostname, description) = match target {
        Target::Procgen | Target::Preset(_) => ("DropLaunch test server".to_string(), "Local test server, made with DropLaunch.gg"),
        Target::Map(_) => {
            let mode = if prefab { " (prefab test mode)" } else { "" };
            (format!("DropLaunch - {}{mode}", target.title().replace('"', "'")), "Map test server, made with DropLaunch.gg")
        }
    };
    #[cfg(windows)]
    let hostname = bat_escape(&hostname);
    args.push(format!("+server.hostname \"{hostname}\""));
    args.push(format!("+server.description \"{description}\""));
    args.push(format!("+server.identity \"{identity}\""));
    // The settings of a start file of yours, or one that came with the map: in place of
    // DropLaunch's own value, or added (prefab test mode's arguments stay as they are).
    for (name, value) in extras {
        if prefab && PREFAB_ARGS.iter().any(|a| a.split(' ').next() == Some(&format!("+{name}"))) {
            continue;
        }
        let arg = if value.is_empty() || value.contains(char::is_whitespace) { format!("+{name} \"{value}\"") } else { format!("+{name} {value}") };
        match args.iter_mut().find(|a| a.split(' ').next().is_some_and(|n| n.eq_ignore_ascii_case(&format!("+{name}")))) {
            Some(existing) => *existing = arg,
            None => args.push(arg),
        }
    }
    if prefab {
        args.extend(PREFAB_ARGS.map(String::from));
    }
    args.push("+rcon.ip 127.0.0.1".into());
    args.push(format!("+rcon.port {DEFAULT_RCON_PORT}"));
    args.push(format!("+rcon.password \"{password}\""));
    args.push("+rcon.web 1".into());
    #[cfg(windows)]
    args.push(format!("-logfile \"logs\\{log}.log\""));
    #[cfg(not(windows))]
    args.push(format!("-logfile \"logs/{log}.log\""));
    args
}

/// The settings that go in besides DropLaunch's own: yours, then the map's own start file's
/// (a map knows best what it needs).
fn extras(chosen: Option<&Chosen>, bundled: Option<&Bundled>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = chosen.map(|c| c.args.clone()).unwrap_or_default();
    for (name, value) in bundled.map_or(&[][..], |b| &b.args[..]) {
        out.retain(|(n, _)| n != name);
        out.push((name.clone(), value.clone()));
    }
    out
}

/// The value of `+name` in what server_args made.
#[cfg(windows)]
fn arg_in<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().find_map(|a| {
        let (arg, value) = a.split_once(' ')?;
        arg.strip_prefix('+').is_some_and(|n| n.eq_ignore_ascii_case(name)).then(|| value.trim_matches('"'))
    })
}

/// Batch files expand %: double it in anything we write literally.
#[cfg(windows)]
fn bat_escape(text: &str) -> String {
    text.replace('%', "%%")
}

/// For unquoted batch text (title, echo): also escape the command characters.
#[cfg(windows)]
fn bat_escape_bare(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '%' => out.push_str("%%"),
            '^' | '&' | '|' | '<' | '>' => {
                out.push('^');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

#[cfg(windows)]
fn contents(layout: &Layout, target: &Target, prefab: bool, password: &str, bundled: Option<&Bundled>, chosen: Option<&Chosen>) -> String {
    let identity = identity_for(target);
    let title = bat_escape_bare(&target.title());
    let prefab = is_prefab(target, prefab);
    let args = server_args(target, &identity, password, prefab, &extras(chosen, bundled));
    let value = |name| bat_escape_bare(arg_in(&args, name).unwrap_or_default());
    let (seed, size, port) = (value("server.seed"), value("server.worldsize"), value("server.port"));
    let other = |prefab| path_for(layout.server_dir, target, prefab).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut lines: Vec<String> = vec!["@echo off".into()];
    match target {
        Target::Procgen | Target::Preset(_) => {
            lines.push(format!("rem Start file for a procedural map (seed {seed}, size {size}). {TEMPLATE_MARK}"));
            lines.push("rem Double-click it to start the server, DropLaunch isn't needed. Edits you make are kept.".into());
        }
        Target::Map(_) if prefab => {
            lines.push(format!("rem Start file for {title} in prefab test mode. {TEMPLATE_MARK}"));
            lines.push("rem Prefab test mode turns off the cargo ship, boat paths, ocean patrols, events and terrain".into());
            lines.push("rem protection: faster and quieter for unfinished maps.".into());
            lines.push("rem Double-click it to start the server, DropLaunch isn't needed. Every start loads the".into());
            lines.push("rem current map file into a fresh world. Edits you make are kept unless the map moves.".into());
            lines.push(format!("rem {} starts the normal server.", bat_escape_bare(&other(false))));
        }
        Target::Map(_) => {
            lines.push(format!("rem Start file for {title}. {TEMPLATE_MARK}"));
            lines.push("rem Double-click it to start the server, DropLaunch isn't needed. Every start loads the".into());
            lines.push("rem current map file into a fresh world. Edits you make are kept unless the map moves.".into());
            lines.push(format!("rem {} starts it in prefab test mode (for unfinished maps).", bat_escape_bare(&other(true))));
        }
    }
    if let Some(c) = chosen {
        lines.push(format!("rem {CHOSEN_MARK} {}{CHOSEN_END}", bat_escape_bare(&c.file_name())));
    }
    if let Some(b) = bundled {
        let file = b.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        lines.push(format!("rem Also uses the settings of {} (it came with the map).", bat_escape_bare(&file)));
    }
    lines.push(format!("title DropLaunch - {title}{}", if prefab { " (prefab test mode)" } else { "" }));
    lines.push("cd /d \"%~dp0\"".into());
    if let Target::Map(map) = target {
        lines.push(format!("set \"MAP_FILE={}\"", script_path(layout, map)));
        lines.push("for %%I in (\"%MAP_FILE%\") do set \"MAP_FILE=%%~fI\"".into());
        lines.push("if not exist \"%MAP_FILE%\" goto map_missing".into());
        lines.push("set \"LEVEL_URL=file:///%MAP_FILE:\\=/%\"".into());
        lines.push("rem Fresh world: remove the server's copy of the map and the save from the last run.".into());
        lines.push(format!(
            "if exist \"server\\{identity}\\\" del /q \"server\\{identity}\\*.map\" \"server\\{identity}\\*.sav*\" >nul 2>&1"
        ));
    }
    lines.push("if not exist \"logs\\\" mkdir logs".into());
    match target {
        Target::Procgen | Target::Preset(_) => {
            lines.push(format!("echo Procedural Rust test server (seed {seed}, size {size}). {TEMPLATE_MARK}"));
            lines.push("echo The first start generates the map, give it a few minutes.".into());
        }
        Target::Map(_) if prefab => {
            lines.push(format!("echo Rust test server for {title} in prefab test mode. {TEMPLATE_MARK}"));
            lines.push("echo No cargo ship, boat paths, ocean patrols, events or terrain protection.".into());
        }
        Target::Map(_) => lines.push(format!("echo Rust test server for {title}. {TEMPLATE_MARK}")),
    }
    lines.push(format!("echo When it says \"Server startup complete\": open Rust, press F1 and type  client.connect 127.0.0.1:{port}"));
    lines.push("echo.".into());
    // Full path: cmd may be set to not run programs from the current folder by bare name.
    lines.push("\"%~dp0RustDedicated.exe\" -batchmode ^".into());
    let last = args.len() - 1;
    lines.extend(args.into_iter().enumerate().map(|(i, a)| if i == last { format!(" {a}") } else { format!(" {a} ^") }));
    lines.push("echo.".into());
    lines.push("echo The server has stopped.".into());
    if matches!(target, Target::Map(_)) {
        lines.push("goto done".into());
        lines.push(":map_missing".into());
        lines.push("echo The map file was not found: \"%MAP_FILE%\"".into());
        lines.push("echo Start the map again from DropLaunch, or fix the MAP_FILE line in this file.".into());
        lines.push(":done".into());
    }
    lines.push(format!("if not defined {LAUNCH_ENV} pause"));
    finish_bat(lines)
}

/// cmd.exe wants CRLF; non-ASCII paths need the UTF-8 code page (the file is written as UTF-8).
#[cfg(windows)]
fn finish_bat(mut lines: Vec<String>) -> String {
    if lines.iter().any(|l| !l.is_ascii()) {
        lines.insert(1, "chcp 65001 >nul".into());
    }
    lines.join("\r\n") + "\r\n"
}

#[cfg(not(windows))]
fn contents(layout: &Layout, target: &Target, prefab: bool, password: &str, bundled: Option<&Bundled>, chosen: Option<&Chosen>) -> String {
    let identity = identity_for(target);
    let prefab = is_prefab(target, prefab);
    let mode = if prefab { " in prefab test mode" } else { "" };
    let mut lines: Vec<String> = vec![
        "#!/usr/bin/env bash".into(),
        format!("# Start file for {}{mode}, runs without DropLaunch. {TEMPLATE_MARK}", target.title()),
    ];
    if let Some(c) = chosen {
        lines.push(format!("# {CHOSEN_MARK} {}{CHOSEN_END}", c.file_name()));
    }
    lines.push("HERE=\"$(cd \"$(dirname \"$0\")\" && pwd)\"".into());
    lines.push("cd \"$HERE\"".into());
    if let Target::Map(map) = target {
        lines.push(format!("MAP_FILE=\"{}\"", script_path(layout, map)));
        lines.push("[ -f \"$MAP_FILE\" ] || { echo \"Map file not found: $MAP_FILE\"; exit 1; }".into());
        lines.push("LEVEL_URL=\"file://$MAP_FILE\"".into());
        lines.push(format!("rm -f \"server/{identity}/\"*.map \"server/{identity}/\"*.sav*"));
    }
    lines.push("mkdir -p logs".into());
    lines.push("export LD_LIBRARY_PATH=\"$HERE/RustDedicated_Data/Plugins/x86_64:$LD_LIBRARY_PATH\"".into());
    lines.push("[ -f carbon/tools/environment.sh ] && source carbon/tools/environment.sh".into());
    lines.push("./RustDedicated -batchmode \\".into());
    let args = server_args(target, &identity, password, prefab, &extras(chosen, bundled));
    let last = args.len() - 1;
    lines.extend(args.into_iter().enumerate().map(|(i, a)| if i == last { format!("  {a}") } else { format!("  {a} \\") }));
    lines.join("\n") + "\n"
}

fn write_file(path: &Path, text: &str) -> std::io::Result<()> {
    std::fs::write(path, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// `chosen`: the start file of yours whose settings it takes.
pub fn write(layout: &Layout, target: &Target, prefab: bool, password: &str, chosen: Option<&Chosen>) -> std::io::Result<PathBuf> {
    let path = path_for(layout.server_dir, target, prefab);
    let bundled = match target {
        Target::Map(map) => bundled(map),
        Target::Procgen | Target::Preset(_) => None,
    };
    write_file(&path, &contents(layout, target, prefab, password, bundled.as_ref(), chosen))?;
    Ok(path)
}

/// The file is older than `other` (a start file that came with the map, dropped later).
fn older_than(path: &Path, other: &Path) -> bool {
    let modified = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    matches!((modified(path), modified(other)), (Some(a), Some(b)) if a < b)
}

/// The start file for `target` (`prefab`: the map's prefab test mode file), written first when
/// it's missing or out of date. A map always gets both files, so either one can be
/// double-clicked later. Also says whether the requested file was (re)written. `chosen`: the start
/// file of yours whose settings they take (None: DropLaunch's own).
pub fn ensure(layout: &Layout, target: &Target, prefab: bool, chosen: Option<&Chosen>) -> std::io::Result<(StartFile, bool)> {
    let variants: &[bool] = match target {
        Target::Procgen | Target::Preset(_) => &[false],
        Target::Map(_) => &[false, true],
    };
    let files: Vec<(bool, Option<StartFile>)> =
        variants.iter().map(|&v| (v, read(&path_for(layout.server_dir, target, v)))).collect();
    // One RCON password for both, kept across rewrites so an open console stays valid.
    let password = files
        .iter()
        .find_map(|(_, sf)| sf.as_ref()?.rcon_password.clone())
        .unwrap_or_else(|| random_password(12));
    // A start file that came with the map after these were written: its settings go in.
    let bundled = match target {
        Target::Map(map) => bundled(map),
        Target::Procgen | Target::Preset(_) => None,
    };
    let chosen_name = chosen.map(|c| c.file_name().to_ascii_lowercase());
    let stale = |sf: &StartFile| {
        needs_rewrite(sf, target)
            || bundled.as_ref().is_some_and(|b| older_than(&sf.path, &b.path))
            // Another start file of yours was chosen (or none any more), or yours is newer.
            || sf.chosen.as_ref().map(|n| n.to_ascii_lowercase()) != chosen_name
            || chosen.is_some_and(|c| older_than(&sf.path, &c.path))
    };
    let mut rewritten = false;
    for (variant, existing) in &files {
        if existing.as_ref().is_none_or(stale) {
            write(layout, target, *variant, &password, chosen)?;
            rewritten |= *variant == is_prefab(target, prefab);
        }
    }
    let path = path_for(layout.server_dir, target, prefab);
    let parsed = read(&path).ok_or_else(|| std::io::Error::other(format!("could not read {}", path.display())))?;
    Ok((parsed, rewritten))
}

fn needs_rewrite(sf: &StartFile, target: &Target) -> bool {
    if !sf.current_template && sf.made_by_droplaunch() {
        return true;
    }
    match target {
        Target::Procgen | Target::Preset(_) => sf.map.is_some(),
        Target::Map(map) => sf.map.as_deref().is_none_or(|m| !same_path(m, map)),
    }
}

/// What a start file in `server_dir` starts: its map, else a start file of yours (the file made
/// for it from the map list), else the procedural map.
pub fn target_of(sf: &StartFile, server_dir: &Path) -> Target {
    if let Some(map) = &sf.map {
        return Target::Map(map.clone());
    }
    match sf.chosen.clone().map(Target::Preset) {
        Some(preset) if same_path(&sf.path, &path_for(server_dir, &preset, false)) => preset,
        _ => Target::Procgen,
    }
}

impl StartFile {
    pub fn made_by_droplaunch(&self) -> bool {
        self.identity.starts_with("droplaunch_")
    }
}

// ---------------------------------------------------------------- reading

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("valid regex"))
}

/// Value of a `+name value` or `+name "value"` argument.
fn arg_value(text: &str, cell: &'static OnceLock<Regex>, name: &str) -> Option<String> {
    let pattern = format!(r#"(?i)(?:^|\s)[+-]{}\s+(?:"([^"]*)"|([^\s"^]+))"#, regex::escape(name));
    let caps = re(cell, &pattern).captures_iter(text).last()?;
    caps.get(1).or_else(|| caps.get(2)).map(|m| m.as_str().to_string())
}

pub fn read(path: &Path) -> Option<StartFile> {
    let text = std::fs::read_to_string(path).ok()?;
    let dir = path.parent()?;
    Some(parse(&text, path, dir))
}

pub fn parse(text: &str, path: &Path, dir: &Path) -> StartFile {
    static MAP_FILE: OnceLock<Regex> = OnceLock::new();
    static LEVELURL: OnceLock<Regex> = OnceLock::new();
    static IDENTITY: OnceLock<Regex> = OnceLock::new();
    static PORT: OnceLock<Regex> = OnceLock::new();
    static RCON_PORT: OnceLock<Regex> = OnceLock::new();
    static PASSWORD: OnceLock<Regex> = OnceLock::new();
    static LOGFILE: OnceLock<Regex> = OnceLock::new();
    static EVENTS: OnceLock<Regex> = OnceLock::new();
    static CHOSEN: OnceLock<Regex> = OnceLock::new();

    let unescape = |s: &str| if cfg!(windows) { s.replace("%%", "%") } else { s.to_string() };
    let resolve = |s: &str| -> PathBuf {
        let s = unescape(s);
        let rel = s.strip_prefix("%~dp0").or_else(|| s.strip_prefix("$HERE/"));
        match rel {
            Some(rel) => normalize(&dir.join(rel.trim_start_matches(['\\', '/']))),
            None if Path::new(&s).is_absolute() => normalize(Path::new(&s)),
            None => normalize(&dir.join(s)),
        }
    };

    let map = re(&MAP_FILE, r#"(?im)^\s*(?:set\s+"MAP_FILE=([^"\r\n]*)"|MAP_FILE="([^"\r\n]*)")"#)
        .captures(text)
        .and_then(|c| c.get(1).or_else(|| c.get(2)))
        .map(|m| resolve(m.as_str()))
        .or_else(|| {
            // Older/hand-made files: a literal file:/// URL.
            let url = arg_value(text, &LEVELURL, "server.levelurl")?;
            let raw = url.strip_prefix("file:///").or_else(|| url.strip_prefix("file://"))?;
            (!raw.contains('%') && !raw.contains('$')).then(|| normalize(Path::new(&raw.replace('/', std::path::MAIN_SEPARATOR_STR))))
        });

    let port = |cell, name, default| arg_value(text, cell, name).and_then(|v| v.parse().ok()).unwrap_or(default);
    let log_file = arg_value(text, &LOGFILE, "logfile").filter(|s| !s.is_empty()).map(|s| resolve(&s));
    StartFile {
        path: path.to_path_buf(),
        identity: arg_value(text, &IDENTITY, "server.identity").unwrap_or_else(|| "my_server_identity".into()),
        server_port: port(&PORT, "server.port", DEFAULT_PORT),
        rcon_port: port(&RCON_PORT, "rcon.port", DEFAULT_RCON_PORT),
        rcon_password: arg_value(text, &PASSWORD, "rcon.password").map(|p| unescape(&p)).filter(|p| !p.is_empty()),
        log_file,
        prefab_mode: arg_value(text, &EVENTS, "server.events").is_some_and(|v| v.eq_ignore_ascii_case("false") || v == "0"),
        current_template: text.contains(LAUNCH_ENV) && text.contains(TEMPLATE_MARK) && !text.contains(RETIRED_LEVELURL_ENV),
        chosen: re(&CHOSEN, &format!(r"(?m)^\s*(?:rem|#)\s+{CHOSEN_MARK} (.+?){}\s*$", regex::escape(CHOSEN_END)))
            .captures(text)
            .map(|c| c[1].to_string()),
        map,
    }
}

/// Every DropLaunch-style start file in the server folder.
pub fn list(server_dir: &Path) -> Vec<StartFile> {
    let Ok(entries) = std::fs::read_dir(server_dir) else { return Vec::new() };
    let mut files: Vec<StartFile> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_ascii_lowercase();
            name.starts_with("start_") && name.ends_with(&format!(".{EXT}"))
        })
        .filter_map(|p| read(&p))
        .collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}

// ---------------------------------------------------------------- update scripts

/// In every server folder: updates that folder.
#[cfg(windows)]
pub const UPDATE_SCRIPT: &str = "update_server.bat";
#[cfg(not(windows))]
pub const UPDATE_SCRIPT: &str = "update_server.sh";
/// In the SteamCMD folder: updates every server folder.
#[cfg(windows)]
pub const STEAMCMD_UPDATE_SCRIPT: &str = "update_rust_server.bat";
#[cfg(not(windows))]
pub const STEAMCMD_UPDATE_SCRIPT: &str = "update_rust_server.sh";

/// Writes `text` unless the file holds it already.
fn write_changed(path: &Path, text: &str) -> std::io::Result<()> {
    if std::fs::read(path).is_ok_and(|old| old == text.as_bytes()) {
        return Ok(());
    }
    write_file(path, text)
}

/// `update_server.bat`: updates (or with `validate`, checks) the server in its own folder with
/// SteamCMD, the way DropLaunch does (see update_routine). Rewritten when the branch or the
/// update setting changes.
#[cfg(windows)]
pub fn write_update_script(layout: &Layout, branch: &str, update_framework: bool) -> std::io::Result<PathBuf> {
    let steamcmd = script_path(layout, &install::executable(layout.steamcmd_dir));
    let branch = sanitize_branch(branch);
    let mut lines: Vec<String> = vec![
        "@echo off".into(),
        format!("rem DropLaunch: updates the Rust server files in this folder with SteamCMD ({branch} branch). {TEMPLATE_MARK}"),
        "rem Works without DropLaunch. Run it as  update_server.bat validate  to also check every file.".into(),
        format!("rem {STEAMCMD_UPDATE_SCRIPT} in the SteamCMD folder updates all your server folders."),
        "rem Stop the server first: files that are in use can't be updated.".into(),
        "title DropLaunch - update the Rust server files".into(),
    ];
    lines.extend(update_head(&steamcmd, update_framework));
    lines.push(format!("set \"DIR=%~dp0.\" & set \"BRANCH={branch}\" & call :update"));
    lines.extend(update_routine());
    let path = layout.server_dir.join(UPDATE_SCRIPT);
    write_changed(&path, &finish_bat(lines))?;
    Ok(path)
}

/// `update_rust_server.bat` in the SteamCMD folder: updates every server folder (`servers`, each
/// with its branch) the way DropLaunch does, so the servers can be updated without starting it.
#[cfg(windows)]
pub fn write_steamcmd_update_script(base: &Path, steamcmd_dir: &Path, servers: &[(PathBuf, String)], update_framework: bool) -> std::io::Result<PathBuf> {
    let at = |path: &Path| script_path_from(base, steamcmd_dir, path);
    let mut lines: Vec<String> = vec![
        "@echo off".into(),
        format!("rem DropLaunch: updates your Rust server files with SteamCMD, with DropLaunch's update settings. {TEMPLATE_MARK}"),
        format!("rem Works without DropLaunch. Run it as  {STEAMCMD_UPDATE_SCRIPT} validate  to also check every file."),
        "rem Stop the servers first: files that are in use can't be updated.".into(),
        "rem DropLaunch writes this file again when its update settings or server folders change.".into(),
        "title DropLaunch - update the Rust server".into(),
    ];
    lines.extend(update_head(&at(&install::executable(steamcmd_dir)), update_framework));
    lines.push("rem The server folders, each with its branch.".into());
    for (dir, branch) in servers {
        lines.push(format!("set \"DIR={}\" & set \"BRANCH={}\" & call :update", at(dir), sanitize_branch(branch)));
    }
    lines.extend(update_routine());
    let path = steamcmd_dir.join(STEAMCMD_UPDATE_SCRIPT);
    write_changed(&path, &finish_bat(lines))?;
    Ok(path)
}

/// The settings both update scripts start with; `validate` as the first argument.
#[cfg(windows)]
fn update_head(steamcmd: &str, update_framework: bool) -> Vec<String> {
    vec![
        "setlocal".into(),
        "cd /d \"%~dp0\"".into(),
        "rem DropLaunch's update setting (Settings > Servers > Update Carbon or Oxide with every server".into(),
        "rem update): when Rust's build changed, Carbon or Oxide goes to its newest version too. 1 = on, 0 = off.".into(),
        format!("set \"UPDATE_FRAMEWORK={}\"", u8::from(update_framework)),
        format!("set \"STEAMCMD={steamcmd}\""),
        "set \"VALIDATE=\"".into(),
        "if /i \"%~1\"==\"validate\" set \"VALIDATE=validate\"".into(),
        "if not exist \"%STEAMCMD%\" goto steamcmd_missing".into(),
    ]
}

/// The end of both update scripts (after their `set "DIR=..." & set "BRANCH=..." & call :update`
/// lines; not `call :update "<folder>"`, since CALL expands a folder's % signs a second time), and
/// the routine those call: what DropLaunch's own update does. SteamCMD (a second run that checks every
/// file when the first didn't go through), then Carbon or Oxide: the newest one when Rust's build
/// changed and UPDATE_FRAMEWORK is 1, else Oxide put back when the update replaced its files.
#[cfg(windows)]
fn update_routine() -> Vec<String> {
    let app = RUST_SERVER_APP_ID;
    let steamcmd = |validate: &str| {
        format!("\"%STEAMCMD%\" +@ShutdownOnFailedCommand 1 +@NoPromptForPassword 1 +force_install_dir \"%DIR%\\.\" +login anonymous +app_update {app} -beta %BRANCH% {validate} +quit")
    };
    // Steam's manifest has `"buildid"		"21345678"`: the second word, without its quotes.
    let buildid = |var: &str| format!(r#"if exist "%MANIFEST%" for /f "tokens=2" %%a in ('findstr /c:"\"buildid\"" "%MANIFEST%"') do set "{var}=%%~a""#);
    let (carbon, oxide) = (crate::mods::CARBON_URL, crate::mods::OXIDE_URL);
    vec![
        "goto finished".into(),
        ":steamcmd_missing".into(),
        "echo SteamCMD was not found at \"%STEAMCMD%\".".into(),
        "echo Run DropLaunch once to download it, or fix the STEAMCMD line in this file.".into(),
        ":finished".into(),
        "echo.".into(),
        "echo Done.".into(),
        format!("if not defined {LAUNCH_ENV} pause"),
        "exit /b".into(),
        String::new(),
        "rem set \"DIR=<folder>\" & set \"BRANCH=<branch>\" & call :update  updates one server folder the way DropLaunch does.".into(),
        ":update".into(),
        "for %%d in (\"%DIR%\") do set \"DIR=%%~fd\"".into(),
        format!("set \"MANIFEST=%DIR%\\steamapps\\appmanifest_{app}.acf\""),
        "echo.".into(),
        "echo Updating \"%DIR%\" (%BRANCH% branch) with SteamCMD...".into(),
        "echo.".into(),
        "if not exist \"%DIR%\\\" goto update_missing".into(),
        "set \"BEFORE=\"".into(),
        buildid("BEFORE"),
        steamcmd("%VALIDATE%"),
        "call :wait_steamcmd".into(),
        "rem Steam's manifest says StateFlags 4 when the update went through. When it didn't, files it had to".into(),
        "rem patch were often missing (Carbon removes three of Rust's own files whenever the server starts):".into(),
        "rem Steam has marked them now, and a second run that checks every file downloads them again.".into(),
        r#"findstr /c:"StateFlags" "%MANIFEST%" 2>nul | findstr /c:"\"4\"" >nul && goto update_done"#.into(),
        "echo.".into(),
        "echo The update didn't go through (often: some of the server's files are missing).".into(),
        "echo Checking every file and trying again...".into(),
        "echo.".into(),
        steamcmd("validate"),
        "call :wait_steamcmd".into(),
        ":update_done".into(),
        "set \"AFTER=\"".into(),
        buildid("AFTER"),
        "rem Carbon or Oxide, when one is in the folder (Carbon first, as in DropLaunch).".into(),
        "set \"FRAMEWORK=\"".into(),
        "if exist \"%DIR%\\RustDedicated_Data\\Managed\\Oxide.Rust.dll\" set \"FRAMEWORK=Oxide\"".into(),
        "if exist \"%DIR%\\carbon\\managed\\Carbon.dll\" set \"FRAMEWORK=Carbon\"".into(),
        "if not defined FRAMEWORK goto :eof".into(),
        "rem A new Rust build usually needs the newest Carbon or Oxide (plugins and their settings stay).".into(),
        "if not \"%UPDATE_FRAMEWORK%\"==\"1\" goto update_oxide".into(),
        "if not defined BEFORE goto update_oxide".into(),
        "if \"%BEFORE%\"==\"%AFTER%\" goto update_oxide".into(),
        "echo.".into(),
        "echo Rust's build changed: updating %FRAMEWORK% to its newest version...".into(),
        "set \"DONE=%FRAMEWORK% updated.\"".into(),
        "goto update_framework".into(),
        ":update_oxide".into(),
        "rem Oxide replaces Rust files that every update or validate puts back: re-apply it when that happened.".into(),
        "if not \"%FRAMEWORK%\"==\"Oxide\" goto :eof".into(),
        r#"findstr /m /c:"Oxide.Core" "%DIR%\RustDedicated_Data\Managed\Assembly-CSharp.dll" >nul 2>&1 && goto :eof"#.into(),
        "echo.".into(),
        "echo The update removed Oxide, putting it back...".into(),
        "set \"DONE=Oxide re-applied.\"".into(),
        ":update_framework".into(),
        "set \"ZIP=%TEMP%\\droplaunch_%FRAMEWORK%.zip\"".into(),
        format!("set \"URL={oxide}\""),
        format!("if \"%FRAMEWORK%\"==\"Carbon\" set \"URL={carbon}\""),
        "\"%SystemRoot%\\System32\\curl.exe\" -fL --retry 2 -o \"%ZIP%\" \"%URL%\" || goto update_framework_failed".into(),
        "\"%SystemRoot%\\System32\\tar.exe\" -xf \"%ZIP%\" -C \"%DIR%\" || goto update_framework_failed".into(),
        "del \"%ZIP%\" >nul 2>&1".into(),
        "echo %DONE%".into(),
        "goto :eof".into(),
        ":update_framework_failed".into(),
        "del \"%ZIP%\" >nul 2>&1".into(),
        "echo Could not update %FRAMEWORK%. DropLaunch can do it, on its Server files page.".into(),
        "goto :eof".into(),
        ":update_missing".into(),
        "echo The folder isn't there: skipped.".into(),
        "goto :eof".into(),
        String::new(),
        "rem SteamCMD restarts itself after updating itself: wait until it's really done.".into(),
        ":wait_steamcmd".into(),
        "tasklist /NH /FI \"IMAGENAME eq steamcmd.exe\" 2>nul | find /i \"steamcmd.exe\" >nul && (ping -n 3 127.0.0.1 >nul & goto wait_steamcmd)".into(),
        "goto :eof".into(),
    ]
}

#[cfg(not(windows))]
pub fn write_update_script(layout: &Layout, branch: &str, _update_framework: bool) -> std::io::Result<PathBuf> {
    let steamcmd = script_path(layout, &install::executable(layout.steamcmd_dir));
    let branch = sanitize_branch(branch);
    let mut lines: Vec<String> = vec![
        "#!/usr/bin/env bash".into(),
        format!("# DropLaunch: updates the Rust server files in this folder with SteamCMD ({branch} branch). {TEMPLATE_MARK}"),
        "HERE=\"$(cd \"$(dirname \"$0\")\" && pwd)\"".into(),
        "VALIDATE=\"$1\"".into(),
    ];
    lines.extend(update_function_sh(&steamcmd));
    lines.push(format!("update \"$HERE\" {branch}"));
    let path = layout.server_dir.join(UPDATE_SCRIPT);
    write_changed(&path, &(lines.join("\n") + "\n"))?;
    Ok(path)
}

#[cfg(not(windows))]
pub fn write_steamcmd_update_script(base: &Path, steamcmd_dir: &Path, servers: &[(PathBuf, String)], _update_framework: bool) -> std::io::Result<PathBuf> {
    let at = |path: &Path| script_path_from(base, steamcmd_dir, path);
    let mut lines: Vec<String> = vec![
        "#!/usr/bin/env bash".into(),
        format!("# DropLaunch: updates your Rust server files with SteamCMD. {TEMPLATE_MARK}"),
        "HERE=\"$(cd \"$(dirname \"$0\")\" && pwd)\"".into(),
        "VALIDATE=\"$1\"".into(),
    ];
    lines.extend(update_function_sh(&at(&install::executable(steamcmd_dir))));
    for (dir, branch) in servers {
        lines.push(format!("update \"{}\" {}", at(dir), sanitize_branch(branch)));
    }
    let path = steamcmd_dir.join(STEAMCMD_UPDATE_SCRIPT);
    write_changed(&path, &(lines.join("\n") + "\n"))?;
    Ok(path)
}

/// `update <folder> <branch>` for the .sh scripts.
#[cfg(not(windows))]
fn update_function_sh(steamcmd: &str) -> Vec<String> {
    let app = RUST_SERVER_APP_ID;
    vec![
        "update() {".into(),
        "  [ -d \"$1\" ] || { echo \"$1 isn't there: skipped.\"; return; }".into(),
        format!("  \"{steamcmd}\" +force_install_dir \"$1\" +login anonymous +app_update {app} -beta \"$2\" $VALIDATE +quit"),
        "  # Not through (StateFlags 4 in Steam's manifest)? Often files it had to patch were missing (Carbon".into(),
        "  # removes three of Rust's own files whenever the server starts): a run that checks every file gets them.".into(),
        format!(
            "  grep -q '\"StateFlags\"[[:space:]]*\"4\"' \"$1/steamapps/appmanifest_{app}.acf\" 2>/dev/null || \"{steamcmd}\" +force_install_dir \"$1\" +login anonymous +app_update {app} -beta \"$2\" validate +quit"
        ),
        "}".into(),
    ]
}

/// Branch names are plain words; never let one inject batch syntax.
fn sanitize_branch(branch: &str) -> String {
    let clean: String = branch.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '.').collect();
    if clean.is_empty() {
        "public".into()
    } else {
        clean
    }
}

pub fn random_password(len: usize) -> String {
    const CHARS: &[u8] = b"abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut out = String::with_capacity(len);
    while out.len() < len {
        // RandomState is seeded from the OS for every instance: plenty for a local RCON password.
        let mut h = RandomState::new().build_hasher();
        h.write_usize(out.len());
        let mut v = h.finish();
        for _ in 0..8 {
            if out.len() == len {
                break;
            }
            out.push(CHARS[(v % CHARS.len() as u64) as usize] as char);
            v /= CHARS.len() as u64;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("droplaunch-sf-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn names_are_safe_and_never_collide_with_procgen() {
        assert_eq!(sanitize("My Map (v2) & co"), "My_Map_v2_co");
        assert_eq!(sanitize("..."), "map");
        assert_eq!(key_for(&Target::Map(PathBuf::from("x/procgen_server.map"))), "procgen_server_map");
        // A start file of yours from the map list: a file of its own, never a map's or the
        // procedural map's.
        assert_eq!(key_for(&Target::Preset("start_server.bat".into())), "procgen_start_server");
        assert_eq!(key_for(&Target::Preset("server.bat".into())), "procgen_server_file");
        assert_eq!(key_for(&Target::Map(PathBuf::from("x/procgen_start_server.map"))), "procgen_start_server_map");
        assert_eq!(path_for(Path::new("S"), &Target::Procgen, true).file_name().unwrap(), format!("start_procgen_server.{EXT}").as_str());
        let map = Target::Map(PathBuf::from("x/Race.map"));
        assert_eq!(path_for(Path::new("S"), &map, true).file_name().unwrap(), format!("start_Race_prefab.{EXT}").as_str());
        // A map called "Race prefab" must not take Race's prefab file.
        let other = Target::Map(PathBuf::from("x/Race prefab.map"));
        assert_eq!(path_for(Path::new("S"), &other, false).file_name().unwrap(), format!("start_Race_prefab_map.{EXT}").as_str());
        assert!(is_prefab_file(Path::new("S"), &map, &Path::new("S").join(format!("start_Race_prefab.{EXT}"))));
        assert!(!is_prefab_file(Path::new("S"), &Target::Procgen, &path_for(Path::new("S"), &Target::Procgen, true)));
    }

    #[test]
    fn relative_paths() {
        let rel = relative(Path::new("C:/x/DropLaunch/Server_Files"), Path::new("C:/x/DropLaunch/Maps/a b/m.map")).unwrap();
        assert_eq!(rel, PathBuf::from("../Maps/a b/m.map"));
        assert!(is_inside(Path::new("C:/x/DropLaunch/Maps/m.map"), Path::new("C:/x/DropLaunch")));
        assert!(!is_inside(Path::new("C:/x/Other/m.map"), Path::new("C:/x/DropLaunch")));
        assert_eq!(normalize(Path::new("C:/a/./b/../c")), PathBuf::from("C:/a/c"));
    }

    #[cfg(windows)]
    #[test]
    fn map_start_file_round_trips() {
        let base = temp_dir("map");
        let server = base.join("Server_Files");
        std::fs::create_dir_all(&server).unwrap();
        let map = base.join("Maps").join("Sub 100%").join("My Map & co.map");
        let layout = Layout { base: &base, server_dir: &server, steamcmd_dir: &base.join("SteamCMD") };
        let target = Target::Map(map.clone());

        let path = write(&layout, &target, true, "pw123", None).unwrap();
        assert_eq!(path.file_name().unwrap(), "start_My_Map_co_prefab.bat");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("set \"MAP_FILE=%~dp0..\\Maps\\Sub 100%%\\My Map & co.map\""), "{text}");
        assert!(text.contains("set \"LEVEL_URL=file:///%MAP_FILE:\\=/%\""), "{text}");
        assert!(!text.contains(RETIRED_LEVELURL_ENV));
        assert!(text.contains("+server.levelurl \"%LEVEL_URL%\""));
        assert!(text.contains("title DropLaunch - My Map ^& co (prefab test mode)"));
        assert!(text.contains("echo Rust test server for My Map ^& co in prefab test mode. Made with DropLaunch.gg"));
        assert!(text.contains("+server.events false"));
        assert!(text.contains("\r\n"));
        // every argument line but the last continues with ^
        let carets = text.lines().filter(|l| l.ends_with(" ^")).count();
        assert_eq!(carets, server_args(&target, "x", "y", true, &[]).len());

        let sf = read(&path).unwrap();
        assert!(same_path(sf.map.as_deref().unwrap(), &map), "{:?}", sf.map);
        assert_eq!(sf.identity, "droplaunch_My_Map_co");
        assert_eq!(sf.rcon_password.as_deref(), Some("pw123"));
        assert_eq!((sf.server_port, sf.rcon_port), (28015, 28016));
        assert_eq!(sf.log_file.as_deref(), Some(server.join("logs").join("droplaunch_My_Map_co_prefab.log").as_path()));
        assert!(sf.prefab_mode && sf.current_template);

        // Asking for the normal file writes it next to the prefab one, with the same password.
        let (normal, rewritten) = ensure(&layout, &target, false, None).unwrap();
        assert!(rewritten && !normal.prefab_mode);
        assert_eq!(normal.path.file_name().unwrap(), "start_My_Map_co.bat");
        assert_eq!(normal.identity, sf.identity);
        assert_eq!(normal.rcon_password.as_deref(), Some("pw123"));
        assert_ne!(normal.log_file, sf.log_file);

        // Both exist and point at the map: kept as they are, so user edits survive.
        std::fs::write(&path, text.replace("+server.maxplayers 10", "+server.maxplayers 50")).unwrap();
        let (_, rewritten) = ensure(&layout, &target, true, None).unwrap();
        assert!(!rewritten);
        assert!(std::fs::read_to_string(&path).unwrap().contains("maxplayers 50"));

        // Another map with the same name takes the files over.
        let moved = base.join("Elsewhere").join("My Map & co.map");
        let (sf, rewritten) = ensure(&layout, &Target::Map(moved.clone()), true, None).unwrap();
        assert!(rewritten && same_path(sf.map.as_deref().unwrap(), &moved));
        assert_eq!(sf.rcon_password.as_deref(), Some("pw123"));
        let _ = std::fs::remove_dir_all(base);
    }

    #[cfg(windows)]
    #[test]
    fn maps_outside_the_folder_use_absolute_paths() {
        let base = temp_dir("abs");
        let server = base.join("Server_Files");
        std::fs::create_dir_all(&server).unwrap();
        let layout = Layout { base: &base, server_dir: &server, steamcmd_dir: &base.join("SteamCMD") };
        let elsewhere = PathBuf::from(r"D:\RustEdit\Exports\test.map");
        let path = write(&layout, &Target::Map(elsewhere.clone()), false, "pw", None).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(r#"set "MAP_FILE=D:\RustEdit\Exports\test.map""#), "{text}");
        assert_eq!(read(&path).unwrap().map, Some(elsewhere));
        let _ = std::fs::remove_dir_all(base);
    }

    #[cfg(windows)]
    #[test]
    fn a_start_file_that_came_with_the_map_adds_its_settings() {
        let base = temp_dir("bundled");
        let server = base.join("Server_Files");
        let pack = base.join("Maps").join("Pack");
        std::fs::create_dir_all(&server).unwrap();
        std::fs::create_dir_all(&pack).unwrap();
        let map = pack.join("Island.map");
        std::fs::write(&map, b"map").unwrap();
        let layout = Layout { base: &base, server_dir: &server, steamcmd_dir: &base.join("SteamCMD") };
        // Written before the map's own start file came along: no extras yet.
        ensure(&layout, &Target::Map(map.clone()), false, None).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(
            pack.join("start.bat"),
            "RustDedicated.exe -batchmode +server.port 1234 +server.maxplayers 50 +decay.scale 0 +server.hostname \"Theirs\" +server.events true +evil %PATH% +server.pve \"true\"\r\n",
        )
        .unwrap();
        let b = bundled(&map).unwrap();
        assert_eq!(b.args, vec![("server.maxplayers".into(), "50".into()), ("decay.scale".into(), "0".into()), ("server.events".into(), "true".into()), ("server.pve".into(), "true".into())]);
        let (sf, rewritten) = ensure(&layout, &Target::Map(map.clone()), false, None).unwrap();
        assert!(rewritten);
        let text = std::fs::read_to_string(&sf.path).unwrap();
        assert!(text.contains("+server.maxplayers 50") && !text.contains("+server.maxplayers 10") && text.contains("+decay.scale 0"), "{text}");
        assert!(text.contains("+server.port 28015") && !text.contains("Theirs") && !text.contains("%PATH%"), "{text}");
        assert!(text.contains("rem Also uses the settings of start.bat"));
        // Prefab test mode keeps its own: events stay off.
        let prefab = std::fs::read_to_string(path_for(&server, &Target::Map(map.clone()), true)).unwrap();
        assert!(prefab.contains("+server.events false") && !prefab.contains("+server.events true"), "{prefab}");
        // Nothing new: left alone.
        assert!(!ensure(&layout, &Target::Map(map.clone()), false, None).unwrap().1);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn dropped_start_files_say_what_they_are_for() {
        // From the website's generator: a procedural map, and a custom map.
        let procgen = "@echo off\r\n\"%~dp0RustDedicated.exe\" -batchmode ^\r\n +server.port 28025 ^\r\n +server.level \"Procedural Map\" ^\r\n +server.seed 42 ^\r\n +server.worldsize 4000 ^\r\n +server.maxplayers 50 ^\r\n +server.hostname \"My server\" ^\r\n +server.identity \"my_server\" ^\r\n +rcon.ip 0.0.0.0 ^\r\n +rcon.password \"secret\" ^\r\n +rcon.web 1 ^\r\n +decay.scale 0 ^\r\n -logfile \"logs\\my_server.log\"\r\n";
        let map = "RustDedicated.exe -batchmode +server.levelurl \"%LEVEL_URL%\" +server.seed 1 +server.maxplayers 20 +server.description \"100%% fun\"";
        assert_eq!(use_of(procgen), Some(Use::Procgen));
        assert_eq!(use_of(map), Some(Use::Maps));
        // update_server.bat runs SteamCMD (and names RustDedicated_Data): not a start file.
        assert_eq!(use_of("\"%STEAMCMD%\" +login anonymous +app_update 258550 +quit\r\nif not exist \"RustDedicated_Data\\Managed\\Oxide.Rust.dll\" goto finished"), None);
        assert_eq!(use_of("echo hello"), None);
        // Started from the map list: all of it. Chosen for the procedural map: not its map.
        assert_eq!(
            settings_for(procgen, true),
            [("server.port", "28025"), ("server.level", "Procedural Map"), ("server.seed", "42"), ("server.worldsize", "4000"), ("server.maxplayers", "50"), ("server.hostname", "My server"), ("decay.scale", "0")]
                .map(|(n, v)| (n.to_string(), v.to_string()))
        );
        assert_eq!(
            settings_for(procgen, false),
            [("server.port", "28025"), ("server.maxplayers", "50"), ("server.hostname", "My server"), ("decay.scale", "0")].map(|(n, v)| (n.to_string(), v.to_string()))
        );
        // A custom map's: no procedural map settings, nothing with a % in it.
        assert_eq!(settings_for(map, false), vec![("server.maxplayers".to_string(), "20".to_string())]);
        // In a .zip with a map: only one with a map of its own isn't that map's.
        assert!(makes_its_own_map(procgen));
        assert!(!makes_its_own_map(map) && !makes_its_own_map("RustDedicated.exe -batchmode +server.maxplayers 50"));
    }

    #[cfg(windows)]
    #[test]
    fn a_chosen_start_file_gives_its_settings_until_another_is_chosen() {
        let base = temp_dir("chosen");
        let server = base.join("Server_Files");
        let data = base.join("data");
        std::fs::create_dir_all(&server).unwrap();
        let layout = Layout { base: &base, server_dir: &server, steamcmd_dir: &base.join("SteamCMD") };
        let dropped = base.join("Downloads").join("my start (1).bat");
        std::fs::create_dir_all(dropped.parent().unwrap()).unwrap();
        std::fs::write(&dropped, "RustDedicated.exe -batchmode +server.port 28025 +server.seed 42 +server.worldsize 4000 +server.maxplayers 50 +server.identity \"theirs\" +rcon.password \"x\"\r\n").unwrap();

        // DropLaunch's own first, edited by hand.
        let (sf, _) = ensure(&layout, &Target::Procgen, false, None).unwrap();
        assert!(sf.chosen.is_none() && sf.server_port == DEFAULT_PORT);
        let text = std::fs::read_to_string(&sf.path).unwrap();
        std::fs::write(&sf.path, text.replace("+server.maxplayers 10", "+server.maxplayers 12")).unwrap();
        assert!(!ensure(&layout, &Target::Procgen, false, None).unwrap().1);

        // Dropped: kept under a safe name. Chosen: the file is made again with its settings.
        std::thread::sleep(std::time::Duration::from_millis(20));
        let (copy, usage) = add_dropped(&data, &dropped).unwrap();
        assert_eq!((copy.file_name().unwrap().to_str().unwrap(), usage), ("my_start_1.bat.txt", Use::Procgen));
        assert_eq!((copy.parent().unwrap(), dropped_name(&copy).as_deref()), (data.join(DROPPED_DIR).as_path(), Some("my_start_1.bat")));
        let chosen = Chosen::read(&copy, Use::Procgen).unwrap();
        assert!(Chosen::read(&copy, Use::Maps).is_none());
        let (sf, rewritten) = ensure(&layout, &Target::Procgen, false, Some(&chosen)).unwrap();
        assert!(rewritten);
        assert_eq!(sf.chosen.as_deref(), Some("my_start_1.bat"));
        assert_eq!((sf.server_port, sf.identity.as_str()), (28025, PROCGEN_IDENTITY));
        assert_eq!(target_of(&sf, &server), Target::Procgen);
        let text = std::fs::read_to_string(&sf.path).unwrap();
        assert!(text.contains("rem Uses the settings of my_start_1.bat, your start file."), "{text}");
        // The procedural map keeps its own seed and size.
        assert!(text.contains(&format!("+server.seed {SEED}")) && text.contains(&format!("+server.worldsize {WORLD_SIZE}")) && text.contains("+server.maxplayers 50"), "{text}");
        assert!(!text.contains("+server.seed 42") && !text.contains("+server.worldsize 4000"), "{text}");
        assert!(text.contains(&format!("(seed {SEED}, size {WORLD_SIZE})")) && text.contains("client.connect 127.0.0.1:28025"), "{text}");
        assert!(!text.contains("theirs") && !text.contains("+rcon.password \"x\""), "{text}");
        assert!(!ensure(&layout, &Target::Procgen, false, Some(&chosen)).unwrap().1);

        // Dropped again (changed): made again. Back to DropLaunch's own: made again without it.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&dropped, "RustDedicated.exe +server.seed 7 +server.maxplayers 7\r\n").unwrap();
        let (copy, _) = add_dropped(&data, &dropped).unwrap();
        let chosen = Chosen::read(&copy, Use::Procgen).unwrap();
        assert!(ensure(&layout, &Target::Procgen, false, Some(&chosen)).unwrap().1);
        let text = std::fs::read_to_string(&sf.path).unwrap();
        assert!(text.contains("+server.maxplayers 7") && text.contains(&format!("+server.seed {SEED}")), "{text}");
        let (sf, rewritten) = ensure(&layout, &Target::Procgen, false, None).unwrap();
        assert!(rewritten && sf.chosen.is_none());
        assert!(std::fs::read_to_string(&sf.path).unwrap().contains(&format!("+server.seed {SEED}")));

        // Custom maps: both files take it, prefab test mode keeps its own arguments.
        std::fs::write(&dropped, "RustDedicated.exe +server.levelurl \"https://x/y.map\" +server.maxplayers 30 +server.events true\r\n").unwrap();
        let (copy, usage) = add_dropped(&data, &dropped).unwrap();
        assert_eq!(usage, Use::Maps);
        let chosen = Chosen::read(&copy, Use::Maps).unwrap();
        let map = Target::Map(base.join("Maps").join("x.map"));
        ensure(&layout, &map, false, Some(&chosen)).unwrap();
        let normal = std::fs::read_to_string(path_for(&server, &map, false)).unwrap();
        let prefab = std::fs::read_to_string(path_for(&server, &map, true)).unwrap();
        assert!(normal.contains("+server.maxplayers 30") && normal.contains("+server.events true") && !normal.contains("https://x"), "{normal}");
        assert!(prefab.contains("+server.maxplayers 30") && prefab.contains("+server.events false") && !prefab.contains("+server.events true"), "{prefab}");

        // Not a start file.
        let other = base.join("notes.bat");
        std::fs::write(&other, "echo hi").unwrap();
        assert!(add_dropped(&data, &other).is_err());
        let _ = std::fs::remove_dir_all(base);
    }

    #[cfg(windows)]
    #[test]
    fn a_procedural_start_file_from_the_map_list_starts_with_all_its_settings() {
        let base = temp_dir("preset");
        let server = base.join("Server_Files");
        let data = base.join("data");
        std::fs::create_dir_all(&server).unwrap();
        let layout = Layout { base: &base, server_dir: &server, steamcmd_dir: &base.join("SteamCMD") };
        let dropped = base.join("Downloads").join("start_server.bat");
        std::fs::create_dir_all(dropped.parent().unwrap()).unwrap();
        std::fs::write(&dropped, "RustDedicated.exe -batchmode +server.port 28025 +server.seed 42 +server.worldsize 4000 +server.maxplayers 50 +server.identity \"theirs\"\r\n").unwrap();
        let (copy, _) = add_dropped(&data, &dropped).unwrap();
        let preset = Chosen::preset(&copy).unwrap();
        assert_eq!((preset.value("server.seed"), preset.value("server.worldsize")), (Some("42"), Some("4000")));

        // A file of its own, with its own seed and size and its own world folder.
        let target = Target::Preset("start_server.bat".into());
        let (sf, rewritten) = ensure(&layout, &target, false, Some(&preset)).unwrap();
        assert!(rewritten);
        assert_eq!(sf.path, server.join("start_procgen_start_server.bat"));
        assert_eq!((sf.identity.as_str(), sf.server_port, sf.chosen.as_deref()), ("droplaunch_procgen_start_server", 28025, Some("start_server.bat")));
        assert!(sf.map.is_none() && sf.made_by_droplaunch() && sf.current_template);
        let text = std::fs::read_to_string(&sf.path).unwrap();
        assert!(text.contains("+server.seed 42") && text.contains("+server.worldsize 4000") && text.contains("+server.maxplayers 50"), "{text}");
        assert!(text.contains("(seed 42, size 4000)") && !text.contains("theirs"), "{text}");
        assert!(!ensure(&layout, &target, false, Some(&preset)).unwrap().1);
        // A running server started from it is known by its file.
        assert_eq!(target_of(&sf, &server), target);

        // A custom map's start file isn't one.
        std::fs::write(&dropped, "RustDedicated.exe +server.levelurl \"https://x/y.map\" +server.maxplayers 30\r\n").unwrap();
        let (copy, usage) = add_dropped(&data, &dropped).unwrap();
        assert_eq!(usage, Use::Maps);
        assert!(Chosen::preset(&copy).is_none());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn same_settings_whatever_the_map() {
        let a = "RustDedicated -batchmode +server.port 28015 +server.levelurl \"file:///a.map\" +server.identity \"droplaunch_a\" +server.hostname \"A\" +server.maxplayers 10 +rcon.password \"x\"";
        let b = "RustDedicated -batchmode +server.port 28015 +server.levelurl \"file:///b.map\" +server.identity \"droplaunch_b\" +server.hostname \"B\" +server.maxplayers 10 +rcon.password \"y\"";
        let c = "RustDedicated -batchmode +server.port 28015 +server.levelurl \"file:///b.map\" +server.identity \"droplaunch_b\" +server.maxplayers 50";
        assert_eq!(settings_of(a), settings_of(b));
        assert_ne!(settings_of(a), settings_of(c));
        assert_eq!(settings_of(a), vec!["+server.maxplayers 10", "+server.port 28015"]);
    }

    #[cfg(windows)]
    #[test]
    fn old_droplaunch_procgen_files_are_upgraded() {
        let base = temp_dir("old");
        let server = base.join("Server_Files");
        std::fs::create_dir_all(&server).unwrap();
        let old = "@echo off\r\n\"%~dp0RustDedicated.exe\" -batchmode ^\r\n +server.level \"Procedural Map\" ^\r\n +server.identity \"droplaunch_procgen\" ^\r\n +rcon.password \"keepme\" ^\r\n -logfile \"droplaunch_procgen.log\"\r\npause\r\n";
        std::fs::write(server.join("start_procgen_server.bat"), old).unwrap();
        let layout = Layout { base: &base, server_dir: &server, steamcmd_dir: &base.join("SteamCMD") };
        let (sf, rewritten) = ensure(&layout, &Target::Procgen, false, None).unwrap();
        assert!(rewritten && sf.current_template);
        assert_eq!(sf.rcon_password.as_deref(), Some("keepme"));
        assert!(sf.map.is_none());
        let text = std::fs::read_to_string(&sf.path).unwrap();
        assert!(text.contains("echo Procedural Rust test server (seed 1184645116, size 3000). Made with DropLaunch.gg"), "{text}");
        // The previous preview's wording ("made by DropLaunch") is upgraded too.
        std::fs::write(&sf.path, text.replace(TEMPLATE_MARK, "made by DropLaunch")).unwrap();
        assert!(!read(&sf.path).unwrap().current_template);
        assert!(ensure(&layout, &Target::Procgen, false, None).unwrap().1);
        // Map files from the preview versions (they took DROPLAUNCH_LEVELURL) are rewritten.
        let map = Target::Map(base.join("Maps").join("x.map"));
        let (sf, _) = ensure(&layout, &map, false, None).unwrap();
        let text = std::fs::read_to_string(&sf.path).unwrap();
        let old = text.replacen("cd /d", &format!("if defined {RETIRED_LEVELURL_ENV} set \"LEVEL_URL=%{RETIRED_LEVELURL_ENV}%\"\r\ncd /d"), 1);
        std::fs::write(&sf.path, old).unwrap();
        assert!(!read(&sf.path).unwrap().current_template);
        assert!(ensure(&layout, &map, false, None).unwrap().1);
        let _ = std::fs::remove_dir_all(base);
    }

    #[cfg(windows)]
    #[test]
    fn update_script_points_at_steamcmd_and_reapplies_oxide() {
        let base = temp_dir("upd");
        let server = base.join("Server_Files");
        std::fs::create_dir_all(&server).unwrap();
        let layout = Layout { base: &base, server_dir: &server, steamcmd_dir: &base.join("SteamCMD") };
        let path = write_update_script(&layout, "staging & del", true).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(r#"set "STEAMCMD=%~dp0..\SteamCMD\steamcmd.exe""#), "{text}");
        assert!(text.contains("set \"DIR=%~dp0.\" & set \"BRANCH=stagingdel\" & call :update\r\n"), "{text}");
        assert!(text.contains("-beta %BRANCH% %VALIDATE% +quit"));
        // Not through: once more, checking every file.
        assert!(text.contains(r#"findstr /c:"StateFlags" "%MANIFEST%" 2>nul | findstr /c:"\"4\"" >nul && goto update_done"#), "{text}");
        assert!(text.contains("-beta %BRANCH% validate +quit"));
        assert!(text.contains("findstr /m /c:\"Oxide.Core\""));
        // The update setting: Carbon or Oxide with a new Rust build.
        assert!(text.contains("set \"UPDATE_FRAMEWORK=1\"\r\n"));
        assert!(text.contains(crate::mods::CARBON_URL) && text.contains(crate::mods::OXIDE_URL));
        write_update_script(&layout, "staging", false).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("set \"UPDATE_FRAMEWORK=0\"\r\n"));
        let _ = std::fs::remove_dir_all(base);
    }

    #[cfg(windows)]
    #[test]
    fn the_steamcmd_folder_gets_a_script_for_every_server_folder() {
        let base = temp_dir("updall");
        let steamcmd = base.join("SteamCMD");
        std::fs::create_dir_all(&steamcmd).unwrap();
        let elsewhere = temp_dir("updall 100%");
        let servers = vec![
            (base.join("Server_Files"), "public".to_string()),
            (base.join("Server_Files_staging"), "staging & del".to_string()),
            (elsewhere.join("Rust"), "aux01".to_string()),
        ];
        let path = write_steamcmd_update_script(&base, &steamcmd, &servers, false).unwrap();
        assert_eq!(path, steamcmd.join(STEAMCMD_UPDATE_SCRIPT));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(r#"set "STEAMCMD=%~dp0steamcmd.exe""#), "{text}");
        assert!(text.contains("set \"UPDATE_FRAMEWORK=0\"\r\n"));
        // In DropLaunch's folder: relative, so the folder can move. Elsewhere: the full path.
        assert!(text.contains("set \"DIR=%~dp0..\\Server_Files\" & set \"BRANCH=public\" & call :update\r\n"), "{text}");
        assert!(text.contains("set \"DIR=%~dp0..\\Server_Files_staging\" & set \"BRANCH=stagingdel\" & call :update\r\n"), "{text}");
        let outside = normalize(&elsewhere.join("Rust")).display().to_string().replace('%', "%%");
        assert!(text.contains(&format!("set \"DIR={outside}\" & set \"BRANCH=aux01\" & call :update\r\n")), "{text}");
        // The calls come before the routine, which only runs through them.
        assert!(text.find("call :update").unwrap() < text.find("goto finished").unwrap());
        assert!(text.find(":finished").unwrap() < text.find("\r\n:update\r\n").unwrap());
        // Unchanged settings: the file isn't written again.
        let written = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        write_steamcmd_update_script(&base, &steamcmd, &servers, false).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), written);
        let _ = std::fs::remove_dir_all(base);
        let _ = std::fs::remove_dir_all(elsewhere);
    }

    #[test]
    fn passwords_are_random_and_sized() {
        let (a, b) = (random_password(12), random_password(12));
        assert_eq!(a.len(), 12);
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric()));
    }
}
