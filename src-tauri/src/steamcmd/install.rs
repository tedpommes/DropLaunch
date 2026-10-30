//! Downloading and unpacking SteamCMD itself.

use crate::t;
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

use super::CancelToken;
use crate::archive;

#[cfg(windows)]
pub const EXECUTABLE: &str = "steamcmd.exe";
#[cfg(not(windows))]
pub const EXECUTABLE: &str = "steamcmd.sh";

#[cfg(windows)]
const DOWNLOAD_URL: &str = "https://steamcdn-a.akamaihd.net/client/installer/steamcmd.zip";
#[cfg(target_os = "linux")]
const DOWNLOAD_URL: &str = "https://steamcdn-a.akamaihd.net/client/installer/steamcmd_linux.tar.gz";
#[cfg(target_os = "macos")]
const DOWNLOAD_URL: &str = "https://steamcdn-a.akamaihd.net/client/installer/steamcmd_osx.tar.gz";

pub fn executable(dir: &Path) -> PathBuf {
    dir.join(EXECUTABLE)
}

pub fn is_installed(dir: &Path) -> bool {
    executable(dir).is_file()
}

/// SteamCMD replaces its own files when it updates itself and leaves the old ones behind
/// (`steamcmd.exe.old`, `crashhandler.dll.old`, `steamcmd.exe.1.delete`): they're of no use
/// afterwards. One that's still in use can't be removed and just stays until next time.
pub fn tidy(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        let binary = name.contains(".exe.") || name.contains(".dll.");
        if binary && (name.ends_with(".old") || name.ends_with(".delete")) && entry.path().is_file() {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Download and unpack SteamCMD into `dir`; `progress(done, total)` is called while downloading.
pub fn install(dir: &Path, cancel: &CancelToken, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<()> {
    fs::create_dir_all(dir).with_context(|| t!("error.create", dir = dir.display()))?;
    let name = DOWNLOAD_URL.rsplit('/').next().unwrap_or("steamcmd.zip");
    let file = dir.join(name);
    archive::download(DOWNLOAD_URL, &file, cancel, progress)?;
    let unpacked = archive::extract(&file, dir);
    let _ = fs::remove_file(&file);
    unpacked?;
    if !is_installed(dir) {
        bail!(t!("error.steamcmdMissingExe", exe = EXECUTABLE));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn tidy_removes_only_leftovers() {
        let dir = std::env::temp_dir().join(format!("droplaunch-steamcmd-tidy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let leftovers = ["steamcmd.exe.old", "crashhandler.dll.old", "steamcmd.exe.1.delete"];
        let kept = ["steamcmd.exe", "steamclient.dll", "notes.old", "libraryfolder.vdf"];
        for name in leftovers.iter().chain(&kept) {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        super::tidy(&dir);
        for name in leftovers {
            assert!(!dir.join(name).exists(), "{name}");
        }
        for name in kept {
            assert!(dir.join(name).exists(), "{name}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
