//! The whitelist: only the server owner and the players added in Settings can join DropLaunch's
//! servers. Rust has no whitelist of its own, so the DropLaunch plugin checks every login against
//! `<carbon|oxide>/data/DropLaunch/whitelist.json`, which DropLaunch writes (the plugin reads it
//! at each login, so a change counts right away, also on a running server).
use std::path::{Path, PathBuf};

use serde::Serialize;

/// The plugin's data folder, under the framework's data folder (its key goes there too).
pub const DATA_DIR: &str = "DropLaunch";
const DATA_FILE: &str = "whitelist.json";

#[derive(Serialize)]
struct Data<'a> {
    on: bool,
    ids: &'a [String],
}

/// Where the file goes in a server folder: each framework folder that's there.
fn files(server_dir: &Path) -> Vec<PathBuf> {
    ["carbon", "oxide"]
        .iter()
        .map(|f| server_dir.join(f))
        .filter(|dir| dir.is_dir())
        .map(|dir| dir.join("data").join(DATA_DIR).join(DATA_FILE))
        .collect()
}

/// Writes who may join (`None`: everyone, the whitelist is off).
pub fn write(server_dir: &Path, ids: Option<&[String]>) -> std::io::Result<()> {
    let data = Data { on: ids.is_some(), ids: ids.unwrap_or(&[]) };
    let text = serde_json::to_string_pretty(&data).map_err(std::io::Error::other)?;
    for file in files(server_dir) {
        if std::fs::read_to_string(&file).is_ok_and(|old| old == text) {
            continue;
        }
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        write_whole(&file, &text)?;
    }
    Ok(())
}

/// Writes a file the plugin reads while the server runs (the whitelist, the key): whole or not at
/// all, through a file next to it. Written in place, it would be empty for a moment, and a login
/// checked in that moment would read a broken whitelist.
pub fn write_whole(file: &Path, text: &str) -> std::io::Result<()> {
    let mut partial = file.as_os_str().to_owned();
    partial.push(".part");
    let partial = PathBuf::from(partial);
    std::fs::write(&partial, text)?;
    // The plugin may be reading the old one right now, which holds it for a moment.
    let mut tries = 0;
    loop {
        match std::fs::rename(&partial, file) {
            Ok(()) => return Ok(()),
            Err(_) if tries < 5 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(40));
            }
            Err(e) => {
                let _ = std::fs::remove_file(&partial);
                return Err(e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn written_for_the_framework_that_is_there() {
        let dir = std::env::temp_dir().join(format!("droplaunch-whitelist-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("carbon")).unwrap();
        let ids = vec!["76561198000000001".to_string()];
        write(&dir, Some(&ids)).unwrap();
        let text = std::fs::read_to_string(dir.join("carbon/data/DropLaunch/whitelist.json")).unwrap();
        assert!(text.contains("\"on\": true") && text.contains("76561198000000001"));
        assert!(!dir.join("oxide").exists());
        write(&dir, None).unwrap();
        let text = std::fs::read_to_string(dir.join("carbon/data/DropLaunch/whitelist.json")).unwrap();
        assert!(text.contains("\"on\": false"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
