//! The whitelist: only the server owner and the players added in Settings can join DropLaunch's
//! servers. Rust has no whitelist of its own, so the DropLaunch plugin checks every login against
//! `<carbon|oxide>/data/DropLaunch/whitelist.json`, which DropLaunch writes (the plugin reads it
//! at each login, so a change counts right away, also on a running server).
use std::path::{Path, PathBuf};

use serde::Serialize;

/// The plugin's data file, under the framework's data folder.
const DATA_DIR: &str = "DropLaunch";
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
        std::fs::write(&file, &text)?;
    }
    Ok(())
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
