//! Start files: DropLaunch writes a .bat next to RustDedicated.exe for every server it starts
//! (the procedural map; two per custom map: a normal one and one in prefab test mode) plus
//! `update_server.bat`, so everything keeps working without DropLaunch. Before launching it
//! reads the file back: the file is the source of truth, so edits to ports, password, max
//! players etc. are respected. A file is only rewritten when the map moved, or it was made by an
//! older DropLaunch.

use regex::Regex;
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
/// A map's prefab test mode file is `start_<map>_prefab.bat`.
const PREFAB_SUFFIX: &str = "_prefab";
pub const DEFAULT_PORT: u16 = 28015;
pub const DEFAULT_RCON_PORT: u16 = 28016;
/// Rust's query and Rust+ ports next to those two (they aren't written into start files).
pub const DEFAULT_QUERY_PORT: u16 = 28017;
pub const DEFAULT_APP_PORT: u16 = 28083;
const SEED: u32 = 1184645116;
const WORLD_SIZE: u32 = 3000;
/// Set by DropLaunch when it starts a file, so the window closes instead of waiting for a key.
pub const LAUNCH_ENV: &str = "DROPLAUNCH_SERVER";
/// In every file this DropLaunch writes; DropLaunch files without it are from an older version.
const TEMPLATE_MARK: &str = "Made with DropLaunch.gg";
/// Preview versions let DropLaunch hand the map's URL over in this variable: their files are
/// rewritten once.
const RETIRED_LEVELURL_ENV: &str = "DROPLAUNCH_LEVELURL";

/// What a start file runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Procgen,
    Map(PathBuf),
}

impl Target {
    pub fn title(&self) -> String {
        match self {
            Target::Procgen => crate::t!("maps.procgen.name"),
            Target::Map(path) => map_title(path),
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
    /// `+server.queryport` and `+app.port`, when the file sets them.
    pub query_port_arg: Option<u16>,
    pub app_port_arg: Option<u16>,
    pub rcon_password: Option<String>,
    pub log_file: Option<PathBuf>,
    /// Has the prefab test mode arguments.
    pub prefab_mode: bool,
    /// Written by this version of DropLaunch.
    pub current_template: bool,
}

impl StartFile {
    pub fn connect_command(&self) -> String {
        format!("client.connect 127.0.0.1:{}", self.server_port)
    }

    /// Where the server answers Steam's server queries. Rust's own choice when it isn't set (or
    /// clashes): one above the game and RCON ports, so 28017 next to 28015 and 28016.
    pub fn query_port(&self) -> u16 {
        match self.query_port_arg {
            Some(port) if port != 0 && port != self.server_port && port != self.rcon_port => port,
            _ => self.server_port.max(self.rcon_port).saturating_add(1),
        }
    }

    /// The Rust+ companion port (TCP): Rust's default is 67 above the game and RCON ports.
    pub fn app_port(&self) -> u16 {
        match self.app_port_arg {
            Some(port) if port != 0 => port,
            _ => self.server_port.max(self.rcon_port).saturating_add(67),
        }
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
            // Never collide with the procedural map's file, or with another map's prefab file.
            if lower == PROCGEN_KEY || lower.ends_with(PREFAB_SUFFIX) {
                format!("{key}_map")
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
        Target::Map(_) => format!("droplaunch_{}", key_for(target)),
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
    let portable = is_inside(path, layout.base) && is_inside(layout.server_dir, layout.base);
    match relative(layout.server_dir, path).filter(|_| portable) {
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

fn server_args(target: &Target, identity: &str, password: &str, prefab: bool, extras: &[(String, String)]) -> Vec<String> {
    let log = log_name(target, prefab);
    let mut args = vec![format!("+server.port {DEFAULT_PORT}")];
    match target {
        Target::Procgen => {
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
    args.push("+server.maxplayers 10".into());
    let (hostname, description) = match target {
        Target::Procgen => ("DropLaunch test server".to_string(), "Local test server, made with DropLaunch.gg"),
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
    // The settings of a start file that came with the map: in place of DropLaunch's own value,
    // or added (prefab test mode's arguments stay as they are).
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
fn contents(layout: &Layout, target: &Target, prefab: bool, password: &str, bundled: Option<&Bundled>) -> String {
    let identity = identity_for(target);
    let title = bat_escape_bare(&target.title());
    let prefab = is_prefab(target, prefab);
    let other = |prefab| path_for(layout.server_dir, target, prefab).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut lines: Vec<String> = vec!["@echo off".into()];
    match target {
        Target::Procgen => {
            lines.push(format!("rem Start file for a procedural map (seed {SEED}, size {WORLD_SIZE}). {TEMPLATE_MARK}"));
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
        Target::Procgen => {
            lines.push(format!("echo Procedural Rust test server (seed {SEED}, size {WORLD_SIZE}). {TEMPLATE_MARK}"));
            lines.push("echo The first start generates the map, give it a few minutes.".into());
        }
        Target::Map(_) if prefab => {
            lines.push(format!("echo Rust test server for {title} in prefab test mode. {TEMPLATE_MARK}"));
            lines.push("echo No cargo ship, boat paths, ocean patrols, events or terrain protection.".into());
        }
        Target::Map(_) => lines.push(format!("echo Rust test server for {title}. {TEMPLATE_MARK}")),
    }
    lines.push(format!(
        "echo When it says \"Server startup complete\": open Rust, press F1 and type  client.connect 127.0.0.1:{DEFAULT_PORT}"
    ));
    lines.push("echo.".into());
    // Full path: cmd may be set to not run programs from the current folder by bare name.
    lines.push("\"%~dp0RustDedicated.exe\" -batchmode ^".into());
    let extras = bundled.map_or(&[][..], |b| &b.args[..]);
    let args = server_args(target, &identity, password, prefab, extras);
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
fn contents(layout: &Layout, target: &Target, prefab: bool, password: &str, bundled: Option<&Bundled>) -> String {
    let identity = identity_for(target);
    let prefab = is_prefab(target, prefab);
    let mode = if prefab { " in prefab test mode" } else { "" };
    let mut lines: Vec<String> = vec![
        "#!/usr/bin/env bash".into(),
        format!("# Start file for {}{mode}, runs without DropLaunch. {TEMPLATE_MARK}", target.title()),
        "HERE=\"$(cd \"$(dirname \"$0\")\" && pwd)\"".into(),
        "cd \"$HERE\"".into(),
    ];
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
    let extras = bundled.map_or(&[][..], |b| &b.args[..]);
    let args = server_args(target, &identity, password, prefab, extras);
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

pub fn write(layout: &Layout, target: &Target, prefab: bool, password: &str) -> std::io::Result<PathBuf> {
    let path = path_for(layout.server_dir, target, prefab);
    let bundled = match target {
        Target::Map(map) => bundled(map),
        Target::Procgen => None,
    };
    write_file(&path, &contents(layout, target, prefab, password, bundled.as_ref()))?;
    Ok(path)
}

/// The file is older than `other` (a start file that came with the map, dropped later).
fn older_than(path: &Path, other: &Path) -> bool {
    let modified = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    matches!((modified(path), modified(other)), (Some(a), Some(b)) if a < b)
}

/// The start file for `target` (`prefab`: the map's prefab test mode file), written first when
/// it's missing or out of date. A map always gets both files, so either one can be
/// double-clicked later. Also says whether the requested file was (re)written.
pub fn ensure(layout: &Layout, target: &Target, prefab: bool) -> std::io::Result<(StartFile, bool)> {
    let variants: &[bool] = match target {
        Target::Procgen => &[false],
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
        Target::Procgen => None,
    };
    let mut rewritten = false;
    for (variant, existing) in &files {
        if existing.as_ref().is_none_or(|sf| needs_rewrite(sf, target) || bundled.as_ref().is_some_and(|b| older_than(&sf.path, &b.path))) {
            write(layout, target, *variant, &password)?;
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
        Target::Procgen => sf.map.is_some(),
        Target::Map(map) => sf.map.as_deref().is_none_or(|m| !same_path(m, map)),
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
    static QUERY_PORT: OnceLock<Regex> = OnceLock::new();
    static APP_PORT: OnceLock<Regex> = OnceLock::new();
    static PASSWORD: OnceLock<Regex> = OnceLock::new();
    static LOGFILE: OnceLock<Regex> = OnceLock::new();
    static EVENTS: OnceLock<Regex> = OnceLock::new();

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
        query_port_arg: arg_value(text, &QUERY_PORT, "server.queryport").and_then(|v| v.parse().ok()),
        app_port_arg: arg_value(text, &APP_PORT, "app.port").and_then(|v| v.parse().ok()),
        rcon_password: arg_value(text, &PASSWORD, "rcon.password").map(|p| unescape(&p)).filter(|p| !p.is_empty()),
        log_file,
        prefab_mode: arg_value(text, &EVENTS, "server.events").is_some_and(|v| v.eq_ignore_ascii_case("false") || v == "0"),
        current_template: text.contains(LAUNCH_ENV) && text.contains(TEMPLATE_MARK) && !text.contains(RETIRED_LEVELURL_ENV),
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

// ---------------------------------------------------------------- update script

#[cfg(windows)]
pub const UPDATE_SCRIPT: &str = "update_server.bat";
#[cfg(not(windows))]
pub const UPDATE_SCRIPT: &str = "update_server.sh";

const OXIDE_ZIP_URL: &str = "https://github.com/OxideMod/Oxide.Rust/releases/latest/download/Oxide.Rust.zip";

/// `update_server.bat`: update (or `validate`) the server with SteamCMD, then put Oxide back
/// when the update replaced its files. Rewritten when the branch changes.
#[cfg(windows)]
pub fn write_update_script(layout: &Layout, branch: &str) -> std::io::Result<PathBuf> {
    let steamcmd = script_path(layout, &install::executable(layout.steamcmd_dir));
    let branch = sanitize_branch(branch);
    let lines: Vec<String> = vec![
        "@echo off".into(),
        format!("rem DropLaunch: updates the Rust server files in this folder with SteamCMD ({branch} branch)."),
        "rem Works without DropLaunch. Run it as  update_server.bat validate  to also check every file.".into(),
        "rem Stop the server first: files that are in use can't be updated.".into(),
        "title DropLaunch - update the Rust server files".into(),
        "cd /d \"%~dp0\"".into(),
        format!("set \"STEAMCMD={steamcmd}\""),
        "if not exist \"%STEAMCMD%\" goto steamcmd_missing".into(),
        "set \"VALIDATE=\"".into(),
        "if /i \"%~1\"==\"validate\" set \"VALIDATE=validate\"".into(),
        format!("echo Updating the Rust server files ({branch} branch) with SteamCMD..."),
        "echo.".into(),
        format!(
            "\"%STEAMCMD%\" +@ShutdownOnFailedCommand 1 +@NoPromptForPassword 1 +force_install_dir \"%~dp0.\" +login anonymous +app_update {RUST_SERVER_APP_ID} -beta {branch} %VALIDATE% +quit"
        ),
        "rem SteamCMD restarts itself after updating itself: wait until it's really done.".into(),
        ":wait_steamcmd".into(),
        "tasklist /NH /FI \"IMAGENAME eq steamcmd.exe\" 2>nul | find /i \"steamcmd.exe\" >nul && (ping -n 3 127.0.0.1 >nul & goto wait_steamcmd)".into(),
        "rem Steam's manifest says StateFlags 4 when the update went through. When it didn't, files it had to".into(),
        "rem patch were often missing (Carbon removes three of Rust's own files whenever the server starts):".into(),
        "rem Steam has marked them now, and a second run that checks every file downloads them again.".into(),
        format!(r#"findstr /c:"StateFlags" "steamapps\appmanifest_{RUST_SERVER_APP_ID}.acf" 2>nul | findstr /c:"\"4\"" >nul && goto updated"#),
        "echo.".into(),
        "echo The update didn't go through (often: some of the server's files are missing).".into(),
        "echo Checking every file and trying again...".into(),
        "echo.".into(),
        format!(
            "\"%STEAMCMD%\" +@ShutdownOnFailedCommand 1 +@NoPromptForPassword 1 +force_install_dir \"%~dp0.\" +login anonymous +app_update {RUST_SERVER_APP_ID} -beta {branch} validate +quit"
        ),
        ":wait_steamcmd_again".into(),
        "tasklist /NH /FI \"IMAGENAME eq steamcmd.exe\" 2>nul | find /i \"steamcmd.exe\" >nul && (ping -n 3 127.0.0.1 >nul & goto wait_steamcmd_again)".into(),
        ":updated".into(),
        "rem Oxide replaces Rust files that every update or validate puts back: re-apply it when that happened.".into(),
        "if not exist \"RustDedicated_Data\\Managed\\Oxide.Rust.dll\" goto finished".into(),
        "findstr /m /c:\"Oxide.Core\" \"RustDedicated_Data\\Managed\\Assembly-CSharp.dll\" >nul 2>&1 && goto finished".into(),
        "echo.".into(),
        "echo The update removed Oxide, putting it back...".into(),
        format!(
            "\"%SystemRoot%\\System32\\curl.exe\" -fL --retry 2 -o \"%TEMP%\\droplaunch_oxide.zip\" \"{OXIDE_ZIP_URL}\" || goto oxide_failed"
        ),
        "\"%SystemRoot%\\System32\\tar.exe\" -xf \"%TEMP%\\droplaunch_oxide.zip\" -C \"%~dp0.\" || goto oxide_failed".into(),
        "del \"%TEMP%\\droplaunch_oxide.zip\" >nul 2>&1".into(),
        "echo Oxide re-applied.".into(),
        "goto finished".into(),
        ":oxide_failed".into(),
        "echo Could not re-apply Oxide. DropLaunch can do it, or unpack Oxide.Rust.zip from umod.org here.".into(),
        "goto finished".into(),
        ":steamcmd_missing".into(),
        "echo SteamCMD was not found at \"%STEAMCMD%\".".into(),
        "echo Run DropLaunch once to download it, or fix the STEAMCMD line in this file.".into(),
        ":finished".into(),
        "echo.".into(),
        "echo Done.".into(),
        format!("if not defined {LAUNCH_ENV} pause"),
    ];
    let path = layout.server_dir.join(UPDATE_SCRIPT);
    write_file(&path, &finish_bat(lines))?;
    Ok(path)
}

#[cfg(not(windows))]
pub fn write_update_script(layout: &Layout, branch: &str) -> std::io::Result<PathBuf> {
    let steamcmd = script_path(layout, &install::executable(layout.steamcmd_dir));
    let branch = sanitize_branch(branch);
    let lines: Vec<String> = vec![
        "#!/usr/bin/env bash".into(),
        format!("# DropLaunch: updates the Rust server files in this folder with SteamCMD ({branch} branch)."),
        "HERE=\"$(cd \"$(dirname \"$0\")\" && pwd)\"".into(),
        "VALIDATE=\"$1\"".into(),
        format!("\"{steamcmd}\" +force_install_dir \"$HERE\" +login anonymous +app_update {RUST_SERVER_APP_ID} -beta {branch} $VALIDATE +quit"),
        "# Not through (StateFlags 4 in Steam's manifest)? Often files it had to patch were missing (Carbon removes".into(),
        "# three of Rust's own files whenever the server starts): Steam has marked them now, and a run that checks".into(),
        "# every file downloads them again.".into(),
        format!(
            "grep -q '\"StateFlags\"[[:space:]]*\"4\"' \"$HERE/steamapps/appmanifest_{RUST_SERVER_APP_ID}.acf\" 2>/dev/null || \"{steamcmd}\" +force_install_dir \"$HERE\" +login anonymous +app_update {RUST_SERVER_APP_ID} -beta {branch} validate +quit"
        ),
    ];
    let path = layout.server_dir.join(UPDATE_SCRIPT);
    write_file(&path, &(lines.join("\n") + "\n"))?;
    Ok(path)
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

/// Branch an existing update script was written for (None if there's none).
pub fn update_script_branch(server_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(server_dir.join(UPDATE_SCRIPT)).ok()?;
    static BETA: OnceLock<Regex> = OnceLock::new();
    re(&BETA, r"-beta\s+([A-Za-z0-9_.-]+)").captures(&text).map(|c| c[1].to_string())
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

        let path = write(&layout, &target, true, "pw123").unwrap();
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
        let (normal, rewritten) = ensure(&layout, &target, false).unwrap();
        assert!(rewritten && !normal.prefab_mode);
        assert_eq!(normal.path.file_name().unwrap(), "start_My_Map_co.bat");
        assert_eq!(normal.identity, sf.identity);
        assert_eq!(normal.rcon_password.as_deref(), Some("pw123"));
        assert_ne!(normal.log_file, sf.log_file);

        // Both exist and point at the map: kept as they are, so user edits survive.
        std::fs::write(&path, text.replace("+server.maxplayers 10", "+server.maxplayers 50")).unwrap();
        let (_, rewritten) = ensure(&layout, &target, true).unwrap();
        assert!(!rewritten);
        assert!(std::fs::read_to_string(&path).unwrap().contains("maxplayers 50"));

        // Another map with the same name takes the files over.
        let moved = base.join("Elsewhere").join("My Map & co.map");
        let (sf, rewritten) = ensure(&layout, &Target::Map(moved.clone()), true).unwrap();
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
        let path = write(&layout, &Target::Map(elsewhere.clone()), false, "pw").unwrap();
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
        ensure(&layout, &Target::Map(map.clone()), false).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(
            pack.join("start.bat"),
            "RustDedicated.exe -batchmode +server.port 1234 +server.maxplayers 50 +decay.scale 0 +server.hostname \"Theirs\" +server.events true +evil %PATH% +server.pve \"true\"\r\n",
        )
        .unwrap();
        let b = bundled(&map).unwrap();
        assert_eq!(b.args, vec![("server.maxplayers".into(), "50".into()), ("decay.scale".into(), "0".into()), ("server.events".into(), "true".into()), ("server.pve".into(), "true".into())]);
        let (sf, rewritten) = ensure(&layout, &Target::Map(map.clone()), false).unwrap();
        assert!(rewritten);
        let text = std::fs::read_to_string(&sf.path).unwrap();
        assert!(text.contains("+server.maxplayers 50") && !text.contains("+server.maxplayers 10") && text.contains("+decay.scale 0"), "{text}");
        assert!(text.contains("+server.port 28015") && !text.contains("Theirs") && !text.contains("%PATH%"), "{text}");
        assert!(text.contains("rem Also uses the settings of start.bat"));
        // Prefab test mode keeps its own: events stay off.
        let prefab = std::fs::read_to_string(path_for(&server, &Target::Map(map.clone()), true)).unwrap();
        assert!(prefab.contains("+server.events false") && !prefab.contains("+server.events true"), "{prefab}");
        // Nothing new: left alone.
        assert!(!ensure(&layout, &Target::Map(map.clone()), false).unwrap().1);
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
        let (sf, rewritten) = ensure(&layout, &Target::Procgen, false).unwrap();
        assert!(rewritten && sf.current_template);
        assert_eq!(sf.rcon_password.as_deref(), Some("keepme"));
        assert!(sf.map.is_none());
        let text = std::fs::read_to_string(&sf.path).unwrap();
        assert!(text.contains("echo Procedural Rust test server (seed 1184645116, size 3000). Made with DropLaunch.gg"), "{text}");
        // The previous preview's wording ("made by DropLaunch") is upgraded too.
        std::fs::write(&sf.path, text.replace(TEMPLATE_MARK, "made by DropLaunch")).unwrap();
        assert!(!read(&sf.path).unwrap().current_template);
        assert!(ensure(&layout, &Target::Procgen, false).unwrap().1);
        // Map files from the preview versions (they took DROPLAUNCH_LEVELURL) are rewritten.
        let map = Target::Map(base.join("Maps").join("x.map"));
        let (sf, _) = ensure(&layout, &map, false).unwrap();
        let text = std::fs::read_to_string(&sf.path).unwrap();
        let old = text.replacen("cd /d", &format!("if defined {RETIRED_LEVELURL_ENV} set \"LEVEL_URL=%{RETIRED_LEVELURL_ENV}%\"\r\ncd /d"), 1);
        std::fs::write(&sf.path, old).unwrap();
        assert!(!read(&sf.path).unwrap().current_template);
        assert!(ensure(&layout, &map, false).unwrap().1);
        let _ = std::fs::remove_dir_all(base);
    }

    #[cfg(windows)]
    #[test]
    fn update_script_points_at_steamcmd_and_reapplies_oxide() {
        let base = temp_dir("upd");
        let server = base.join("Server_Files");
        std::fs::create_dir_all(&server).unwrap();
        let layout = Layout { base: &base, server_dir: &server, steamcmd_dir: &base.join("SteamCMD") };
        let path = write_update_script(&layout, "staging & del").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(r#"set "STEAMCMD=%~dp0..\SteamCMD\steamcmd.exe""#), "{text}");
        assert!(text.contains("-beta stagingdel %VALIDATE%"));
        // Not through: once more, checking every file.
        assert!(text.contains(r#"findstr /c:"StateFlags" "steamapps\appmanifest_258550.acf" 2>nul | findstr /c:"\"4\"" >nul && goto updated"#), "{text}");
        assert!(text.contains("-beta stagingdel validate +quit"));
        assert!(text.contains("findstr /m /c:\"Oxide.Core\""));
        assert_eq!(update_script_branch(&server).as_deref(), Some("stagingdel"));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn query_port_follows_rust() {
        let dir = Path::new("S");
        let parse_args = |args: &str| parse(&format!("RustDedicated.exe -batchmode {args}"), &dir.join("start_x.bat"), dir);
        // Not set: one above the game and RCON ports, like Rust picks it.
        assert_eq!(parse_args("+server.port 28015 +rcon.port 28016").query_port(), DEFAULT_QUERY_PORT);
        assert_eq!(parse_args("+server.port 28030 +rcon.port 28016").query_port(), 28031);
        assert_eq!(parse_args("+server.port 28015 +server.queryport 27100").query_port(), 27100);
        // Clashing with another port: Rust ignores it.
        assert_eq!(parse_args("+server.port 28015 +rcon.port 28016 +server.queryport 28016").query_port(), DEFAULT_QUERY_PORT);
        // Rust+ (what a real server opened next to 28015/28016).
        assert_eq!(parse_args("+server.port 28015 +rcon.port 28016").app_port(), DEFAULT_APP_PORT);
        assert_eq!(parse_args("+server.port 28015 +app.port 28100").app_port(), 28100);
    }

    #[test]
    fn passwords_are_random_and_sized() {
        let (a, b) = (random_password(12), random_password(12));
        assert_eq!(a.len(), 12);
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric()));
    }
}
