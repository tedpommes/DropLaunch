//! Your own scripts in a server folder, run by DropLaunch when they're there: `before_start.bat`
//! before a server starts (it isn't started when the script ends with an error), and
//! `after_update.bat` after the server files were updated (also by the update .bat files).
//! DropLaunch never makes them. They run hidden from the server folder, and what they print goes
//! into DropLaunch's console.
//!
//! A script runs only as you OK'd it: one that's new in the folder, or changed since, is asked
//! about first (the start of a server), and isn't run until then. Anything that can write to the
//! server folder (a plugin from a pack, a mod) could otherwise leave one there for DropLaunch to
//! run. What you OK'd, and how each script's last run went, is kept in the data folder
//! (`KNOWN_FILE`), never next to the script.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::sync::Locked;
use crate::t;

#[cfg(windows)]
pub const BEFORE_START: &str = "before_start.bat";
#[cfg(not(windows))]
pub const BEFORE_START: &str = "before_start.sh";
#[cfg(windows)]
pub const AFTER_UPDATE: &str = "after_update.bat";
#[cfg(not(windows))]
pub const AFTER_UPDATE: &str = "after_update.sh";

/// How long a script may take before it's ended.
pub const BEFORE_START_TIMEOUT: Duration = Duration::from_secs(120);
pub const AFTER_UPDATE_TIMEOUT: Duration = Duration::from_secs(600);
/// Lines of its output kept (the last ones).
const MAX_LINES: usize = 200;
/// ...and bytes, while it runs (a script that prints without end doesn't fill the memory).
const MAX_OUTPUT: usize = 256 * 1024;
/// How long the rest of its output is waited for once the script itself has ended. A program it
/// started and left running (`start "" tool.exe`) keeps the output open for as long as it runs:
/// the server start doesn't wait for that.
const OUTPUT_GRACE: Duration = Duration::from_secs(1);

/// How a script went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ran {
    /// What it printed (the last `MAX_LINES` lines).
    pub lines: Vec<String>,
    /// Its exit code (None: it was ended, or it couldn't be started).
    pub code: Option<i32>,
    pub timed_out: bool,
}

impl Ran {
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }

    /// Why it went wrong, for the console (None: it went fine).
    pub fn problem(&self, name: &str) -> Option<String> {
        if self.timed_out {
            Some(t!("hook.timedOut", name))
        } else {
            match self.code {
                Some(0) => None,
                Some(code) => Some(t!("hook.failed", name, code)),
                None => Some(t!("hook.ended", name)),
            }
        }
    }
}

/// The script is in the folder.
pub fn present(dir: &Path, name: &str) -> bool {
    dir.join(name).is_file()
}

// ---------------------------------------------------------------- what you OK'd
/// In the data folder: per server folder and script, what you OK'd and its last run.
const KNOWN_FILE: &str = "hook-scripts.json";
/// Server folders remembered at most (the oldest go).
const MAX_KNOWN: usize = 50;
/// The file is read and written one at a time.
static KNOWN_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Known {
    /// The script's fingerprint (blake3) when you OK'd it.
    ok: String,
    last: Option<LastRun>,
    /// When this was last changed (ms), to forget the oldest.
    at: i64,
}

/// How a script's last run went (Settings › Advanced shows it).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastRun {
    /// Milliseconds since 1970.
    pub at: i64,
    pub code: Option<i32>,
    pub timed_out: bool,
    /// Lines it printed.
    pub lines: usize,
}

/// Is the script there, and is it the one you OK'd?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Trust {
    /// No such script.
    None,
    Ok,
    /// Never OK'd (in this folder).
    New,
    /// Not the one you OK'd: it changed since.
    Changed,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

fn known_key(dir: &Path, name: &str) -> String {
    format!("{}|{name}", dir.display().to_string().to_lowercase().trim_end_matches(['\\', '/']))
}

fn load_known(data: &Path) -> BTreeMap<String, Known> {
    std::fs::read_to_string(data.join(KNOWN_FILE)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

fn save_known(data: &Path, mut all: BTreeMap<String, Known>) -> Result<(), String> {
    while all.len() > MAX_KNOWN * 2 {
        let Some(oldest) = all.iter().min_by_key(|(_, k)| k.at).map(|(key, _)| key.clone()) else { break };
        all.remove(&oldest);
    }
    let text = serde_json::to_string_pretty(&all).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(data).map_err(|e| e.to_string())?;
    let file = data.join(KNOWN_FILE);
    let partial = file.with_extension("json.part");
    std::fs::write(&partial, text).map_err(|e| e.to_string())?;
    std::fs::rename(&partial, &file).map_err(|e| e.to_string())
}

/// The script's fingerprint (None: it isn't there). A script is small; past 1 MB, the start and
/// the size.
fn fingerprint(dir: &Path, name: &str) -> Option<String> {
    let file = std::fs::File::open(dir.join(name)).ok()?;
    let size = file.metadata().ok()?.len();
    let mut bytes = Vec::new();
    file.take(1024 * 1024).read_to_end(&mut bytes).ok()?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(&bytes);
    hasher.update(&size.to_le_bytes());
    Some(hasher.finalize().to_hex().to_string())
}

/// Whether `name` in `dir` is there and is the one you OK'd.
pub fn trust(data: &Path, dir: &Path, name: &str) -> Trust {
    let Some(now) = fingerprint(dir, name) else { return Trust::None };
    let _one = KNOWN_LOCK.locked();
    match load_known(data).get(&known_key(dir, name)).map(|k| k.ok.clone()) {
        Some(ok) if ok == now => Trust::Ok,
        Some(ok) if !ok.is_empty() => Trust::Changed,
        _ => Trust::New,
    }
}

/// You OK'd the script as it is now: it runs from now on, until it changes.
pub fn approve(data: &Path, dir: &Path, name: &str) -> Result<(), String> {
    let now = fingerprint(dir, name).ok_or_else(|| t!("hook.gone", name))?;
    let _one = KNOWN_LOCK.locked();
    let mut all = load_known(data);
    let known = all.entry(known_key(dir, name)).or_default();
    known.ok = now;
    known.at = now_ms();
    save_known(data, all)
}

fn record(data: &Path, dir: &Path, name: &str, ran: &Ran) {
    let _one = KNOWN_LOCK.locked();
    let mut all = load_known(data);
    let known = all.entry(known_key(dir, name)).or_default();
    known.at = now_ms();
    known.last = Some(LastRun { at: known.at, code: ran.code, timed_out: ran.timed_out, lines: ran.lines.len() });
    if let Err(e) = save_known(data, all) {
        crate::log::write(&format!("hook scripts: {e}"));
    }
}

/// A script as Settings › Advanced shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub trust: Trust,
    pub last: Option<LastRun>,
}

pub fn status(data: &Path, dir: &Path, name: &str) -> Status {
    let trust = trust(data, dir, name);
    let last = {
        let _one = KNOWN_LOCK.locked();
        load_known(data).remove(&known_key(dir, name)).and_then(|k| k.last)
    };
    Status { trust, last }
}

/// What came of running a script only if you OK'd it.
#[derive(Debug)]
pub enum Checked {
    /// It isn't there.
    Missing,
    /// It's new or changed since you OK'd it: not run.
    NotOk(Trust),
    Ran(std::io::Result<Ran>),
}

/// Run `name` from `dir` (as `run`) only when it's the one you OK'd; its run is remembered.
pub fn run_checked(data: &Path, dir: &Path, name: &str, env: &[(&str, String)], timeout: Duration) -> Checked {
    match trust(data, dir, name) {
        Trust::None => Checked::Missing,
        Trust::Ok => {
            let ran = run_now(dir, name, env, timeout);
            if let Ok(ran) = &ran {
                record(data, dir, name, ran);
            }
            Checked::Ran(ran)
        }
        other => Checked::NotOk(other),
    }
}

/// Run `name` from `dir` if it's there (None: it isn't), hidden, with `env` set, ended after
/// `timeout`, whether you OK'd it or not (DropLaunch itself uses `run_checked`).
#[cfg(test)]
fn run(dir: &Path, name: &str, env: &[(&str, String)], timeout: Duration) -> Option<std::io::Result<Ran>> {
    if !present(dir, name) {
        return None;
    }
    Some(run_now(dir, name, env, timeout))
}

fn run_now(dir: &Path, name: &str, env: &[(&str, String)], timeout: Duration) -> std::io::Result<Ran> {
    #[cfg(windows)]
    let mut cmd = {
        let shell = std::env::var_os("ComSpec").unwrap_or_else(|| "cmd.exe".into());
        let mut cmd = Command::new(shell);
        // ".\": cmd won't run a file from the current folder by bare name when
        // NoDefaultCurrentDirectoryInExePath is set.
        cmd.args(["/D", "/C", &format!(".\\{name}")]);
        cmd
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut cmd = Command::new("bash");
        cmd.arg(format!("./{name}"));
        cmd
    };
    cmd.current_dir(dir).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.env("DROPLAUNCH_SERVER_DIR", dir);
    for (key, value) in env {
        cmd.env(key, value);
    }
    crate::platform::hide_console(&mut cmd);
    let mut child = cmd.spawn()?;
    // Both pipes are read on their own threads, so a script that prints a lot can't stall. What
    // they read is shared: it's taken when the script has ended, not when the pipes close.
    let read = |pipe: Option<Box<dyn Read + Send>>| {
        let output = Arc::new(Mutex::new(Vec::new()));
        let into = output.clone();
        let reader = std::thread::spawn(move || {
            let Some(mut pipe) = pipe else { return };
            let mut chunk = [0u8; 8192];
            loop {
                match pipe.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let mut all = into.locked();
                        all.extend_from_slice(&chunk[..n]);
                        if all.len() > MAX_OUTPUT {
                            let cut = all.len() - MAX_OUTPUT;
                            all.drain(..cut);
                        }
                    }
                }
            }
        });
        (output, reader)
    };
    let out = read(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let err = read(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let started = Instant::now();
    let (code, timed_out) = loop {
        if let Some(status) = child.try_wait()? {
            break (status.code(), false);
        }
        if started.elapsed() >= timeout {
            end_tree(&mut child);
            break (None, true);
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    // The script is done. Its last output is on its way; a program it left running holds the
    // pipes open, and isn't waited for.
    let done = Instant::now();
    while !(out.1.is_finished() && err.1.is_finished()) && done.elapsed() < OUTPUT_GRACE {
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut lines = text_lines(&out.0.locked());
    lines.extend(text_lines(&err.0.locked()));
    if lines.len() > MAX_LINES {
        lines.drain(..lines.len() - MAX_LINES);
    }
    Ok(Ran { lines, code, timed_out })
}

/// The script and what it started (a script's child holds the pipes open otherwise).
fn end_tree(child: &mut std::process::Child) {
    #[cfg(windows)]
    {
        let mut kill = Command::new("taskkill");
        kill.args(["/T", "/F", "/PID", &child.id().to_string()]).stdout(Stdio::null()).stderr(Stdio::null());
        crate::platform::hide_console(&mut kill);
        let _ = kill.status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn text_lines(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes).lines().map(|l| l.trim_end().to_string()).filter(|l| !l.is_empty()).collect()
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("droplaunch-hook-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_script_runs_only_when_its_there_and_its_output_comes_back() {
        let dir = temp_dir("run");
        assert!(run(&dir, BEFORE_START, &[], Duration::from_secs(10)).is_none());
        std::fs::write(dir.join(BEFORE_START), "@echo off\r\necho hello %DROPLAUNCH_MAP%\r\necho oops 1>&2\r\nexit /b 3\r\n").unwrap();
        let ran = run(&dir, BEFORE_START, &[("DROPLAUNCH_MAP", "Race".into())], Duration::from_secs(20)).unwrap().unwrap();
        assert_eq!(ran.lines, ["hello Race", "oops"]);
        assert_eq!(ran.code, Some(3));
        assert!(!ran.ok() && ran.problem(BEFORE_START).is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_script_that_takes_too_long_is_ended() {
        let dir = temp_dir("slow");
        std::fs::write(dir.join(AFTER_UPDATE), "@echo off\r\necho waiting\r\nping -n 30 127.0.0.1 >nul\r\n").unwrap();
        let started = Instant::now();
        let ran = run(&dir, AFTER_UPDATE, &[], Duration::from_secs(2)).unwrap().unwrap();
        assert!(ran.timed_out && started.elapsed() < Duration::from_secs(15));
        assert_eq!(ran.lines, ["waiting"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Third sweep, 5.6: a script runs only as you OK'd it. A new one, or one that changed since
    /// (something else wrote to the server folder), isn't run; its last run is remembered.
    #[test]
    fn a_script_runs_only_as_you_okd_it() {
        let dir = temp_dir("trust");
        let data = dir.join("data");
        let env: [(&str, String); 0] = [];
        assert_eq!(trust(&data, &dir, BEFORE_START), Trust::None);
        assert!(matches!(run_checked(&data, &dir, BEFORE_START, &env, Duration::from_secs(10)), Checked::Missing));
        std::fs::write(dir.join(BEFORE_START), "@echo off\r\necho one\r\n").unwrap();
        assert_eq!(trust(&data, &dir, BEFORE_START), Trust::New);
        assert!(matches!(run_checked(&data, &dir, BEFORE_START, &env, Duration::from_secs(10)), Checked::NotOk(Trust::New)));
        assert!(status(&data, &dir, BEFORE_START).last.is_none(), "it didn't run");
        approve(&data, &dir, BEFORE_START).unwrap();
        match run_checked(&data, &dir, BEFORE_START, &env, Duration::from_secs(20)) {
            Checked::Ran(Ok(ran)) => assert_eq!(ran.lines, ["one"]),
            other => panic!("{other:?}"),
        }
        let status_now = status(&data, &dir, BEFORE_START);
        assert_eq!(status_now.trust, Trust::Ok);
        assert_eq!(status_now.last.as_ref().map(|l| (l.code, l.lines)), Some((Some(0), 1)));
        // Changed by something else: not run until it's OK'd again.
        std::fs::write(dir.join(BEFORE_START), "@echo off\r\necho two\r\n").unwrap();
        assert_eq!(trust(&data, &dir, BEFORE_START), Trust::Changed);
        assert!(matches!(run_checked(&data, &dir, BEFORE_START, &env, Duration::from_secs(10)), Checked::NotOk(Trust::Changed)));
        // The same script in another server folder is OK'd on its own.
        let other = dir.join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join(BEFORE_START), "@echo off\r\necho one\r\n").unwrap();
        assert_eq!(trust(&data, &other, BEFORE_START), Trust::New);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A script that starts a program and ends (a bot, a sync tool): the server start goes on,
    /// it doesn't wait until that program is closed.
    #[test]
    fn a_program_the_script_leaves_running_isnt_waited_for() {
        let dir = temp_dir("helper");
        std::fs::write(dir.join(BEFORE_START), "@echo off\r\nstart \"\" /b ping -n 8 127.0.0.1 >nul\r\necho started my tool\r\n").unwrap();
        let started = Instant::now();
        let ran = run(&dir, BEFORE_START, &[], Duration::from_secs(60)).unwrap().unwrap();
        assert!(started.elapsed() < Duration::from_secs(4), "came back after {:?}", started.elapsed());
        assert!(ran.ok() && !ran.timed_out);
        assert_eq!(ran.lines, ["started my tool"]);
        // (The folder goes once the ping has ended: it's its working folder.)
        std::thread::sleep(Duration::from_secs(8));
        let _ = std::fs::remove_dir_all(dir);
    }
}
