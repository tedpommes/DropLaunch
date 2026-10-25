//! DropLaunch updates itself from GitHub Releases.
//!
//! Every release has a `latest.json` next to `DropLaunch.exe`: the version, release notes, the
//! exe's size and BLAKE3 hash, and an Ed25519 signature over those made with DropLaunch's release
//! key (kept offline by the author; `examples/release.rs` makes it and signs releases). DropLaunch
//! reads `.../releases/latest/download/latest.json`, checks the signature with the public key
//! built in, downloads the exe next to itself, checks its size and hash, swaps it in (a running
//! exe can be renamed but not overwritten) and starts the new one. So a changed release on GitHub
//! without the key can't put anything on users' PCs.
//!
//! The check is the only time DropLaunch asks the internet for something without being told to,
//! and it can be switched off (`updateCheck`).

use crate::t;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const REPO: &str = "tedpommes/DropLaunch";
/// The exe in a release, and the file that describes the release.
pub const EXE_NAME: &str = "DropLaunch.exe";
pub const MANIFEST_NAME: &str = "latest.json";
/// Points the check at another latest.json (testing).
const URL_ENV: &str = "DROPLAUNCH_UPDATE_URL";
/// DropLaunch's release key (public half). The private half signs releases.
const PUBLIC_KEY: &str = "cf23697e51aa6991383dab98c0923f9193b56333cf8ddf47babe97b08a29334e";
/// Signatures are over this, so they can't be mistaken for anything else.
const CONTEXT: &str = "DropLaunch update v1";
/// Argument for the new exe: wait for this process (the old DropLaunch) to close first.
pub const AFTER_UPDATE_ARG: &str = "--after-update";

/// `latest.json`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub version: String,
    #[serde(default)]
    pub notes: String,
    /// Release date, e.g. 2026-10-09.
    #[serde(default)]
    pub published: String,
    pub url: String,
    pub size: u64,
    pub blake3: String,
    /// Ed25519 over `signed_message(version, size, blake3)`, hex.
    pub signature: String,
}

/// What signatures are made over.
pub fn signed_message(version: &str, size: u64, blake3: &str) -> Vec<u8> {
    format!("{CONTEXT}\n{version}\n{size}\n{}", blake3.to_ascii_lowercase()).into_bytes()
}

/// For the UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current: String,
    pub version: String,
    pub notes: String,
    pub published: String,
    pub size: u64,
    /// The release on GitHub (to download by hand when DropLaunch can't update itself).
    pub page: String,
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub fn releases_page() -> String {
    format!("https://github.com/{REPO}/releases/latest")
}

fn manifest_url() -> String {
    std::env::var(URL_ENV).unwrap_or_else(|_| format!("https://github.com/{REPO}/releases/latest/download/{MANIFEST_NAME}"))
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .user_agent(&format!("DropLaunch/{}", current_version()))
        .build()
}

/// Is `a` a newer version than `b` ("2.1.0" vs "2.0.3")?
pub fn is_newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> {
        v.trim().trim_start_matches(['v', 'V']).split(['.', '-', '+']).take(3).map(|p| p.parse().unwrap_or(0)).collect()
    };
    let (mut a, mut b) = (parts(a), parts(b));
    a.resize(3, 0);
    b.resize(3, 0);
    a > b
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn unhex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let text = text.trim();
    if text.len() != N * 2 {
        return None;
    }
    let mut out = [0u8; N];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(text.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

/// The signature is DropLaunch's release key's.
pub fn verify(manifest: &Manifest, public_key: &str) -> Result<(), String> {
    use ed25519_dalek::{Signature, VerifyingKey};
    let bad = || t!("update.error.signature");
    let key = unhex::<32>(public_key).and_then(|k| VerifyingKey::from_bytes(&k).ok()).ok_or_else(bad)?;
    let signature = unhex::<64>(&manifest.signature).map(|s| Signature::from_bytes(&s)).ok_or_else(bad)?;
    key.verify_strict(&signed_message(&manifest.version, manifest.size, &manifest.blake3), &signature).map_err(|_| bad())
}

/// A newer DropLaunch, if there is one. Ok(None) also when the newest release has no
/// latest.json (e.g. an older DropLaunch's release).
pub fn check() -> Result<Option<(Manifest, UpdateInfo)>, String> {
    let response = match agent().get(&manifest_url()).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(404, _)) => return Ok(None),
        Err(e) => return Err(t!("update.error.check", error = e)),
    };
    let text = response.into_string().map_err(|e| t!("update.error.check", error = e))?;
    let manifest: Manifest = serde_json::from_str(&text).map_err(|e| t!("update.error.manifest", error = e))?;
    if !is_newer(&manifest.version, current_version()) {
        return Ok(None);
    }
    verify(&manifest, PUBLIC_KEY)?;
    let info = UpdateInfo {
        current: current_version().to_string(),
        version: manifest.version.clone(),
        notes: manifest.notes.clone(),
        published: manifest.published.clone(),
        size: manifest.size,
        page: releases_page(),
    };
    Ok(Some((manifest, info)))
}

fn current_exe() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|e| t!("update.error.exe", error = e))
}

fn sibling(exe: &Path, suffix: &str) -> PathBuf {
    let mut name = exe.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    exe.with_file_name(name)
}

/// Download the new exe next to the running one and check it. Returns its path.
pub fn download(manifest: &Manifest, progress: &mut dyn FnMut(u64, u64)) -> Result<PathBuf, String> {
    let exe = current_exe()?;
    let part = sibling(&exe, ".download");
    let dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut out = std::fs::File::create(&part).map_err(|e| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            t!("update.error.readOnly", dir = dir.display())
        } else {
            t!("update.error.saveNext", error = e)
        }
    })?;
    let result = (|| {
        let response = agent().get(&manifest.url).call().map_err(|e| t!("update.error.download", error = e))?;
        let mut body = response.into_reader().take(manifest.size + 1);
        let mut hasher = blake3::Hasher::new();
        let mut buf = vec![0u8; 256 * 1024];
        let mut done = 0u64;
        loop {
            let n = body.read(&mut buf).map_err(|e| t!("update.error.broke", error = e))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            out.write_all(&buf[..n]).map_err(|e| t!("update.error.save", error = e))?;
            done += n as u64;
            progress(done, manifest.size);
        }
        out.flush().map_err(|e| t!("update.error.save", error = e))?;
        if done != manifest.size || !hasher.finalize().to_hex().as_str().eq_ignore_ascii_case(&manifest.blake3) {
            return Err(t!("update.error.mismatch"));
        }
        Ok(())
    })();
    drop(out);
    match result {
        Ok(()) => Ok(part),
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

/// Put the downloaded exe where the running one is (the running one becomes `<name>.old`).
pub fn install(new_exe: &Path) -> Result<PathBuf, String> {
    let exe = current_exe()?;
    let old = sibling(&exe, ".old");
    match std::fs::remove_file(&old) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(t!("update.error.removeOld", file = old.display(), error = e)),
    }
    std::fs::rename(&exe, &old).map_err(|e| t!("update.error.moveAside", error = e))?;
    if let Err(e) = std::fs::rename(new_exe, &exe) {
        let _ = std::fs::rename(&old, &exe);
        return Err(t!("update.error.place", error = e));
    }
    Ok(exe)
}

/// Start the new DropLaunch; it waits for this one to close (single instance).
pub fn launch(exe: &Path) -> Result<(), String> {
    std::process::Command::new(exe)
        .arg(AFTER_UPDATE_ARG)
        .arg(std::process::id().to_string())
        .spawn()
        .map(drop)
        .map_err(|e| t!("update.error.start", error = e))
}

/// Started by an update: wait (a while) for the old DropLaunch to close, and take the
/// `--after-update <pid>` arguments out. Leftovers of an earlier update are removed.
pub fn after_update(args: &mut Vec<String>) {
    if let Some(i) = args.iter().position(|a| a == AFTER_UPDATE_ARG) {
        let pid = args.get(i + 1).and_then(|p| p.parse::<u32>().ok());
        args.drain(i..(i + 2).min(args.len()));
        if let Some(process) = pid.and_then(crate::platform::ProcessHandle::open) {
            let deadline = std::time::Instant::now() + Duration::from_secs(20);
            while process.is_running() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        for leftover in [sibling(&exe, ".old"), sibling(&exe, ".download")] {
            let _ = std::fs::remove_file(leftover);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn versions() {
        assert!(is_newer("2.0.1", "2.0.0"));
        assert!(is_newer("2.1", "2.0.9"));
        assert!(is_newer("v3.0.0", "2.9.9"));
        assert!(!is_newer("2.0.0", "2.0.0"));
        assert!(!is_newer("1.9.9", "2.0.0"));
        assert!(!is_newer("2.0.0-beta", "2.0.0"));
    }

    #[test]
    fn signatures() {
        let key = SigningKey::from_bytes(&[7u8; 32]);
        let public = hex(key.verifying_key().as_bytes());
        let mut manifest = Manifest {
            version: "2.0.1".into(),
            notes: "Notes aren't signed".into(),
            published: String::new(),
            url: "https://example.com/DropLaunch.exe".into(),
            size: 123,
            blake3: "AB".repeat(32),
            signature: String::new(),
        };
        manifest.signature = hex(&key.sign(&signed_message("2.0.1", 123, &manifest.blake3)).to_bytes());
        assert!(verify(&manifest, &public).is_ok());
        // Another version, size or file: rejected.
        for change in [|m: &mut Manifest| m.version = "2.0.2".into(), |m: &mut Manifest| m.size = 124, |m: &mut Manifest| m.blake3 = "cd".repeat(32)] {
            let mut changed = manifest.clone();
            change(&mut changed);
            assert!(verify(&changed, &public).is_err());
        }
        assert!(verify(&manifest, &hex(SigningKey::from_bytes(&[8u8; 32]).verifying_key().as_bytes())).is_err());
        assert_eq!(unhex::<2>("0aFf"), Some([10, 255]));
        assert_eq!(unhex::<2>("0aF"), None);
    }

    #[test]
    fn after_update_arguments_are_taken_out() {
        let mut args = vec!["--after-update".to_string(), "4000000000".to_string(), r"C:\Maps\a.map".to_string()];
        after_update(&mut args);
        assert_eq!(args, vec![r"C:\Maps\a.map".to_string()]);
    }
}
