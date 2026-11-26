//! Config files for add-ons (`configs:read`, `configs:write`): the server's own
//! (`server/<identity>/cfg`), Carbon's or Oxide's, their plugins' and Carbon's modules'. An add-on
//! names a file by its path inside a server folder, with forward slashes
//! (`carbon/configs/Kits.json`). Only paths in those places, ending in .cfg or .json, are taken,
//! and only when the file really is there (no link or junction leading elsewhere). Nothing new is
//! made, except a server.cfg in an identity folder that has none.

use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::mods::Framework;
use crate::t;

/// The biggest config file read or written (some loot tables are a few MB).
pub const MAX_SIZE: u64 = 8 * 1024 * 1024;
const BOM: &str = "\u{feff}";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// Rust's own: `server/<identity>/cfg/*.cfg` (and its .json files).
    Server,
    /// Carbon's or Oxide's own settings (`carbon/config*.json`, `oxide/*.json`).
    Framework,
    /// A plugin's (`carbon/configs`, `oxide/config`).
    Plugin,
    /// A Carbon module's (`carbon/modules/<module>/config.json`).
    Module,
}

/// A config file, as an add-on sees it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigFile {
    /// Inside the server folder, with forward slashes: what read and write take.
    pub path: String,
    pub kind: Kind,
    /// What it's for: the plugin's or module's name, else the file's name.
    pub name: String,
    /// The server identity (`server/<identity>`) of a server file.
    pub identity: Option<String>,
    /// A plugin's file (`carbon/plugins/<name>.cs`) when there is one: it can be reloaded by name.
    pub plugin_file: Option<String>,
    pub size: u64,
    /// When it was last written (ms): pass it back when writing.
    pub modified: i64,
}

/// A server folder and its config files.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    /// What read and write take as `folder`: the folder's branch.
    pub id: String,
    /// The folder's own name (not its whole path).
    pub name: String,
    pub framework: Framework,
    /// A server runs from this folder now.
    pub running: bool,
    /// The running server's identity (its server files are the ones in use).
    pub identity: Option<String>,
    /// Every `server/<identity>` folder: a server.cfg can be made in each.
    pub identities: Vec<String>,
    pub files: Vec<ConfigFile>,
}

/// Why a file can't be read or written: `code` for the add-on (as in `AddonError`).
#[derive(Debug, PartialEq)]
pub struct Refused {
    pub code: &'static str,
    pub message: String,
}

fn refused(code: &'static str, message: String) -> Refused {
    Refused { code, message }
}

/// One part of a path: a plain file or folder name.
fn plain(part: &str) -> bool {
    const DEVICES: [&str; 22] = [
        "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
    ];
    let stem = part.split('.').next().unwrap_or_default().trim_end().to_ascii_lowercase();
    !part.is_empty()
        && part.len() <= 120
        && part != "."
        && part != ".."
        && !part.ends_with(['.', ' '])
        && !part.chars().any(|c| c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\\' | '/'))
        && !DEVICES.contains(&stem.as_str())
}

fn ends_with(name: &str, ext: &str) -> bool {
    name.len() > ext.len() && name.to_ascii_lowercase().ends_with(ext)
}

fn stem(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(s, _)| s)
}

/// What a path is, when it's one of the config places (None: not one).
pub fn classify(path: &str) -> Option<(Kind, String, Option<String>)> {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() > 5 || !parts.iter().all(|p| plain(p)) {
        return None;
    }
    let lower: Vec<String> = parts.iter().map(|p| p.to_ascii_lowercase()).collect();
    let lower: Vec<&str> = lower.iter().map(String::as_str).collect();
    let json = |name: &str| ends_with(name, ".json");
    match (lower.as_slice(), parts.as_slice()) {
        (["server", _, "cfg", file], [_, identity, _, name]) if ends_with(file, ".cfg") || json(file) => Some((Kind::Server, name.to_string(), Some(identity.to_string()))),
        (["carbon", file], [_, name]) if file.starts_with("config") && json(file) => Some((Kind::Framework, name.to_string(), None)),
        (["oxide", file], [_, name]) if json(file) => Some((Kind::Framework, name.to_string(), None)),
        (["carbon", "configs", file] | ["oxide", "config", file], [_, _, name]) if json(file) => Some((Kind::Plugin, stem(name).to_string(), None)),
        // A plugin that keeps several files keeps them in a folder named after it.
        (["carbon", "configs", _, file] | ["oxide", "config", _, file], [_, _, plugin, _]) if json(file) => Some((Kind::Plugin, plugin.to_string(), None)),
        (["carbon", "modules", _, "config.json"], [_, _, module, _]) => Some((Kind::Module, module.to_string(), None)),
        _ => None,
    }
}

fn modified_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as i64)
}

fn same(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy())
    } else {
        a == b
    }
}

/// The file a path names, when it is a config file there, really inside `root` (no link or
/// junction on the way: its real path is the one asked for). `create`: it may be missing (its
/// folder has to be there).
fn resolve(root: &Path, path: &str, create: bool) -> Result<(PathBuf, Kind), Refused> {
    let bad = || refused("invalid", t!("configs.error.path", path));
    let (kind, _, _) = classify(path).ok_or_else(bad)?;
    let root = std::fs::canonicalize(root).map_err(|_| refused("notFound", t!("configs.error.missing", path)))?;
    let wanted = path.split('/').fold(root.clone(), |p, part| p.join(part));
    match std::fs::canonicalize(&wanted) {
        Ok(real) => {
            if !same(&real, &wanted) || !real.is_file() {
                return Err(bad());
            }
            Ok((real, kind))
        }
        Err(_) if create => {
            // Its folder has to be there, or (no cfg folder yet) the identity's folder: a real
            // one, not a link. Only the cfg folder is made.
            let parent = wanted.parent().ok_or_else(bad)?;
            let existing = if parent.is_dir() { parent } else { parent.parent().ok_or_else(bad)? };
            let real = std::fs::canonicalize(existing).map_err(|_| refused("notFound", t!("configs.error.missing", path)))?;
            if !same(&real, existing) || existing.strip_prefix(&root).map_or(true, |rel| rel.components().count() < 2) {
                return Err(bad());
            }
            Ok((wanted, kind))
        }
        Err(_) => Err(refused("notFound", t!("configs.error.missing", path))),
    }
}

/// The plain files in a folder (no links), by name.
fn files_in(dir: &Path) -> Vec<(String, std::fs::Metadata)> {
    let mut out: Vec<(String, std::fs::Metadata)> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_file() && !t.is_symlink()))
                .filter_map(|e| Some((e.file_name().to_str()?.to_string(), e.metadata().ok()?)))
                .collect()
        })
        .unwrap_or_default();
    out.sort_by_key(|(name, _)| name.to_lowercase());
    out
}

/// The plain folders in a folder (no links), by name.
fn dirs_in(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| entries.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir() && !t.is_symlink())).filter_map(|e| e.file_name().to_str().map(str::to_string)).collect())
        .unwrap_or_default();
    out.sort_by_key(|name| name.to_lowercase());
    out
}

/// Every config file in a server folder.
pub fn list(root: &Path) -> (Vec<ConfigFile>, Vec<String>) {
    let mut found: Vec<(String, std::fs::Metadata)> = Vec::new();
    let mut add = |rel: String, dir: &Path| {
        for (name, meta) in files_in(dir) {
            found.push((format!("{rel}/{name}"), meta));
        }
    };
    let identities = dirs_in(&root.join("server"));
    for identity in &identities {
        add(format!("server/{identity}/cfg"), &root.join("server").join(identity).join("cfg"));
    }
    for framework in ["carbon", "oxide"] {
        add(framework.to_string(), &root.join(framework));
    }
    for (configs, rel) in [(root.join("carbon").join("configs"), "carbon/configs"), (root.join("oxide").join("config"), "oxide/config")] {
        add(rel.to_string(), &configs);
        for sub in dirs_in(&configs) {
            add(format!("{rel}/{sub}"), &configs.join(&sub));
        }
    }
    for module in dirs_in(&root.join("carbon").join("modules")) {
        add(format!("carbon/modules/{module}"), &root.join("carbon").join("modules").join(&module));
    }
    // Only what really is in this folder: a linked folder (carbon/configs a junction to
    // elsewhere, say) isn't listed, as read and write wouldn't take it.
    let Ok(real_root) = std::fs::canonicalize(root) else { return (Vec::new(), identities) };
    let inside = |path: &str| {
        let wanted = path.split('/').fold(real_root.clone(), |p, part| p.join(part));
        std::fs::canonicalize(&wanted).is_ok_and(|real| same(&real, &wanted))
    };
    let mut files: Vec<ConfigFile> = found
        .into_iter()
        .filter(|(path, meta)| meta.len() <= MAX_SIZE && inside(path))
        .filter_map(|(path, meta)| {
            let (kind, name, identity) = classify(&path)?;
            let plugin_file = (kind == Kind::Plugin).then(|| plugin_file(root, &path, &name)).flatten();
            Some(ConfigFile { path, kind, name, identity, plugin_file, size: meta.len(), modified: modified_ms(&meta) })
        })
        .collect();
    files.sort_by_key(|f| (f.kind as u8, f.name.to_lowercase(), f.path.to_lowercase()));
    (files, identities)
}

/// The plugin a config belongs to, when its .cs is in the framework's plugins folder.
fn plugin_file(root: &Path, path: &str, name: &str) -> Option<String> {
    let framework = path.split('/').next()?;
    let rel = format!("{framework}/plugins/{name}.cs");
    root.join(framework).join("plugins").join(format!("{name}.cs")).is_file().then_some(rel)
}

/// What read gives: the text (without a byte order mark) and when it was written.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Contents {
    pub path: String,
    pub text: String,
    pub modified: i64,
    pub size: u64,
}

pub fn read(root: &Path, path: &str) -> Result<Contents, Refused> {
    let (file, _) = resolve(root, path, false)?;
    let meta = std::fs::metadata(&file).map_err(|e| refused("failed", t!("configs.error.read", path, error = e)))?;
    if meta.len() > MAX_SIZE {
        return Err(refused("invalid", t!("configs.error.tooBig", path)));
    }
    let bytes = std::fs::read(&file).map_err(|e| refused("failed", t!("configs.error.read", path, error = e)))?;
    let text = String::from_utf8(bytes).map_err(|_| refused("invalid", t!("configs.error.text", path)))?;
    let text = text.strip_prefix(BOM).map(str::to_string).unwrap_or(text);
    Ok(Contents { path: path.to_string(), text, modified: modified_ms(&meta), size: meta.len() })
}

/// What write gives back: the file as it is now.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Written {
    pub path: String,
    pub modified: i64,
    pub size: u64,
}

/// Write a config file. `modified`: when it was written when the add-on read it (it isn't written
/// over when it changed since); None: make it, only a server.cfg that isn't there. A .json file
/// has to be JSON. A byte order mark it had stays.
pub fn write(root: &Path, path: &str, text: &str, modified: Option<i64>) -> Result<Written, Refused> {
    let creating = modified.is_none();
    if creating && !(classify(path).is_some_and(|(kind, name, _)| kind == Kind::Server && name.eq_ignore_ascii_case("server.cfg"))) {
        return Err(refused("invalid", t!("configs.error.path", path)));
    }
    let (file, _) = resolve(root, path, creating)?;
    if text.len() as u64 > MAX_SIZE {
        return Err(refused("invalid", t!("configs.error.tooBig", path)));
    }
    // A .json file that is JSON stays JSON. Not every one is: Carbon's config.auto.json holds
    // console lines (`c.setting "value"`), like a .cfg file.
    let is_json = |bytes: &[u8]| serde_json::from_slice::<serde_json::Value>(bytes.strip_prefix(BOM.as_bytes()).unwrap_or(bytes)).is_ok();
    if ends_with(path, ".json") && std::fs::read(&file).is_ok_and(|b| is_json(&b)) {
        if let Err(e) = serde_json::from_str::<serde_json::Value>(text) {
            return Err(refused("invalid", t!("configs.error.json", error = e)));
        }
    }
    let mut bom = false;
    match (std::fs::metadata(&file), modified) {
        (Ok(_), None) => return Err(refused("changed", t!("configs.error.exists", path))),
        (Ok(meta), Some(expected)) => {
            if modified_ms(&meta) != expected {
                return Err(refused("changed", t!("configs.error.changed", path)));
            }
            bom = std::fs::read(&file).is_ok_and(|b| b.starts_with(BOM.as_bytes()));
        }
        (Err(_), Some(_)) => return Err(refused("notFound", t!("configs.error.missing", path))),
        (Err(_), None) => {
            if let Some(parent) = file.parent() {
                std::fs::create_dir_all(parent).map_err(|e| refused("failed", t!("configs.error.write", path, error = e)))?;
            }
        }
    }
    let whole = if bom { format!("{BOM}{text}") } else { text.to_string() };
    crate::whitelist::write_whole(&file, &whole).map_err(|e| refused("failed", t!("configs.error.write", path, error = e)))?;
    let meta = std::fs::metadata(&file).map_err(|e| refused("failed", t!("configs.error.write", path, error = e)))?;
    Ok(Written { path: path.to_string(), modified: modified_ms(&meta), size: meta.len() })
}

/// The server folder `id` names, among the ones there are.
pub fn folder_dir<'a>(folders: &'a [(String, PathBuf)], id: &str) -> Result<&'a Path, Refused> {
    folders.iter().find(|(branch, _)| branch == id).map(|(_, dir)| dir.as_path()).ok_or_else(|| refused("notFound", t!("configs.error.noFolder", folder = id)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("droplaunch-configs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(root: &Path, rel: &str, text: &str) {
        let file = rel.split('/').fold(root.to_path_buf(), |p, part| p.join(part));
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }

    #[test]
    fn only_config_places_are_taken() {
        assert_eq!(classify("server/droplaunch/cfg/server.cfg"), Some((Kind::Server, "server.cfg".into(), Some("droplaunch".into()))));
        assert_eq!(classify("carbon/config.json").map(|c| c.0), Some(Kind::Framework));
        assert_eq!(classify("oxide/oxide.config.json").map(|c| c.0), Some(Kind::Framework));
        assert_eq!(classify("carbon/configs/Kits.json").map(|c| (c.0, c.1)), Some((Kind::Plugin, "Kits".into())));
        assert_eq!(classify("oxide/config/Loot/Tables.json").map(|c| (c.0, c.1)), Some((Kind::Plugin, "Loot".into())));
        assert_eq!(classify("carbon/modules/GatherManager/config.json").map(|c| (c.0, c.1)), Some((Kind::Module, "GatherManager".into())));
        for bad in [
            "carbon/plugins/Kits.cs",
            "carbon/data/DropLaunch/whitelist.json",
            "carbon/modules/Admin/data.json",
            "carbon/configs/../plugins/x.json",
            "carbon/configs/./x.json",
            "carbon\\configs\\x.json",
            "carbon/configs/x.json.",
            "carbon/configs/CON.json",
            "carbon/configs/a/b/c.json",
            "server/x/cfg/../../../start.bat",
            "server/x/server.cfg",
            "carbon/lang/en/Kits.json",
            "C:/Windows/win.ini",
            "/carbon/configs/x.json",
            "",
        ] {
            assert!(classify(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn the_list_has_every_kind_and_the_plugins_files() {
        let root = scratch("list");
        put(&root, "server/droplaunch/cfg/serverauto.cfg", "server.pve \"False\"\n");
        put(&root, "server/droplaunch/cfg/relay_cfg.json", "{}");
        std::fs::create_dir_all(root.join("server").join("other")).unwrap();
        put(&root, "carbon/config.json", "{}");
        put(&root, "carbon/config.auto.json", "{}");
        put(&root, "carbon/configs/Kits.json", "{}");
        put(&root, "carbon/configs/Loot/Tables.json", "{}");
        put(&root, "carbon/plugins/Kits.cs", "class Kits {}");
        put(&root, "carbon/modules/GatherManager/config.json", "{}");
        put(&root, "carbon/modules/Admin/data.json", "{}");
        put(&root, "carbon/data/DropLaunch/key.json", "{}");
        let (files, identities) = list(&root);
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "server/droplaunch/cfg/relay_cfg.json",
                "server/droplaunch/cfg/serverauto.cfg",
                "carbon/config.auto.json",
                "carbon/config.json",
                "carbon/configs/Kits.json",
                "carbon/configs/Loot/Tables.json",
                "carbon/modules/GatherManager/config.json",
            ]
        );
        assert_eq!(identities, ["droplaunch", "other"]);
        let kits = files.iter().find(|f| f.name == "Kits").unwrap();
        assert_eq!(kits.plugin_file.as_deref(), Some("carbon/plugins/Kits.cs"));
        assert!(files.iter().find(|f| f.name == "Loot").unwrap().plugin_file.is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn writing_needs_the_file_as_it_was_read() {
        let root = scratch("write");
        put(&root, "carbon/configs/Kits.json", "\u{feff}{\r\n  \"A\": 1\r\n}");
        let read_now = read(&root, "carbon/configs/Kits.json").unwrap();
        assert_eq!(read_now.text, "{\r\n  \"A\": 1\r\n}", "the byte order mark isn't handed out");
        // Not JSON: not written.
        assert_eq!(write(&root, "carbon/configs/Kits.json", "{\"A\": ", Some(read_now.modified)).unwrap_err().code, "invalid");
        // Changed since it was read: not written over.
        assert_eq!(write(&root, "carbon/configs/Kits.json", "{}", Some(read_now.modified - 5000)).unwrap_err().code, "changed");
        let written = write(&root, "carbon/configs/Kits.json", "{\r\n  \"A\": 2\r\n}", Some(read_now.modified)).unwrap();
        let bytes = std::fs::read(root.join("carbon").join("configs").join("Kits.json")).unwrap();
        assert_eq!(bytes, "\u{feff}{\r\n  \"A\": 2\r\n}".as_bytes(), "the byte order mark stays");
        assert_eq!(read(&root, "carbon/configs/Kits.json").unwrap().modified, written.modified);
        // Only a missing server.cfg is made, nothing else.
        std::fs::create_dir_all(root.join("server").join("droplaunch")).unwrap();
        assert!(write(&root, "server/droplaunch/cfg/server.cfg", "server.pve true\n", None).is_ok());
        assert_eq!(write(&root, "server/droplaunch/cfg/server.cfg", "x", None).unwrap_err().code, "changed");
        assert_eq!(write(&root, "carbon/configs/New.json", "{}", None).unwrap_err().code, "invalid");
        assert_eq!(write(&root, "server/nobody/cfg/server.cfg", "x", None).unwrap_err().code, "notFound");
        assert_eq!(read(&root, "carbon/configs/Gone.json").unwrap_err().code, "notFound");
        assert_eq!(read(&root, "carbon/plugins/Kits.cs").unwrap_err().code, "invalid");
        // Carbon's config.auto.json isn't JSON (console lines): it stays what it is.
        put(&root, "carbon/config.auto.json", "c.recycletickmultiplier \"-1\"\n");
        let auto = read(&root, "carbon/config.auto.json").unwrap();
        assert!(write(&root, "carbon/config.auto.json", "c.recycletickmultiplier \"2\"\n", Some(auto.modified)).is_ok());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A junction in the server folder that leads elsewhere isn't followed.
    #[cfg(windows)]
    #[test]
    fn a_junction_out_of_the_folder_is_refused() {
        let root = scratch("junction");
        let outside = scratch("junction-outside");
        put(&outside, "Kits.json", "{}");
        std::fs::create_dir_all(root.join("carbon")).unwrap();
        let made = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(root.join("carbon").join("configs")).arg(&outside).output().unwrap();
        assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
        assert_eq!(read(&root, "carbon/configs/Kits.json").unwrap_err().code, "invalid");
        assert!(list(&root).0.is_empty(), "the junction isn't listed");
        let _ = std::fs::remove_dir(root.join("carbon").join("configs"));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }
}
