//! Downloads with progress, and unpacking .zip / .tar.gz into a folder.

use crate::t;
use anyhow::{bail, Context, Result};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use crate::steamcmd::CancelToken;

/// Download `url` to `dest` (via a .part file); `progress(done, total)` is called as bytes arrive.
pub fn download(url: &str, dest: &Path, cancel: &CancelToken, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<()> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(20))
        .timeout_read(Duration::from_secs(60))
        .build();
    let response = agent.get(url).call().with_context(|| t!("error.download", url))?;
    let total = response.header("Content-Length").and_then(|v| v.parse::<u64>().ok());
    let mut reader = response.into_reader();

    let partial = dest.with_extension("part");
    let mut file = File::create(&partial).with_context(|| t!("error.write", file = partial.display()))?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    let mut last_report = Instant::now();
    loop {
        if cancel.is_cancelled() {
            drop(file);
            let _ = fs::remove_file(&partial);
            bail!(t!("error.cancelled"));
        }
        let n = reader.read(&mut buf).context(t!("error.interrupted"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        done += n as u64;
        if last_report.elapsed() >= Duration::from_millis(150) {
            progress(done, total);
            last_report = Instant::now();
        }
    }
    progress(done, total);
    file.flush()?;
    drop(file);
    fs::rename(&partial, dest)?;
    Ok(())
}

/// Unpack a .zip or .tar.gz into `dir`, overwriting existing files.
pub fn extract(archive: &Path, dir: &Path) -> Result<()> {
    let name = archive.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_ascii_lowercase();
    if name.ends_with(".zip") {
        extract_zip(archive, dir)
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        extract_tar_gz(archive, dir)
    } else {
        bail!(t!("error.unpackUnknown", name))
    }
}

fn extract_zip(archive: &Path, dir: &Path) -> Result<()> {
    let mut zip = zip::ZipArchive::new(File::open(archive)?).with_context(|| t!("error.badZip", file = archive.display()))?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let Some(relative) = entry.enclosed_name() else { continue }; // refuses "../" paths
        let target = dir.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut out = File::create(&target).with_context(|| t!("error.write", file = target.display()))?;
        std::io::copy(&mut entry, &mut out)?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&target, fs::Permissions::from_mode(mode))?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn extract_tar_gz(archive: &Path, dir: &Path) -> Result<()> {
    let gz = flate2::read::GzDecoder::new(File::open(archive)?);
    tar::Archive::new(gz).unpack(dir).with_context(|| t!("error.unpack", file = archive.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn extract_tar_gz(archive: &Path, _dir: &Path) -> Result<()> {
    bail!(t!("error.linuxArchive", file = archive.display()))
}
