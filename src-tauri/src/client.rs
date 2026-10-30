//! The Rust game on this PC and its map cache.
//!
//! Rust keeps every map it loaded from a server in `<Rust>\maps\`, as `<name>_<number>.map`
//! (procedural maps as `proceduralmap.<size>.<seed>.<protocol>_<hash>_<number>.map`), and never
//! removes any. A map saved again under the same name can then be loaded from an old copy, so
//! before a map starts DropLaunch removes Rust's copies of it (v1 tried with the exact file name,
//! which never matched). The whole cache can be cleared too: Rust downloads or makes a map again
//! when it needs it.
//!
//! Where Rust is comes from Steam's own files (`libraryfolders.vdf`, `appmanifest_<app>.acf`).
//!
//! There are two Rust games: Rust (252490) for public servers (and its `last-month` betas), and
//! "Rust - Staging Branch" (700580, free for Rust owners, its own folder `RustStaging`) for
//! staging and Facepunch's other test branches. A client only joins a server of its own build,
//! so Join picks the game by the server's branch, and says so when the other one is open.

use crate::t;
use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::platform;
use crate::startfile;
use crate::steamcmd::vdf;

/// Points DropLaunch at another Rust folder (for testing without touching the real cache).
const DIR_ENV: &str = "DROPLAUNCH_RUST_DIR";

/// One of the two Rust games on Steam.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Game {
    Rust,
    Staging,
}

/// An installed game: its folder and the Steam beta it's set to (None: its default).
#[derive(Debug, Clone)]
pub struct Install {
    pub dir: PathBuf,
    pub beta: Option<String>,
}

impl Game {
    pub fn app_id(self) -> u32 {
        match self {
            Game::Rust => 252490,
            Game::Staging => 700580,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Game::Rust => "Rust",
            Game::Staging => "Rust - Staging Branch",
        }
    }

    /// The game that joins a server of this branch, and the beta it needs (None: its default).
    pub fn for_branch(branch: &str) -> (Game, Option<&str>) {
        match branch {
            "" | "public" | "release" => (Game::Rust, None),
            "last-month" | "last-last-month" => (Game::Rust, Some(branch)),
            // staging, aux01..03, debug: Facepunch's test builds.
            _ => (Game::Staging, None),
        }
    }

    /// Where Steam installed it.
    pub fn install(self) -> Option<Install> {
        if self == Game::Rust {
            if let Some(dir) = std::env::var_os(DIR_ENV) {
                return Some(Install { dir: PathBuf::from(dir), beta: None });
            }
        }
        let app = self.app_id().to_string();
        let steam = crate::owner::steam_dir()?;
        let mut libraries = vec![steam.clone()];
        if let Ok(text) = std::fs::read_to_string(steam.join("steamapps").join("libraryfolders.vdf")) {
            let root = vdf::parse(&text);
            let folders = root.get("libraryfolders").map(vdf::Vdf::entries).unwrap_or_default();
            // The library that lists the game first, then every other one.
            let mut listed: Vec<(bool, PathBuf)> = folders
                .iter()
                .filter_map(|(_, f)| {
                    let path = PathBuf::from(f.get("path")?.as_str()?);
                    let has_it = f.get("apps").is_some_and(|apps| apps.get(&app).is_some());
                    Some((has_it, path))
                })
                .collect();
            listed.sort_by_key(|(has_it, _)| !has_it);
            libraries.splice(0..0, listed.into_iter().map(|(_, p)| p));
        }
        libraries.iter().find_map(|lib| {
            let apps = lib.join("steamapps");
            let manifest = std::fs::read_to_string(apps.join(format!("appmanifest_{app}.acf"))).ok()?;
            let root = vdf::parse(&manifest);
            let state = root.get("AppState");
            let default_dir = if self == Game::Rust { "Rust" } else { "RustStaging" };
            let install = state.and_then(|a| a.get("installdir")).and_then(vdf::Vdf::as_str).unwrap_or(default_dir);
            let beta = state
                .and_then(|a| ["UserConfig", "MountedConfig"].iter().find_map(|s| a.get(s)?.get("BetaKey")?.as_str()))
                .filter(|b| !b.is_empty() && *b != "public")
                .map(String::from);
            let dir = apps.join("common").join(install);
            dir.is_dir().then_some(Install { dir, beta })
        })
    }

    /// `steam://run` with the connect command (Rust joins once it has loaded).
    pub fn run_url(self, port: u16) -> String {
        format!("steam://run/{}//+connect%20127.0.0.1:{port}/", self.app_id())
    }
}

/// Rust's game folder (`...\steamapps\common\Rust`), when it's installed.
pub fn game_dir() -> Option<PathBuf> {
    Game::Rust.install().map(|i| i.dir)
}

/// The games that run now (both are RustClient.exe: the folder tells them apart).
pub fn running() -> Vec<(u32, Game)> {
    let lower = |p: &Path| p.to_string_lossy().to_lowercase();
    let staging_dir = Game::Staging.install().map(|i| lower(&i.dir));
    platform::find_processes(platform::RUST_CLIENT)
        .into_iter()
        .map(|p| {
            let path = lower(&p.path);
            let staging = staging_dir.as_ref().is_some_and(|d| path.starts_with(d.as_str())) || path.contains("\\ruststaging\\") || path.contains("/ruststaging/");
            (p.pid, if staging { Game::Staging } else { Game::Rust })
        })
        .collect()
}

/// What Join found (and did) for a server of some branch on this PC.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinPlan {
    /// started: the right game starts with the connect command; running: it's open already;
    /// wrong: the other game is open; missing: the right game isn't installed.
    pub state: &'static str,
    pub port: u16,
    pub connect: String,
    /// The server's branch.
    pub branch: String,
    /// The game it needs.
    pub game: &'static str,
    pub app_id: u32,
    /// The game that's open instead (state "wrong").
    pub running_game: Option<&'static str>,
    /// The game is set to another Steam beta than the server's.
    pub note: Option<String>,
}

/// Join a server of `branch` on this PC at `port`: start the right game when nothing runs (and
/// `launch`), otherwise say what's open.
pub fn plan_join(branch: &str, port: u16, launch: bool) -> Result<JoinPlan, String> {
    let (game, beta) = Game::for_branch(branch);
    let open = running();
    let install = game.install();
    let state = if open.iter().any(|(_, g)| *g == game) {
        "running"
    } else if !open.is_empty() {
        "wrong"
    } else if install.is_none() && crate::owner::steam_dir().is_some() {
        "missing"
    } else {
        if launch {
            platform::open_url(&game.run_url(port)).map_err(|e| t!("game.error.open", game = game.name(), error = e))?;
        }
        "started"
    };
    // Rust's betas: last-month servers need Rust set to last-month, public ones to none.
    let note = install.filter(|_| game == Game::Rust).and_then(|i| match (beta, i.beta.as_deref()) {
        (Some(need), have) if have != Some(need) => {
            Some(t!("game.beta.need", need, have = have.map_or_else(|| t!("game.beta.normal"), String::from)))
        }
        (None, Some(have)) => Some(t!("game.beta.extra", have)),
        _ => None,
    });
    Ok(JoinPlan {
        state,
        port,
        connect: format!("client.connect 127.0.0.1:{port}"),
        branch: branch.to_string(),
        game: game.name(),
        app_id: game.app_id(),
        running_game: (state == "wrong").then(|| open[0].1.name()),
        note,
    })
}

/// Close every Rust that's open (like its X button) and start the one for `branch` straight
/// into the server at `port`.
pub fn restart_into(branch: &str, port: u16) -> Result<(), String> {
    let (game, _) = Game::for_branch(branch);
    if !platform::close_rust(std::time::Duration::from_secs(45)) {
        return Err(t!("game.error.notClosed"));
    }
    // Steam takes a moment to see that it closed.
    std::thread::sleep(std::time::Duration::from_secs(3));
    platform::open_url(&game.run_url(port)).map_err(|e| t!("game.error.start", game = game.name(), error = e))
}

/// Bring the game for `branch` to the front (any Rust when that one isn't open).
pub fn show(branch: &str) -> bool {
    let (game, _) = Game::for_branch(branch);
    let open = running();
    let pids: Vec<u32> = open.iter().filter(|(_, g)| *g == game).chain(open.iter()).map(|(pid, _)| *pid).collect();
    pids.into_iter().any(platform::bring_to_front)
}

/// Steam's install window for the game `branch` needs.
pub fn install_game(branch: &str) -> Result<(), String> {
    let (game, _) = Game::for_branch(branch);
    platform::open_url(&format!("steam://install/{}", game.app_id())).map_err(|e| t!("game.error.steam", error = e))
}

/// Both games' map caches (the staging game keeps its own).
fn cache_dirs() -> Vec<PathBuf> {
    [Game::Rust, Game::Staging].into_iter().filter_map(|g| g.install()).map(|i| i.dir.join("maps")).filter(|d| d.is_dir()).collect()
}

/// Rust's map cache, for the UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheInfo {
    /// Rust's game folder (None: Rust isn't installed, or Steam's files don't say where).
    pub game_dir: Option<String>,
    pub cache_dir: Option<String>,
    pub maps: usize,
    pub bytes: u64,
}

pub fn cache_info() -> CacheInfo {
    let game = game_dir();
    let cache = game.as_ref().map(|g| g.join("maps"));
    let files: Vec<(PathBuf, u64)> = cache_dirs().iter().flat_map(|d| cached_maps(d)).collect();
    CacheInfo {
        game_dir: game.map(|d| d.display().to_string()),
        cache_dir: cache.map(|d| d.display().to_string()),
        maps: files.len(),
        bytes: files.iter().map(|(_, size)| size).sum(),
    }
}

/// What a cleanup removed.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cleaned {
    pub removed: usize,
    pub bytes: u64,
    /// Files Rust has open (it's on that map right now): left alone.
    pub in_use: usize,
}

impl Cleaned {
    /// "2 copies (37 MB)"
    pub fn describe(&self) -> String {
        let (n, size) = (self.removed, size_text(self.bytes));
        if n == 1 {
            t!("game.copies.one", size)
        } else {
            t!("game.copies.other", n, size)
        }
    }
}

fn size_text(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb < 1.0 {
        t!("game.underMb")
    } else if mb >= 1024.0 {
        format!("{:.1} GB", mb / 1024.0)
    } else {
        format!("{mb:.0} MB")
    }
}

fn cached_maps(dir: &Path) -> Vec<(PathBuf, u64)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let meta = e.metadata().ok()?;
            (meta.is_file() && path.extension().is_some_and(|x| x.eq_ignore_ascii_case("map"))).then(|| (path, meta.len()))
        })
        .collect()
}

/// Where removed maps go when they're kept (in DropLaunch's data folder, a folder per day).
pub const BACKUP_DIR: &str = "ClientMapBackups";

/// Backed-up maps are kept this many days.
pub const BACKUP_DAYS: i64 = 30;

/// Seconds since 1970, now.
pub fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Today's date as `2026-10-19` (UTC), for the backup folder's name.
pub fn today() -> String {
    date(now_secs())
}

/// The date (UTC) of a moment in seconds since 1970, as `2026-10-19`.
pub fn date(secs: i64) -> String {
    // Days since 1970 to a date (Howard Hinnant's civil_from_days).
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Moves a map out of the cache into `backups/<date>/` (copied first when it's on another drive).
fn back_up(path: &Path, backups: &Path) -> std::io::Result<()> {
    let day = backups.join(today());
    std::fs::create_dir_all(&day)?;
    let name = path.file_name().ok_or_else(|| std::io::Error::other("no file name"))?;
    let mut dest = day.join(name);
    let mut n = 2;
    while dest.exists() {
        dest = day.join(format!("{}_{n}.map", Path::new(name).file_stem().map(|s| s.to_string_lossy()).unwrap_or_default()));
        n += 1;
    }
    if std::fs::rename(path, &dest).is_ok() {
        return Ok(());
    }
    std::fs::copy(path, &dest)?;
    std::fs::remove_file(path).inspect_err(|_| {
        let _ = std::fs::remove_file(&dest);
    })
}

/// The days' folders in `backups` older than BACKUP_DAYS go (only folders named like a date:
/// anything else in there was put there by someone else).
fn prune_backups(backups: &Path) {
    let oldest = date(now_secs() - BACKUP_DAYS * 86_400);
    let Ok(entries) = std::fs::read_dir(backups) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let dated = name.len() == 10 && name.bytes().enumerate().all(|(i, b)| if i == 4 || i == 7 { b == b'-' } else { b.is_ascii_digit() });
        if dated && name < oldest && entry.path().is_dir() {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Deletes the maps, or moves them to `backups` when that's given.
fn remove(files: impl IntoIterator<Item = (PathBuf, u64)>, backups: Option<&Path>) -> Cleaned {
    if let Some(dir) = backups {
        prune_backups(dir);
    }
    let mut done = Cleaned::default();
    for (path, size) in files {
        let result = match backups {
            Some(dir) => back_up(&path, dir),
            None => std::fs::remove_file(&path),
        };
        match result {
            Ok(()) => {
                done.removed += 1;
                done.bytes += size;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => done.in_use += 1,
        }
    }
    done
}

/// Rust's copies of the map in `map`: `<name>_<number>.map`.
pub fn clean_copies(map: &Path, backups: Option<&Path>) -> Cleaned {
    let title = startfile::map_title(map);
    let files = cache_dirs().iter().flat_map(|d| cached_maps(d)).filter(|(p, _)| file_name(p).is_some_and(|n| is_copy_of(&n, &title))).collect::<Vec<_>>();
    remove(files, backups)
}

/// Every map in Rust's caches (both games').
pub fn clear_cache(backups: Option<&Path>) -> Cleaned {
    remove(cache_dirs().iter().flat_map(|d| cached_maps(d)).collect::<Vec<_>>(), backups)
}

fn file_name(path: &Path) -> Option<String> {
    path.file_name().map(|n| n.to_string_lossy().into_owned())
}

/// `file` is one of Rust's copies of the map called `title`.
fn is_copy_of(file: &str, title: &str) -> bool {
    let lower = file.to_ascii_lowercase();
    let Some(stem) = lower.strip_suffix(".map") else { return false };
    // "<name>_<number>": Rust adds the number.
    let Some((name, number)) = stem.rsplit_once('_') else { return false };
    !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()) && name == title.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_branch_has_its_game() {
        assert_eq!(Game::for_branch("public"), (Game::Rust, None));
        assert_eq!(Game::for_branch("release"), (Game::Rust, None));
        assert_eq!(Game::for_branch("last-month"), (Game::Rust, Some("last-month")));
        assert_eq!(Game::for_branch("staging"), (Game::Staging, None));
        assert_eq!(Game::for_branch("aux02"), (Game::Staging, None));
        assert_eq!(Game::Staging.run_url(28015), "steam://run/700580//+connect%20127.0.0.1:28015/");
    }

    /// What's installed and running on this PC (reads Steam's files and the process list only).
    #[test]
    #[ignore]
    fn games_on_this_pc() {
        for game in [Game::Rust, Game::Staging] {
            println!("{}: {:?}", game.name(), game.install());
        }
        println!("running: {:?}", running());
    }

    #[test]
    fn copies_of_a_map_by_name() {
        assert!(is_copy_of("6KRaceDownv25_1935835635.map", "6KRaceDownv25"));
        assert!(is_copy_of("6kracedownv25_1.MAP", "6KRaceDownv25"));
        // Another version, or another map with the same start.
        assert!(!is_copy_of("6KRaceDownv24_2402663657.map", "6KRaceDownv25"));
        assert!(!is_copy_of("6KRaceDownv25b_1935835635.map", "6KRaceDownv25"));
        // Not one of Rust's names.
        assert!(!is_copy_of("6KRaceDownv25.map", "6KRaceDownv25"));
        assert!(!is_copy_of("6KRaceDownv25_12a.map", "6KRaceDownv25"));
        // Names with underscores keep them: Rust adds one more.
        assert!(is_copy_of("My_Map_v2_3295202127.map", "My_Map_v2"));
        // Procedural maps never match a custom map's name.
        assert!(!is_copy_of("proceduralmap.3000.1791402295802.289_524ae1283b33491c705b04968ddc069e_2725664276.map", "proceduralmap"));
    }

    #[test]
    fn cleanups_remove_the_right_files() {
        let game = std::env::temp_dir().join(format!("droplaunch-client-{}", std::process::id()));
        let cache = game.join("maps");
        let _ = std::fs::remove_dir_all(&game);
        std::fs::create_dir_all(&cache).unwrap();
        let names = [
            "Island_111.map",
            "Island_222.map",
            "Island2_555.map",
            "proceduralmap.3000.1.289_abc_666.map",
            "notes.txt",
        ];
        for n in names {
            std::fs::write(cache.join(n), b"12345").unwrap();
        }
        // Only this test sets DIR_ENV.
        std::env::set_var(DIR_ENV, &game);

        let info = cache_info();
        assert_eq!((info.maps, info.bytes), (4, 20));

        let copies = clean_copies(Path::new(r"C:\Maps\Island.map"), None);
        assert_eq!((copies.removed, copies.bytes), (2, 10));
        assert!(cache.join("Island2_555.map").is_file());

        // Kept: moved to the backups (a day's folder) instead. Days' folders past BACKUP_DAYS go,
        // other folders stay.
        let backups = game.join("backups");
        for old in ["2000-01-01", "notes"] {
            std::fs::create_dir_all(backups.join(old)).unwrap();
            std::fs::write(backups.join(old).join("Old_1.map"), b"12345").unwrap();
        }
        let all = clear_cache(Some(&backups));
        assert_eq!(all.removed, 2);
        assert!(!backups.join("2000-01-01").exists());
        assert!(backups.join("notes").join("Old_1.map").is_file());
        let day = backups.join(today());
        assert!(day.join("Island2_555.map").is_file() && !cache.join("Island2_555.map").exists());
        assert!(cache.join("notes.txt").is_file(), "only maps");
        std::env::remove_var(DIR_ENV);
        let _ = std::fs::remove_dir_all(&game);
    }
}
