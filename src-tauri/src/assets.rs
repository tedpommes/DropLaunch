//! Dropped assets: what was dropped on DropLaunch (its window or its exe) — maps, zips with maps,
//! language files, kit files — kept in dropped.json in the data folder, newest first, each with
//! the DropLaunch folder it was dropped on (a DropLaunch in another folder doesn't list them).
//! Settings shows them by kind, with the start files of the server folders and the kits.
use std::path::Path;

use serde::{Deserialize, Serialize};

pub const DROPPED_FILE: &str = "dropped.json";
/// The newest this many are kept.
const KEEP: usize = 300;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Map,
    Zip,
    Language,
    Kit,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Dropped {
    pub kind: Kind,
    /// The file as it was dropped.
    pub path: String,
    /// When, seconds since 1970.
    pub added: i64,
    /// A zip's maps (where DropLaunch unpacked them); a language's or kit's own copy.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub made: Vec<String>,
    /// The DropLaunch folder it was dropped on ("": by an older version, before folders were kept).
    #[serde(default)]
    pub folder: String,
}

/// What was dropped on the DropLaunch in `folder` (`legacy`: also what older versions recorded
/// without a folder: only an entry from back then lists those).
pub fn load_for(data: &Path, folder: &str, legacy: bool) -> Vec<Dropped> {
    load(data).into_iter().filter(|d| d.folder.eq_ignore_ascii_case(folder) || (legacy && d.folder.is_empty())).collect()
}

fn load(data: &Path) -> Vec<Dropped> {
    std::fs::read_to_string(data.join(DROPPED_FILE)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

fn save(data: &Path, list: &[Dropped]) {
    let _ = std::fs::create_dir_all(data);
    if let Ok(text) = serde_json::to_string_pretty(list) {
        let file = data.join(DROPPED_FILE);
        let partial = file.with_extension("json.part");
        if std::fs::write(&partial, text).is_ok() {
            let _ = std::fs::rename(&partial, &file);
        }
    }
}

/// Dropped just now on the DropLaunch in `folder` (the same file dropped again moves to the top).
pub fn record(data: &Path, folder: &str, kind: Kind, path: &Path, made: Vec<String>) {
    let mut list = load(data);
    let path = path.display().to_string();
    list.retain(|d| !(d.kind == kind && d.path.eq_ignore_ascii_case(&path) && (d.folder.is_empty() || d.folder.eq_ignore_ascii_case(folder))));
    list.insert(0, Dropped { kind, path, added: now(), made, folder: folder.to_string() });
    list.truncate(KEEP);
    save(data, &list);
}

/// Taken off the list (the file itself stays).
pub fn forget(data: &Path, path: &str) {
    let mut list = load(data);
    list.retain(|d| !d.path.eq_ignore_ascii_case(path));
    save(data, &list);
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_first_and_once() {
        let data = std::env::temp_dir().join(format!("droplaunch-dropped-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        let here = r"C:\DropLaunch";
        record(&data, here, Kind::Map, Path::new(r"C:\a.map"), vec![]);
        record(&data, here, Kind::Zip, Path::new(r"C:\b.zip"), vec![r"C:\Maps\b\b.map".into()]);
        record(&data, here, Kind::Map, Path::new(r"c:\A.map"), vec![]);
        let list = load_for(&data, here, false);
        assert_eq!(list.iter().map(|d| d.path.as_str()).collect::<Vec<_>>(), [r"c:\A.map", r"C:\b.zip"]);
        assert_eq!(list[1].made.len(), 1);
        forget(&data, r"C:\B.ZIP");
        assert_eq!(load_for(&data, here, false).len(), 1);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn each_folder_lists_its_own() {
        let data = std::env::temp_dir().join(format!("droplaunch-dropped-folders-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        std::fs::create_dir_all(&data).unwrap();
        // An older version's entry, without a folder.
        std::fs::write(data.join(DROPPED_FILE), r#"[{"kind":"map","path":"C:\\old.map","added":1}]"#).unwrap();
        record(&data, r"C:\One", Kind::Map, Path::new(r"C:\one.map"), vec![]);
        record(&data, r"C:\Two", Kind::Map, Path::new(r"C:\two.map"), vec![]);
        let paths = |folder, legacy| load_for(&data, folder, legacy).into_iter().map(|d| d.path).collect::<Vec<_>>();
        // The folder that was used back then still has the old one; a new folder only its own.
        assert_eq!(paths(r"c:\one", true), [r"C:\one.map", r"C:\old.map"]);
        assert_eq!(paths(r"C:\Two", false), [r"C:\two.map"]);
        let _ = std::fs::remove_dir_all(&data);
    }
}
