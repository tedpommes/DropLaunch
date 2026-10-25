//! The map list: every .map in the Maps folder (subfolders included), maps started from other
//! folders, and turning dropped files (.map, .zip with maps, folders) into maps.

use crate::t;
use serde::Serialize;
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::startfile::{self, Target};

const MAX_DEPTH: usize = 8;
const MAX_MAPS: usize = 5000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapEntry {
    pub path: String,
    pub name: String,
    /// Subfolder inside the Maps folder ("" for the top level), or the full folder for maps elsewhere.
    pub folder: String,
    pub size: u64,
    pub modified: i64,
    /// Its start files in the server folder (normal, prefab test mode), once made.
    pub start_file: Option<String>,
    pub prefab_file: Option<String>,
    /// Outside the Maps folder (dropped from somewhere else).
    pub outside: bool,
    /// History entries: when it was last started (seconds since 1970, 0 if unknown).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started: Option<i64>,
}

pub fn is_map(path: &Path) -> bool {
    has_ext(path, "map")
}

fn has_ext(path: &Path, ext: &str) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

/// The maps' start files in a server folder (normal, prefab test mode), read once for the whole
/// list instead of looking for two files per map.
#[derive(Default)]
pub struct StartFiles(HashMap<String, (Option<String>, Option<String>)>);

impl StartFiles {
    pub fn read(server_dir: &Path) -> Self {
        let mut found: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
        for sf in startfile::list(server_dir) {
            let Some(map) = sf.map.clone() else { continue };
            // Only a map's own files (`start_<map>.bat`, `..._prefab.bat`): two maps can share a name.
            let target = Target::Map(map.clone());
            let slot = found.entry(path_key(&map)).or_default();
            if startfile::same_path(&sf.path, &startfile::path_for(server_dir, &target, false)) {
                slot.0 = Some(sf.path.display().to_string());
            } else if startfile::same_path(&sf.path, &startfile::path_for(server_dir, &target, true)) {
                slot.1 = Some(sf.path.display().to_string());
            }
        }
        Self(found)
    }

    fn of(&self, map: &Path) -> (Option<String>, Option<String>) {
        self.0.get(&path_key(map)).cloned().unwrap_or_default()
    }
}

/// A map path as a lookup key (Windows paths don't care about case).
fn path_key(path: &Path) -> String {
    let text = startfile::normalize(path).to_string_lossy().into_owned();
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    }
}

pub fn entry(path: &Path, maps_dir: &Path, start_files: &StartFiles) -> Option<MapEntry> {
    let meta = std::fs::metadata(path).ok().filter(|m| m.is_file())?;
    let outside = !startfile::is_inside(path, maps_dir);
    let parent = path.parent().unwrap_or(Path::new(""));
    let folder = if outside {
        parent.display().to_string()
    } else {
        parent.strip_prefix(maps_dir).map(|p| p.display().to_string()).unwrap_or_default()
    };
    let (start_file, prefab_file) = start_files.of(path);
    Some(MapEntry {
        path: path.display().to_string(),
        name: startfile::map_title(path),
        folder,
        size: meta.len(),
        modified: meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64),
        start_file,
        prefab_file,
        outside,
        started: None,
    })
}

/// Every .map below `maps_dir`, newest first.
pub fn scan(maps_dir: &Path, start_files: &StartFiles) -> Vec<MapEntry> {
    let mut found = Vec::new();
    walk(maps_dir, 0, &mut found);
    let mut maps: Vec<MapEntry> = found.iter().filter_map(|p| entry(p, maps_dir, start_files)).collect();
    maps.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.name.cmp(&b.name)));
    maps
}

/// Every .map below `dir` (the same folders the list shows).
pub fn find(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    walk(dir, 0, &mut found);
    found
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if out.len() >= MAX_MAPS {
            return;
        }
        let path = entry.path();
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            let hidden = path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.'));
            if depth < MAX_DEPTH && !hidden {
                walk(&path, depth + 1, out);
            }
        } else if is_map(&path) {
            out.push(path);
        }
    }
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Imported {
    /// Maps ready to start, in drop order.
    pub maps: Vec<String>,
    pub problems: Vec<String>,
    /// Other files a .zip held, unpacked next to its maps: .json files (kits, language files:
    /// the UI hands them on) ...
    pub files: Vec<String>,
    /// ... and start files (their settings go into the map's own start files).
    pub start_files: Vec<String>,
}

/// Dropped files: .map files are used where they are (like v1, so a re-export is picked up on
/// the next start), maps inside a .zip are unpacked into `Maps/<zip name>/`, folders are searched.
pub fn import(paths: &[PathBuf], maps_dir: &Path) -> Imported {
    let mut out = Imported::default();
    for path in paths {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
        if path.is_dir() {
            let mut found = Vec::new();
            walk(path, 0, &mut found);
            if found.is_empty() {
                out.problems.push(t!("maps.import.noMaps", name));
            }
            found.sort();
            out.maps.extend(found.iter().map(|p| p.display().to_string()));
        } else if is_map(path) {
            if path.is_file() {
                out.maps.push(path.display().to_string());
            } else {
                out.problems.push(t!("maps.import.notFound", name));
            }
        } else if has_ext(path, "zip") {
            match unzip_maps(path, maps_dir) {
                Ok(z) if z.maps.is_empty() && z.files.is_empty() => out.problems.push(t!("maps.import.emptyZip", name)),
                Ok(z) => {
                    out.maps.extend(z.maps.iter().map(|p| p.display().to_string()));
                    out.files.extend(z.files.iter().map(|p| p.display().to_string()));
                    out.start_files.extend(z.start_files.iter().map(|p| p.display().to_string()));
                }
                Err(e) => out.problems.push(format!("{name}: {e:#}")),
            }
        } else {
            out.problems.push(t!("maps.import.notMap", name));
        }
    }
    out
}

/// What a .zip held, unpacked into `Maps/<zip name>/`.
#[derive(Default)]
struct Unzipped {
    maps: Vec<PathBuf>,
    /// .json files (a kit, a language file).
    files: Vec<PathBuf>,
    /// Start files (.bat / .sh that run RustDedicated).
    start_files: Vec<PathBuf>,
}

/// Small files besides maps are taken along (bigger ones aren't settings or texts).
const MAX_JSON: u64 = 4 * 1024 * 1024;
const MAX_SCRIPT: u64 = 256 * 1024;

fn unzip_maps(zip_path: &Path, maps_dir: &Path) -> anyhow::Result<Unzipped> {
    let mut zip = zip::ZipArchive::new(File::open(zip_path)?)?;
    let stem = zip_path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "zip".into());
    let dest_dir = maps_dir.join(stem.trim());
    let mut out = Unzipped::default();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let Some(inner) = entry.enclosed_name() else { continue };
        if entry.is_dir() {
            continue;
        }
        let json = has_ext(&inner, "json") && entry.size() <= MAX_JSON;
        let script = (has_ext(&inner, "bat") || has_ext(&inner, "sh")) && entry.size() <= MAX_SCRIPT;
        if !is_map(&inner) && !json && !script {
            continue;
        }
        let Some(file_name) = inner.file_name() else { continue };
        if script {
            // Only a start file for a Rust server.
            let mut text = String::new();
            if std::io::Read::read_to_string(&mut entry, &mut text).is_err() || !text.contains("RustDedicated") {
                continue;
            }
            std::fs::create_dir_all(&dest_dir)?;
            let dest = dest_dir.join(file_name);
            std::fs::write(&dest, text)?;
            out.start_files.push(dest);
            continue;
        }
        std::fs::create_dir_all(&dest_dir)?;
        let dest = dest_dir.join(file_name);
        let mut file = File::create(&dest)?;
        std::io::copy(&mut entry, &mut file)?;
        if json {
            out.files.push(dest);
        } else {
            out.maps.push(dest);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("droplaunch-maps-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn scans_subfolders_and_skips_other_files() {
        let base = temp_dir("scan");
        let maps = base.join("Maps");
        std::fs::create_dir_all(maps.join("Tests").join("Old")).unwrap();
        std::fs::create_dir_all(maps.join(".git")).unwrap();
        std::fs::write(maps.join("top.map"), b"a").unwrap();
        std::fs::write(maps.join("Tests").join("Old").join("deep.MAP"), b"bb").unwrap();
        std::fs::write(maps.join("notes.txt"), b"x").unwrap();
        std::fs::write(maps.join(".git").join("hidden.map"), b"x").unwrap();
        let found = scan(&maps, &StartFiles::read(&base.join("Server_Files")));
        let mut names: Vec<_> = found.iter().map(|m| (m.name.as_str(), m.folder.as_str())).collect();
        names.sort();
        let deep = Path::new("Tests").join("Old").display().to_string();
        assert_eq!(names, vec![("deep", deep.as_str()), ("top", "")]);
        assert!(found.iter().all(|m| !m.outside && m.start_file.is_none()));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn start_files_belong_to_their_own_map() {
        let base = temp_dir("starts");
        let maps = base.join("Maps");
        let server = base.join("Server_Files");
        std::fs::create_dir_all(maps.join("Old")).unwrap();
        std::fs::create_dir_all(&server).unwrap();
        std::fs::write(maps.join("Race.map"), b"new").unwrap();
        std::fs::write(maps.join("Old").join("Race.map"), b"old").unwrap();
        let layout = startfile::Layout { base: &base, server_dir: &server, steamcmd_dir: &base.join("SteamCMD") };
        startfile::ensure(&layout, &Target::Map(maps.join("Race.map")), false).unwrap();
        let found = scan(&maps, &StartFiles::read(&server));
        let race = found.iter().find(|m| m.folder.is_empty()).unwrap();
        assert!(race.start_file.is_some() && race.prefab_file.is_some(), "{race:?}");
        // The same name in another folder: not its files.
        let old = found.iter().find(|m| m.folder == "Old").unwrap();
        assert!(old.start_file.is_none() && old.prefab_file.is_none());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn imports_maps_from_zips_and_folders() {
        let base = temp_dir("import");
        let maps = base.join("Maps");
        let zip_path = base.join("Cool Map v3.zip");
        {
            let mut zip = zip::ZipWriter::new(File::create(&zip_path).unwrap());
            let opts = zip::write::SimpleFileOptions::default();
            zip.start_file("Cool Map v3/cool.map", opts).unwrap();
            zip.write_all(b"MAP").unwrap();
            zip.start_file("Cool Map v3/readme.txt", opts).unwrap();
            zip.write_all(b"hi").unwrap();
            zip.start_file("Cool Map v3/Nederlands.json", opts).unwrap();
            zip.write_all(b"{}").unwrap();
            zip.start_file("Cool Map v3/start_cool.bat", opts).unwrap();
            zip.write_all(b"RustDedicated.exe -batchmode +server.maxplayers 50").unwrap();
            zip.start_file("Cool Map v3/install.bat", opts).unwrap();
            zip.write_all(b"del *.*").unwrap();
            zip.finish().unwrap();
        }
        let loose = base.join("loose.map");
        std::fs::write(&loose, b"m").unwrap();
        let result = import(&[zip_path, loose.clone(), base.join("other.txt")], &maps);
        assert_eq!(result.maps.len(), 2, "{result:?}");
        assert!(maps.join("Cool Map v3").join("cool.map").is_file());
        assert_eq!(result.maps[1], loose.display().to_string());
        // What else it held: the .json for the UI, the server's start file next to the map.
        assert_eq!(result.files, vec![maps.join("Cool Map v3").join("Nederlands.json").display().to_string()]);
        assert_eq!(result.start_files, vec![maps.join("Cool Map v3").join("start_cool.bat").display().to_string()]);
        assert!(!maps.join("Cool Map v3").join("install.bat").exists() && !maps.join("Cool Map v3").join("readme.txt").exists());
        assert_eq!(result.problems.len(), 1);
        let _ = std::fs::remove_dir_all(base);
    }
}
