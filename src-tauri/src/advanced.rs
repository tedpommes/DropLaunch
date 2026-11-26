//! Settings › Advanced: what keeps DropLaunch working when something it relies on changes and
//! there's no new DropLaunch for it (yet). Other download links for SteamCMD, Carbon, Oxide and
//! the RustEdit extension; the server log lines DropLaunch waits for; start arguments for every
//! server; one more folder watched for map saves. Kept in the settings (`Config::advanced`) and
//! copied here whenever they change, so what needs them (downloads, the update .bat files, the
//! boot tracker) doesn't have to be handed the settings.

use serde::{Deserialize, Serialize};
use std::sync::RwLock;

use crate::t;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Advanced {
    /// Download links used instead of DropLaunch's own (None: DropLaunch's own).
    pub steamcmd_url: Option<String>,
    pub carbon_url: Option<String>,
    pub oxide_url: Option<String>,
    pub rustedit_url: Option<String>,
    /// A log line saying the server is ready, besides Rust's own "Server startup complete".
    pub ready_text: Option<String>,
    /// A log line saying RCON is up, besides Rust's own "WebSocket RCON Started".
    pub rcon_text: Option<String>,
    /// `+name value` start arguments for every server (like `+rustedit.key "..."` for encrypted
    /// RustEdit maps): DropLaunch's start files take them.
    pub start_args: Option<String>,
    /// One more folder watched for map saves, stored like the maps folder (RustEdit 2 scenes
    /// kept elsewhere export their .map next to the scene).
    pub watch_dir: Option<String>,
}

/// What DropLaunch downloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Link {
    Steamcmd,
    Carbon,
    Oxide,
    Rustedit,
}

impl Link {
    pub const ALL: [Link; 4] = [Link::Steamcmd, Link::Carbon, Link::Oxide, Link::Rustedit];

    /// DropLaunch's own link.
    pub fn default_url(self) -> &'static str {
        match self {
            Link::Steamcmd => crate::steamcmd::install::DOWNLOAD_URL,
            Link::Carbon => crate::mods::CARBON_URL,
            Link::Oxide => crate::mods::OXIDE_URL,
            Link::Rustedit => crate::mods::RUSTEDIT_URL,
        }
    }

    fn custom(self, a: &Advanced) -> Option<&String> {
        match self {
            Link::Steamcmd => a.steamcmd_url.as_ref(),
            Link::Carbon => a.carbon_url.as_ref(),
            Link::Oxide => a.oxide_url.as_ref(),
            Link::Rustedit => a.rustedit_url.as_ref(),
        }
    }

    pub fn slot(self, a: &mut Advanced) -> &mut Option<String> {
        match self {
            Link::Steamcmd => &mut a.steamcmd_url,
            Link::Carbon => &mut a.carbon_url,
            Link::Oxide => &mut a.oxide_url,
            Link::Rustedit => &mut a.rustedit_url,
        }
    }

    /// The link downloads use: yours when one is set.
    pub fn url(self) -> String {
        read(|a| self.custom(a).cloned()).unwrap_or_else(|| self.default_url().to_string())
    }

    /// What the download is saved as: the link's own file name when it says what it is (an
    /// archive is unpacked by its name), else DropLaunch's own link's (a link like
    /// `.../download?id=3` holds what the default one did).
    pub fn file_name(self) -> String {
        file_name_of(&self.url(), self.default_url())
    }
}

fn file_name_of(url: &str, default: &str) -> String {
    let last = |u: &str| {
        let path = u.split(['?', '#']).next().unwrap_or_default();
        path.rsplit('/').next().unwrap_or_default().to_string()
    };
    let own = last(url);
    let lower = own.to_ascii_lowercase();
    let wanted = last(default).to_ascii_lowercase();
    let same_kind = [".zip", ".tar.gz", ".tgz", ".dll"].iter().any(|ext| lower.ends_with(ext) && wanted.ends_with(ext));
    let safe = !own.is_empty() && own.len() <= 100 && own.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    if same_kind && safe {
        own
    } else {
        last(default)
    }
}

static CURRENT: RwLock<Option<Advanced>> = RwLock::new(None);

fn read<R>(f: impl FnOnce(&Advanced) -> R) -> R {
    let guard = CURRENT.read().unwrap_or_else(|e| e.into_inner());
    match guard.as_ref() {
        Some(a) => f(a),
        None => f(&Advanced::default()),
    }
}

/// The settings changed (or were loaded): what's used from now on.
pub fn set(advanced: &Advanced) {
    *CURRENT.write().unwrap_or_else(|e| e.into_inner()) = Some(advanced.clone());
}

/// The start arguments for every server (empty: none).
pub fn start_args() -> String {
    read(|a| a.start_args.clone()).unwrap_or_default()
}

/// The server's log says it's ready: your line (DropLaunch also knows Rust's own).
pub fn is_ready_line(line: &str) -> bool {
    read(|a| a.ready_text.as_deref().is_some_and(|text| line.contains(text)))
}

/// The server's log says RCON is up: your line (DropLaunch also knows Rust's own).
pub fn is_rcon_line(line: &str) -> bool {
    read(|a| a.rcon_text.as_deref().is_some_and(|text| line.contains(text)))
}

/// A hash that stays the same in every version of DropLaunch (FNV-1a), for marking what
/// DropLaunch wrote.
pub fn hash(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// A download link: https, one line, nothing the update .bat files (or the Linux script, where `$`
/// and `(` could run something) can't hold. Empty: DropLaunch's own.
pub fn check_url(text: &str) -> Result<Option<String>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let rest = text.get(..8).filter(|s| s.eq_ignore_ascii_case("https://")).map(|_| &text[8..]);
    let Some(rest) = rest else { return Err(t!("advanced.error.https")) };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let bad = |c: char| c.is_whitespace() || c.is_control() || matches!(c, '"' | '<' | '>' | '|' | '^' | '`' | '\\' | '$' | '(' | ')' | '\'' | ';');
    if host.is_empty() || !host.contains('.') || text.len() > 500 || text.chars().any(bad) {
        return Err(t!("advanced.error.url"));
    }
    Ok(Some(text.to_string()))
}

/// A log text to wait for: one line of at least four characters. Empty: Rust's own only.
pub fn check_text(text: &str) -> Result<Option<String>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    if text.chars().count() < 4 || text.len() > 120 || text.chars().any(char::is_control) {
        return Err(t!("advanced.error.text"));
    }
    Ok(Some(text.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_are_https_and_safe_for_the_update_files() {
        assert_eq!(check_url("  "), Ok(None));
        assert_eq!(check_url(" https://example.com/Carbon.zip ").unwrap().as_deref(), Some("https://example.com/Carbon.zip"));
        assert!(check_url("HTTPS://example.com/a%20b.zip?x=1&y=2").is_ok());
        // Third sweep, D5: nothing a shell reads as code either ($(...), quotes, ;).
        for bad in ["http://example.com/a.zip", "https://", "https://localhost", "https://a.com/a b.zip", "https://a.com/\"x", "https://a.com/x|y", "ftp://a.com/x", "https://a.com/$(id).zip", "https://a.com/x';id;'", "https://a.com/x;y"] {
            assert!(check_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_download_keeps_a_name_it_can_be_unpacked_by() {
        let carbon = "https://github.com/x/Carbon.Windows.Release.zip";
        assert_eq!(file_name_of("https://mirror.example/files/Carbon.Other.zip?raw=1", carbon), "Carbon.Other.zip");
        // No name, another kind of file, or an odd one: the default's.
        assert_eq!(file_name_of("https://mirror.example/download?id=3", carbon), "Carbon.Windows.Release.zip");
        assert_eq!(file_name_of("https://mirror.example/a.tar.gz", carbon), "Carbon.Windows.Release.zip");
        assert_eq!(file_name_of("https://mirror.example/..%2f.zip", carbon), "Carbon.Windows.Release.zip");
    }

    #[test]
    fn log_texts_are_one_short_line() {
        assert_eq!(check_text(""), Ok(None));
        assert_eq!(check_text(" Server is ready ").unwrap().as_deref(), Some("Server is ready"));
        assert!(check_text("abc").is_err());
        assert!(check_text("two\nlines").is_err());
        assert!(check_text(&"x".repeat(121)).is_err());
    }
}
