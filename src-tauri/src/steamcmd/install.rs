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
