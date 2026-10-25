//! Release tool: makes DropLaunch's release key and signs releases for the self-updater.
//!
//!   cargo run --release --example release -- keygen <key file>
//!       Makes a new release key. Keep the file safe and out of the repo; put the printed
//!       public key in src/updater.rs (PUBLIC_KEY). Without the key, releases can't be signed and
//!       users have to download the next version by hand.
//!
//!   cargo run --release --example release -- sign <key file> <DropLaunch.exe> [notes file]
//!       Writes latest.json next to the exe, for the version in Cargo.toml. Upload both files to
//!       a GitHub release tagged v<version>. `--version <v>` and `--url <exe url>` sign for
//!       another version or download address (testing an update from a local web server).

use droplaunch_lib::updater::{self, Manifest};
use ed25519_dalek::{Signer, SigningKey};
use std::path::{Path, PathBuf};

/// `--version` / `--url` for `sign`.
#[derive(Default)]
struct Overrides {
    version: Option<String>,
    url: Option<String>,
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut overrides = Overrides::default();
    for (flag, slot) in [("--version", &mut overrides.version), ("--url", &mut overrides.url)] {
        if let Some(i) = args.iter().position(|a| a == flag) {
            if i + 1 < args.len() {
                *slot = Some(args.remove(i + 1));
            }
            args.remove(i);
        }
    }
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["keygen", key] => keygen(Path::new(key)),
        ["sign", key, exe] => sign(Path::new(key), Path::new(exe), None, &overrides),
        ["sign", key, exe, notes] => sign(Path::new(key), Path::new(exe), Some(Path::new(notes)), &overrides),
        _ => Err("usage: release keygen <key file> | release sign <key file> <DropLaunch.exe> [notes file]".into()),
    };
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

fn keygen(file: &Path) -> Result<(), String> {
    if file.exists() {
        return Err(format!("{} exists already; not replacing a release key.", file.display()));
    }
    let mut secret = [0u8; 32];
    getrandom::fill(&mut secret).map_err(|e| e.to_string())?;
    let key = SigningKey::from_bytes(&secret);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(file, format!("{}\n", updater::hex(&secret))).map_err(|e| e.to_string())?;
    println!("Release key saved to {}", file.display());
    println!("Public key (src/updater.rs, PUBLIC_KEY): {}", updater::hex(key.verifying_key().as_bytes()));
    Ok(())
}

fn sign(key_file: &Path, exe: &Path, notes: Option<&Path>, overrides: &Overrides) -> Result<(), String> {
    let text = std::fs::read_to_string(key_file).map_err(|e| format!("Could not read {}: {e}", key_file.display()))?;
    let secret = updater::unhex::<32>(&text).ok_or("The key file doesn't hold a release key.")?;
    let key = SigningKey::from_bytes(&secret);
    let bytes = std::fs::read(exe).map_err(|e| format!("Could not read {}: {e}", exe.display()))?;
    let version = overrides.version.as_deref().unwrap_or(updater::current_version());
    let blake3 = blake3::hash(&bytes).to_hex().to_string();
    let size = bytes.len() as u64;
    let notes = match notes {
        Some(path) => std::fs::read_to_string(path).map_err(|e| format!("Could not read {}: {e}", path.display()))?.trim().to_string(),
        None => String::new(),
    };
    let manifest = Manifest {
        version: version.to_string(),
        notes,
        published: today(),
        url: overrides.url.clone().unwrap_or_else(|| format!("https://github.com/{}/releases/download/v{version}/{}", updater::REPO, updater::EXE_NAME)),
        size,
        signature: updater::hex(&key.sign(&updater::signed_message(version, size, &blake3)).to_bytes()),
        blake3,
    };
    updater::verify(&manifest, &updater::hex(key.verifying_key().as_bytes()))?;
    let out: PathBuf = exe.with_file_name(updater::MANIFEST_NAME);
    std::fs::write(&out, serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    println!("Signed DropLaunch {version} ({size} bytes): {}", out.display());
    println!("Upload {} and {} to a GitHub release tagged v{version}.", updater::EXE_NAME, updater::MANIFEST_NAME);
    Ok(())
}

/// Today's date (UTC) as 2026-10-09.
fn today() -> String {
    let days = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() / 86_400) as i64;
    // Civil date from days since 1970 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}
