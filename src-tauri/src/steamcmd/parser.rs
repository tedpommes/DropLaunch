//! Recognises the SteamCMD lines DropLaunch acts on. Patterns come from real captures
//! (see tests/fixtures) - SteamCMD has no machine-readable output mode.

use crate::t;
use regex::Regex;
use std::sync::LazyLock;

#[derive(Debug, Clone, PartialEq)]
pub enum Signal {
    /// Bootstrapper phase: `[  0%] Checking for available updates...`, `[----] Verifying installation...`
    /// `bytes` is filled from `Downloading update (7,484 of 43,472 KB)`.
    SelfUpdate { text: String, percent: Option<f64>, bytes: Option<(u64, u64)> },
    SteamApi,
    Connecting,
    Connected,
    /// Signed in, waiting for Steam's answers: `Waiting for client config...`, `Waiting for user
    /// info...` (each can take a minute when Steam is slow).
    Waiting,
    LoggedIn,
    LoginFailed(String),
    /// ` Update state (0x61) downloading, progress: 3.40 (187504970 / 5520792242)`
    Progress { code: String, label: String, percent: f64, done: u64, total: u64 },
    Success { up_to_date: bool },
    Error(String),
}

static SELF_PERCENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\[\s*(\d{1,3})%\]\s*(.*)$").unwrap());
static SELF_DASH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\[----\]\s*(.*)$").unwrap());
static SELF_BYTES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\(([\d,]+) of ([\d,]+) KB\)").unwrap());
static LOGIN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:Connecting anonymously|Logging in user '[^']*') to Steam Public\.\.\.\s*(.*)$").unwrap()
});
static PROGRESS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"Update state \((0x[0-9a-fA-F]+)\)\s+([^,]+),\s*progress:\s*([0-9.]+)\s*\((\d+)\s*/\s*(\d+)\)").unwrap()
});
static SUCCESS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^Success!\s+App\s+'(\d+)'\s+(fully installed|already up to date)").unwrap());
static ERROR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:ERROR!|Error!)\s*(.+)$").unwrap());

pub fn parse(line: &str) -> Option<Signal> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    if let Some(c) = PROGRESS.captures(line) {
        let code = c[1].to_ascii_lowercase();
        if code == "0x0" {
            return None; // idle tick printed right before "Success!"
        }
        return Some(Signal::Progress {
            code,
            label: c[2].trim().to_string(),
            percent: c[3].parse().unwrap_or(0.0),
            done: c[4].parse().unwrap_or(0),
            total: c[5].parse().unwrap_or(0),
        });
    }
    let bootstrap = SELF_PERCENT
        .captures(line)
        .map(|c| (c[2].trim().to_string(), c[1].parse::<f64>().ok()))
        .or_else(|| SELF_DASH.captures(line).map(|c| (c[1].trim().to_string(), None)));
    if let Some((text, mut percent)) = bootstrap {
        // SteamCMD's own percentage lags behind; the KB counters are exact.
        let bytes = SELF_BYTES.captures(&text).and_then(|c| {
            let kb = |s: &str| s.replace(',', "").parse::<u64>().ok().map(|v| v * 1024);
            Some((kb(&c[1])?, kb(&c[2])?))
        });
        if let Some((done, total)) = bytes.filter(|&(_, total)| total > 0) {
            percent = Some(done as f64 * 100.0 / total as f64);
        }
        return Some(Signal::SelfUpdate { text, percent, bytes });
    }
    if line.starts_with("Loading Steam API") && line.ends_with("OK") {
        return Some(Signal::SteamApi);
    }
    if let Some(c) = LOGIN.captures(line) {
        let tail = c[1].trim();
        return Some(if tail.is_empty() {
            Signal::Connecting
        } else if tail.starts_with("OK") {
            Signal::Connected
        } else {
            Signal::LoginFailed(tail.trim_start_matches("FAILED").trim().trim_matches(['(', ')']).to_string())
        });
    }
    if line.starts_with("Waiting for user info") && line.ends_with("OK") {
        return Some(Signal::LoggedIn);
    }
    if line.starts_with("Waiting for client config") || line.starts_with("Waiting for user info") {
        return Some(Signal::Waiting);
    }
    if let Some(c) = SUCCESS.captures(line) {
        return Some(Signal::Success { up_to_date: &c[2] == "already up to date" });
    }
    if let Some(c) = ERROR.captures(line) {
        return Some(Signal::Error(c[1].trim().to_string()));
    }
    None
}

/// DropLaunch's own message when SteamCMD went quiet (worth another try, like a network hiccup).
pub const STOPPED_RESPONDING: &str = "SteamCMD stopped responding";

/// Human wording for the `Update state` labels SteamCMD prints.
pub fn friendly_label(label: &str) -> String {
    match label {
        "downloading" => t!("steamcmd.state.downloading"),
        "verifying install" => t!("steamcmd.state.verifyingInstall"),
        "verifying update" => t!("steamcmd.state.verifyingUpdate"),
        "staging" => t!("steamcmd.state.staging"),
        "preallocating" => t!("steamcmd.state.preallocating"),
        "reconfiguring" => t!("steamcmd.state.reconfiguring"),
        "committing" => t!("steamcmd.state.committing"),
        other => {
            let mut chars = other.chars();
            chars.next().map(|f| f.to_uppercase().chain(chars).collect()).unwrap_or_default()
        }
    }
}

/// Failures that usually go away when SteamCMD simply runs again.
pub fn is_transient(message: &str) -> bool {
    const HINTS: [&str; 10] = [
        "0x402",
        "0x602",
        "0x6 ",
        "Timed out",
        "Timeout",
        "No Connection",
        "Rate Limit",
        "Service Unavailable",
        "stopped responding",
        // Follows `Info request for AppId 7 returned error Timeout` when Steam answers slowly.
        "Failed to load platform info",
    ];
    HINTS.iter().any(|h| message.contains(h))
}

/// The update stopped because files it had to patch aren't there: the app's state has Steam's
/// "Files Missing" flag (0x20, as in `state is 0x426`). Only a validate gets past that: it
/// downloads them again. Carbon deletes three of Rust's own DLLs at every server start, so after a
/// Carbon server ran, a plain update of the next Rust build always ends like this.
pub fn files_missing(message: &str) -> bool {
    app_state(message).is_some_and(|state| state & 0x20 != 0) || message.contains("Missing game files")
}

/// `App '258550' state is 0x426 after update job.` -> 0x426
fn app_state(message: &str) -> Option<u32> {
    let hex: String = message.split("state is 0x").nth(1)?.chars().take_while(char::is_ascii_hexdigit).collect();
    u32::from_str_radix(&hex, 16).ok()
}

/// Explain the common SteamCMD errors in plain words (raw message kept for searching).
pub fn describe_error(message: &str) -> String {
    let shown = if message == STOPPED_RESPONDING { t!("steamcmd.error.stoppedResponding") } else { message.to_string() };
    let hint = if message.contains("0x202") {
        Some(t!("steamcmd.error.space"))
    } else if message.contains("Disk write failure") {
        Some(t!("steamcmd.error.write"))
    } else if files_missing(message) {
        Some(t!("steamcmd.error.missing"))
    } else if is_transient(message) {
        Some(t!("steamcmd.error.network"))
    } else {
        None
    };
    match hint {
        Some(hint) => t!("steamcmd.error.withHint", hint, message = shown),
        None => shown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_lines() {
        let p = parse(" Update state (0x61) downloading, progress: 3.40 (187504970 / 5520792242)");
        assert_eq!(
            p,
            Some(Signal::Progress {
                code: "0x61".into(),
                label: "downloading".into(),
                percent: 3.40,
                done: 187504970,
                total: 5520792242
            })
        );
        assert_eq!(parse(" Update state (0x0) unknown, progress: 0.00 (0 / 0)"), None);
    }

    #[test]
    fn login_states() {
        assert_eq!(parse("Connecting anonymously to Steam Public..."), Some(Signal::Connecting));
        assert_eq!(parse("Connecting anonymously to Steam Public...OK"), Some(Signal::Connected));
        assert_eq!(
            parse("Connecting anonymously to Steam Public...FAILED (No Connection)"),
            Some(Signal::LoginFailed("No Connection".into()))
        );
        assert_eq!(parse("Waiting for user info...OK"), Some(Signal::LoggedIn));
        assert_eq!(parse("Waiting for client config..."), Some(Signal::Waiting));
        assert_eq!(parse("Waiting for client config...OK"), Some(Signal::Waiting));
        assert_eq!(parse("Waiting for user info..."), Some(Signal::Waiting));
        // A slow Steam (seen 2026-10-26): the run quits after these, and another one usually works.
        assert_eq!(
            parse("ERROR! Info request for AppId 7 returned error Timeout."),
            Some(Signal::Error("Info request for AppId 7 returned error Timeout.".into()))
        );
        assert!(is_transient("Failed to load platform info"));
    }

    #[test]
    fn bootstrap_lines() {
        assert_eq!(
            parse("[  0%] Checking for available updates..."),
            Some(Signal::SelfUpdate { text: "Checking for available updates...".into(), percent: Some(0.0), bytes: None })
        );
        assert_eq!(
            parse("[----] Verifying installation..."),
            Some(Signal::SelfUpdate { text: "Verifying installation...".into(), percent: None, bytes: None })
        );
        let Some(Signal::SelfUpdate { percent, bytes, .. }) = parse("[  0%] Downloading update (7,484 of 43,472 KB)...") else {
            panic!("not a self-update line")
        };
        assert_eq!(bytes, Some((7_484 * 1024, 43_472 * 1024)));
        assert!((percent.unwrap() - 17.2).abs() < 0.1);
    }

    #[test]
    fn results() {
        assert_eq!(parse("Success! App '258550' fully installed."), Some(Signal::Success { up_to_date: false }));
        assert_eq!(parse("Success! App '258550' already up to date."), Some(Signal::Success { up_to_date: true }));
        let err = parse("Error! App '258550' state is 0x602 after update job.");
        assert_eq!(err, Some(Signal::Error("App '258550' state is 0x602 after update job.".into())));
        assert!(is_transient("App '258550' state is 0x602 after update job."));
        assert!(!is_transient("App '258550' state is 0x202 after update job."));
        assert!(describe_error("App '258550' state is 0x202 after update job.").starts_with("Not enough free disk space"));
        // Update Required, Fully Installed, Files Missing, Update Started (Carbon removed files).
        assert!(files_missing("App '258550' state is 0x426 after update job."));
        assert!(describe_error("App '258550' state is 0x426 after update job.").starts_with("Some of the server's files are missing"));
        for other in ["0x602", "0x402", "0x202", "0x6"] {
            assert!(!files_missing(&format!("App '258550' state is {other} after update job.")), "{other}");
        }
    }

    #[test]
    fn labels() {
        assert_eq!(friendly_label("verifying install"), "Verifying files");
        assert_eq!(friendly_label("something new"), "Something new");
    }
}
