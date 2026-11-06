//! What older versions kept in the user's app data, moved into the data folder next to the exe
//! (`DropLaunch-data`) the first time a folder runs this version: `%LOCALAPPDATA%\DropLaunch`
//! (settings, kits, item icons, languages, themes, start files, bug reports, map pictures, the
//! log…) and the window's storage in `%LOCALAPPDATA%\gg.droplaunch.desktop` (WebView2's: the
//! layout and the other choices the UI keeps there). The settings file there has an entry per
//! folder DropLaunch ran from, and only this folder's comes along. The rest was shared by all of
//! them, so each folder takes a copy and the last one moves it and removes the old folders (a
//! folder without a DropLaunch exe in it any more doesn't count). A folder that never ran an
//! older version starts new and takes nothing.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::client;
use crate::config::{folder_key, still_there, Settings, SETTINGS_FILE, WEBVIEW_DIR};

/// The app's identifier (tauri.conf.json): WebView2's storage was named after it.
const IDENTIFIER: &str = "gg.droplaunch.desktop";
/// Not taken along: the settings (only this folder's entry is) and the notification icon
/// (written again when it's needed).
const LEFT: [&str; 4] = [SETTINGS_FILE, "settings.json.tmp", "settings.json.bad", "notification-icon.png"];
/// The folder WebView2 makes in its storage folder.
const PROFILE: &str = "EBWebView";

/// DropLaunch's folder in the user's app data: where older versions kept everything, and where
/// it goes when the exe's folder can't be written to.
pub fn dir() -> Option<PathBuf> {
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("DropLaunch"))
    } else if cfg!(target_os = "macos") {
        home().map(|h| h.join("Library").join("Application Support").join("DropLaunch"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| home().map(|h| h.join(".config"))).map(|d| d.join("droplaunch"))
    }
}

/// Moves this folder's things from `old` (an older version's app data) into `data`, once: when
/// `data` has no settings yet and `old`'s have an entry for this folder. Returns whether it did.
pub fn move_in(base: &Path, data: &Path, old: &Path) -> bool {
    let file = data.join(SETTINGS_FILE);
    let old_file = old.join(SETTINGS_FILE);
    if file.exists() {
        return false;
    }
    let Ok(text) = fs::read_to_string(&old_file) else { return false };
    let Ok(mut settings) = serde_json::from_str::<Settings>(text.trim_start_matches('\u{feff}')) else { return false };
    let key = folder_key(base);
    let Some(entry) = settings.folders.remove(&key) else { return false };
    // No other folder could still use it: this one takes it all.
    let last = !settings.folders.keys().any(|folder| could_use(Path::new(folder)));

    let mut all = fs::create_dir_all(data).is_ok();
    for item in fs::read_dir(old).into_iter().flatten().flatten() {
        let name = item.file_name();
        if LEFT.iter().any(|left| name == *left) {
            continue;
        }
        // Rust's own maps put aside aren't any folder's (and they're big): they go with the first.
        let done = if last || name == client::BACKUP_DIR { move_to(&item.path(), &data.join(&name)) } else { copy(&item.path(), &data.join(&name)) };
        all &= done.is_ok();
    }
    // The window's storage goes to the first folder, moved whole (never read: it's a browser's
    // profile). Where it can't be (another drive), the window starts with its defaults.
    let old_webview = old.parent().filter(|_| cfg!(windows)).map(|local| local.join(IDENTIFIER));
    if let Some(old_webview) = &old_webview {
        let (from, to) = (old_webview.join(PROFILE), data.join(WEBVIEW_DIR).join(PROFILE));
        if from.is_dir() && !to.exists() && fs::create_dir_all(data.join(WEBVIEW_DIR)).is_ok() && fs::rename(&from, &to).is_ok() {
            let _ = fs::remove_dir(old_webview);
        }
    }

    // Written before the old entry goes: if it can't be, the old one stays for the next start.
    let own = Settings { version: 1, folders: [(key, entry)].into() };
    if write(&file, &own).is_err() {
        return false;
    }
    if last && all {
        let _ = fs::remove_dir_all(old);
        if let Some(old_webview) = old_webview {
            let _ = fs::remove_dir_all(old_webview);
        }
    } else {
        let _ = write(&old_file, &settings);
    }
    true
}

/// A folder that ran an older version could still use the old data: a DropLaunch exe is in it,
/// or its drive (a USB stick, a network share) is only away.
fn could_use(folder: &Path) -> bool {
    if !folder.is_dir() {
        return still_there(folder);
    }
    fs::read_dir(folder).into_iter().flatten().flatten().any(|item| {
        let name = item.file_name().to_string_lossy().to_lowercase();
        name.starts_with("droplaunch") && name.ends_with(".exe")
    })
}

fn write(file: &Path, settings: &Settings) -> anyhow::Result<()> {
    let tmp = file.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(settings)?)?;
    fs::rename(&tmp, file)?;
    Ok(())
}

/// Copies a file, or a folder with everything in it. What's there already stays; the rest is
/// copied even when something can't be.
fn copy(from: &Path, to: &Path) -> io::Result<()> {
    if !from.is_dir() {
        return if to.exists() { Ok(()) } else { fs::copy(from, to).map(|_| ()) };
    }
    fs::create_dir_all(to)?;
    let mut result = Ok(());
    for item in fs::read_dir(from)? {
        let item = item?;
        if let Err(e) = copy(&item.path(), &to.join(item.file_name())) {
            result = Err(e);
        }
    }
    result
}

/// Moves a file or folder: renamed (at once, on the same drive), else copied and removed.
fn move_to(from: &Path, to: &Path) -> io::Result<()> {
    if !to.exists() && fs::rename(from, to).is_ok() {
        return Ok(());
    }
    copy(from, to)?;
    if from.is_dir() {
        fs::remove_dir_all(from)
    } else {
        fs::remove_file(from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigStore;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("droplaunch-appdata-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn entry(branch: &str) -> serde_json::Value {
        serde_json::json!({ "branch": branch, "lastUsed": 1 })
    }

    /// An older version's app data, `Local\DropLaunch`, with entries for `folders` and its
    /// window storage next to it.
    fn old_app_data(local: &Path, folders: &[(&Path, &str)]) -> PathBuf {
        let old = local.join("DropLaunch");
        let settings = Settings { version: 1, folders: folders.iter().map(|(folder, branch)| (folder_key(folder), entry(branch))).collect() };
        fs::create_dir_all(old.join("Kits")).unwrap();
        fs::write(old.join(SETTINGS_FILE), serde_json::to_string(&settings).unwrap()).unwrap();
        fs::write(old.join("Kits").join("raid.json"), "{}").unwrap();
        fs::write(old.join("dropped.json"), "[]").unwrap();
        fs::write(old.join("notification-icon.png"), "png").unwrap();
        fs::create_dir_all(old.join(client::BACKUP_DIR).join("2026-11-01")).unwrap();
        fs::write(old.join(client::BACKUP_DIR).join("2026-11-01").join("Island_1.map"), "map").unwrap();
        let storage = local.join(IDENTIFIER).join(PROFILE).join("Default");
        fs::create_dir_all(&storage).unwrap();
        fs::write(storage.join("Preferences"), "layout").unwrap();
        old
    }

    /// A folder with a DropLaunch in it.
    fn droplaunch_folder(dir: &Path) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("DropLaunch.exe"), "").unwrap();
    }

    #[test]
    fn each_folder_takes_its_settings_and_a_copy_and_the_last_one_moves_the_rest() {
        let root = temp_dir("move");
        let (local, one, two) = (root.join("Local"), root.join("One"), root.join("Two"));
        droplaunch_folder(&one);
        droplaunch_folder(&two);
        let old = old_app_data(&local, &[(&one, "staging"), (&two, "public")]);
        let storage = |data: &Path| data.join(WEBVIEW_DIR).join(PROFILE).join("Default").join("Preferences");

        let data_one = one.join(crate::config::DATA_FOLDER);
        assert!(move_in(&one, &data_one, &old));
        assert!(data_one.join("Kits").join("raid.json").is_file());
        assert!(data_one.join(client::BACKUP_DIR).join("2026-11-01").join("Island_1.map").is_file());
        assert!(!old.join(client::BACKUP_DIR).exists(), "the put-aside maps go with the first");
        assert!(!data_one.join("notification-icon.png").exists());
        assert_eq!(fs::read_to_string(storage(&data_one)).unwrap(), "layout", "the window's storage goes with the first");
        assert!(!local.join(IDENTIFIER).exists());
        let store = ConfigStore::load_from(one.clone(), data_one.clone());
        assert_eq!(store.config.branch, "staging");
        // The other folder's things are still there for it, without the first one's entry.
        let left: Settings = serde_json::from_str(&fs::read_to_string(old.join(SETTINGS_FILE)).unwrap()).unwrap();
        assert_eq!(left.folders.keys().collect::<Vec<_>>(), [&folder_key(&two)]);
        assert!(old.join("Kits").join("raid.json").is_file());
        // Once is enough.
        assert!(!move_in(&one, &data_one, &old));

        // A folder that never ran DropLaunch takes nothing.
        let three = root.join("Three");
        assert!(!move_in(&three, &three.join(crate::config::DATA_FOLDER), &old));

        let data_two = two.join(crate::config::DATA_FOLDER);
        assert!(move_in(&two, &data_two, &old));
        assert!(data_two.join("Kits").join("raid.json").is_file());
        assert_eq!(ConfigStore::load_from(two.clone(), data_two.clone()).config.branch, "public");
        assert!(!storage(&data_two).exists());
        assert!(!old.exists(), "the old folder is gone");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn folders_that_are_gone_or_without_droplaunch_dont_keep_the_old_data() {
        let root = temp_dir("gone");
        let (local, one, emptied) = (root.join("Local"), root.join("One"), root.join("Emptied"));
        droplaunch_folder(&one);
        fs::create_dir_all(&emptied).unwrap();
        let old = old_app_data(&local, &[(&one, "staging"), (&root.join("Removed"), "public"), (&emptied, "public")]);
        assert!(move_in(&one, &one.join(crate::config::DATA_FOLDER), &old));
        assert!(!old.exists() && !local.join(IDENTIFIER).exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_moved_folder_keeps_its_settings() {
        let root = temp_dir("moved");
        let (before, after) = (root.join("Before"), root.join("After"));
        let mut store = ConfigStore::load_from(before.clone(), before.join(crate::config::DATA_FOLDER));
        store.config.branch = "staging".into();
        store.save().unwrap();
        fs::rename(&before, &after).unwrap();
        let store = ConfigStore::load_from(after.clone(), after.join(crate::config::DATA_FOLDER));
        assert_eq!(store.config.branch, "staging");
        store.save().unwrap();
        let settings: Settings = serde_json::from_str(&fs::read_to_string(after.join(crate::config::DATA_FOLDER).join(SETTINGS_FILE)).unwrap()).unwrap();
        assert_eq!(settings.folders.keys().collect::<Vec<_>>(), [&folder_key(&after)]);
        let _ = fs::remove_dir_all(root);
    }
}
