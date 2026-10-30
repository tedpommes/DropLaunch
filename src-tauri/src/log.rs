//! DropLaunch's own log, `droplaunch.log` in the data folder: what went wrong in the background
//! (and panics), where a release build has no console to print it to. Kept small: at 1 MB it
//! becomes `droplaunch.old.log` and a new one starts.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::sync::Locked;

pub const FILE: &str = "droplaunch.log";
const OLD_FILE: &str = "droplaunch.old.log";
const MAX_SIZE: u64 = 1024 * 1024;

static PATH: OnceLock<PathBuf> = OnceLock::new();
/// One line at a time.
static WRITING: Mutex<()> = Mutex::new(());

/// Log into `data` from now on, panics too (which thread, where, and what it said).
pub fn init(data: &Path) {
    let _ = PATH.set(data.join(FILE));
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        write(&format!("panic in {}: {info}", thread.name().unwrap_or("a thread")));
        default(info);
    }));
}

/// One line, with the date and time (UTC).
pub fn write(text: &str) {
    #[cfg(debug_assertions)]
    eprintln!("{text}");
    let Some(path) = PATH.get() else { return };
    let _one = WRITING.locked();
    if std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_SIZE) {
        let _ = std::fs::rename(path, path.with_file_name(OLD_FILE));
    }
    let _ = path.parent().map(std::fs::create_dir_all);
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{} {text}", timestamp(crate::client::now_secs()));
    }
}

/// `2026-10-26 17:52:03`
fn timestamp(secs: i64) -> String {
    let time = secs.rem_euclid(86_400);
    format!("{} {:02}:{:02}:{:02}", crate::client::date(secs), time / 3600, time % 3600 / 60, time % 60)
}

#[cfg(test)]
mod tests {
    #[test]
    fn timestamps() {
        // 2026-10-01 00:00:00 UTC, plus a bit.
        assert_eq!(super::timestamp(1_790_812_800 + 3_723), "2026-10-01 01:02:03");
    }
}
