//! The SteamCMD command that downloads and updates the server files (Settings → Servers): the
//! arguments steamcmd.exe gets, with `%DIR%` (the server folder), `%BRANCH%` (its branch) and
//! `%VALIDATE%` (`validate` when every file is checked, else nothing) in them. DropLaunch's own
//! updates and the update .bat files (update_server.bat, update_rust_server.bat) both use it.
//!
//! Each argument is checked, so a command can't break the .bat files: letters, numbers and a few
//! signs, quotes around an argument with spaces, the three `%` words as arguments of their own.

use std::path::Path;

use super::RUST_SERVER_APP_ID;
use crate::t;

/// What DropLaunch runs when nothing else is set. Always names the branch: without `-beta`
/// SteamCMD keeps whatever beta was used last.
pub const DEFAULT: &str =
    "+@ShutdownOnFailedCommand 1 +@NoPromptForPassword 1 +force_install_dir %DIR% +login anonymous +app_update 258550 -beta %BRANCH% %VALIDATE% +quit";

/// The longest command (in characters).
pub const MAX_LEN: usize = 600;

/// Signs an argument can have besides ASCII letters and numbers (and spaces, in quotes). None of
/// them means anything to cmd.exe.
const SIGNS: &str = "+-_.@/\\:=,";

#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Text(String),
    Dir,
    Branch,
    Validate,
}

/// A checked command (see the module docs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCommand {
    parts: Vec<Part>,
}

impl Default for UpdateCommand {
    fn default() -> Self {
        Self::parse(DEFAULT).expect("the default SteamCMD command is valid")
    }
}

impl UpdateCommand {
    /// Checks a command as typed in Settings; the error says what to change.
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        if text.chars().count() > MAX_LEN {
            return Err(t!("settings.steamcmd.error.long", max = MAX_LEN));
        }
        if text.chars().any(|c| c.is_control() && !c.is_ascii_whitespace()) {
            return Err(t!("settings.steamcmd.error.chars"));
        }
        let mut parts = Vec::new();
        for (word, quoted) in split(text)? {
            let part = match word.to_ascii_uppercase().as_str() {
                "%DIR%" => Part::Dir,
                "%BRANCH%" => Part::Branch,
                "%VALIDATE%" => Part::Validate,
                _ if word.contains('%') => return Err(t!("settings.steamcmd.error.percent")),
                _ if !word.chars().all(|c| c.is_ascii_alphanumeric() || SIGNS.contains(c) || (quoted && c == ' ')) => {
                    return Err(t!("settings.steamcmd.error.chars"));
                }
                _ => Part::Text(word),
            };
            parts.push(part);
        }
        let text_is = |part: Option<&Part>, word: &str| matches!(part, Some(Part::Text(t)) if t.eq_ignore_ascii_case(word));
        let has = |wanted: &Part| parts.contains(wanted);
        // The server files go to the server folder, not into SteamCMD's.
        if !parts.iter().enumerate().any(|(i, p)| *p == Part::Dir && i > 0 && text_is(parts.get(i - 1), "+force_install_dir")) {
            return Err(t!("settings.steamcmd.error.dir"));
        }
        // Rust's server: DropLaunch reads its manifest (appmanifest_258550.acf) afterwards.
        if !parts.iter().enumerate().any(|(i, p)| text_is(Some(p), "+app_update") && text_is(parts.get(i + 1), RUST_SERVER_APP_ID)) {
            return Err(t!("settings.steamcmd.error.app", app = RUST_SERVER_APP_ID));
        }
        if !has(&Part::Branch) {
            return Err(t!("settings.steamcmd.error.branch"));
        }
        if !has(&Part::Validate) {
            return Err(t!("settings.steamcmd.error.validate"));
        }
        // Without it SteamCMD waits for more commands.
        if !text_is(parts.last(), "+quit") {
            return Err(t!("settings.steamcmd.error.quit"));
        }
        Ok(Self { parts })
    }

    /// The command stored in the settings: DropLaunch's own when none is (or the one stored can't
    /// be used, which the log says).
    pub fn from_setting(setting: Option<&str>) -> Self {
        let Some(text) = setting else { return Self::default() };
        Self::parse(text).unwrap_or_else(|e| {
            crate::log::write(&format!("The SteamCMD command in the settings isn't used ({e}): {text}"));
            Self::default()
        })
    }

    /// DropLaunch's own command.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The command as text, the way Settings shows it (quotes around arguments with spaces).
    pub fn text(&self) -> String {
        self.join(|part| match part {
            Part::Text(t) => quote_bat(t),
            Part::Dir => "%DIR%".into(),
            Part::Branch => "%BRANCH%".into(),
            Part::Validate => "%VALIDATE%".into(),
        })
    }

    /// The arguments for steamcmd.exe.
    pub fn args(&self, dir: &Path, branch: &str, validate: bool) -> Vec<String> {
        self.parts
            .iter()
            .filter_map(|part| match part {
                Part::Text(t) => Some(t.clone()),
                Part::Dir => Some(dir.to_string_lossy().into_owned()),
                Part::Branch => Some(branch.to_string()),
                Part::Validate => validate.then(|| "validate".to_string()),
            })
            .collect()
    }

    /// The arguments in a .bat, after `"%STEAMCMD%"`: its DIR and BRANCH variables, and
    /// `validate` (`%VALIDATE%`, or `validate` for the run that always checks every file).
    pub fn bat(&self, validate: &str) -> String {
        self.join(|part| match part {
            Part::Text(t) => quote_bat(t),
            Part::Dir => "\"%DIR%\\.\"".into(),
            Part::Branch => "%BRANCH%".into(),
            Part::Validate => validate.into(),
        })
    }

    /// The same for a bash script's `update <folder> <branch>` (`$VALIDATE` or `validate`).
    pub fn sh(&self, validate: &str) -> String {
        self.join(|part| match part {
            Part::Text(t) if t.chars().all(|c| c.is_ascii_alphanumeric() || "+-_.@/:=,".contains(c)) => t.clone(),
            Part::Text(t) => format!("'{t}'"),
            Part::Dir => "\"$1\"".into(),
            Part::Branch => "\"$2\"".into(),
            Part::Validate => validate.into(),
        })
    }

    fn join(&self, show: impl Fn(&Part) -> String) -> String {
        self.parts.iter().map(show).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ")
    }
}

/// Splits a command into arguments like a command line: spaces between them, quotes around one
/// with spaces in it. Each with whether it was quoted.
fn split(text: &str) -> Result<Vec<(String, bool)>, String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let (mut quoted, mut in_quotes, mut started) = (false, false, false);
    for c in text.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                quoted = true;
                started = true;
            }
            c if c.is_ascii_whitespace() && !in_quotes => {
                if started {
                    words.push((std::mem::take(&mut word), quoted));
                }
                (quoted, started) = (false, false);
            }
            // A quoted argument stays on one line.
            '\r' | '\n' => return Err(t!("settings.steamcmd.error.chars")),
            c if c.is_ascii_whitespace() => word.push(' '),
            c => {
                word.push(c);
                started = true;
            }
        }
    }
    if in_quotes {
        return Err(t!("settings.steamcmd.error.quote"));
    }
    if started {
        words.push((word, quoted));
    }
    if words.is_empty() {
        return Err(t!("settings.steamcmd.error.dir"));
    }
    Ok(words)
}

/// An argument for a command line: in quotes when it has a space or is empty (backslashes before
/// the closing quote doubled, as Windows programs read them).
fn quote_bat(word: &str) -> String {
    if !word.is_empty() && !word.contains(' ') {
        return word.to_string();
    }
    let trailing = word.len() - word.trim_end_matches('\\').len();
    format!("\"{word}{}\"", "\\".repeat(trailing))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn err(text: &str) -> String {
        UpdateCommand::parse(text).expect_err(text)
    }

    #[test]
    fn the_default_is_what_droplaunch_always_ran() {
        let command = UpdateCommand::default();
        assert!(command.is_default());
        assert!(DEFAULT.contains(&format!("+app_update {RUST_SERVER_APP_ID} ")));
        assert_eq!(command.text(), DEFAULT);
        let dir = PathBuf::from(r"C:\Servers\Rust Server");
        let args = |validate| command.args(&dir, "staging", validate).join("|");
        assert_eq!(
            args(false),
            r"+@ShutdownOnFailedCommand|1|+@NoPromptForPassword|1|+force_install_dir|C:\Servers\Rust Server|+login|anonymous|+app_update|258550|-beta|staging|+quit"
        );
        assert!(args(true).ends_with("|-beta|staging|validate|+quit"));
        assert_eq!(
            command.bat("%VALIDATE%"),
            r#"+@ShutdownOnFailedCommand 1 +@NoPromptForPassword 1 +force_install_dir "%DIR%\." +login anonymous +app_update 258550 -beta %BRANCH% %VALIDATE% +quit"#
        );
        assert!(command.sh("$VALIDATE").contains(r#"+force_install_dir "$1" +login anonymous +app_update 258550 -beta "$2" $VALIDATE +quit"#));
    }

    #[test]
    fn a_command_of_your_own() {
        let command = UpdateCommand::parse(
            r#"  +@sSteamCmdForcePlatformType windows +force_install_dir "%dir%" +login anonymous +app_update 258550 -beta %BRANCH% %VALIDATE% -language english +runscript "C:\My Scripts\after.txt" +quit "#,
        )
        .unwrap();
        assert!(!command.is_default());
        let args = command.args(Path::new("D:/rust"), "public", false);
        assert_eq!(args[0], "+@sSteamCmdForcePlatformType");
        assert_eq!(args[3], "D:/rust");
        assert!(args.contains(&r"C:\My Scripts\after.txt".to_string()));
        assert!(!args.contains(&"validate".to_string()));
        let bat = command.bat("validate");
        assert!(bat.contains(r#"+force_install_dir "%DIR%\." "#), "{bat}");
        assert!(bat.contains(r#"-beta %BRANCH% validate -language english +runscript "C:\My Scripts\after.txt" +quit"#), "{bat}");
        assert!(command.sh("validate").contains(r"+runscript 'C:\My Scripts\after.txt' +quit"));
        // Shown again the way it's used, and read back the same.
        assert_eq!(UpdateCommand::parse(&command.text()).unwrap(), command);
        assert!(UpdateCommand::parse(&DEFAULT.replace(' ', "   ")).unwrap().is_default());
    }

    #[test]
    fn quotes_keep_trailing_backslashes() {
        assert_eq!(quote_bat(r"C:\a b\"), r#""C:\a b\\""#);
        assert_eq!(quote_bat(""), r#""""#);
        assert_eq!(quote_bat("plain"), "plain");
    }

    #[test]
    fn what_a_command_needs() {
        let without = |part: &str| DEFAULT.replace(part, "");
        assert_eq!(err(""), t!("settings.steamcmd.error.dir"));
        assert_eq!(err(&without("+force_install_dir ")), t!("settings.steamcmd.error.dir"));
        assert_eq!(err(&without("%DIR%")), t!("settings.steamcmd.error.dir"));
        assert_eq!(err(&without("258550")), t!("settings.steamcmd.error.app", app = RUST_SERVER_APP_ID));
        assert_eq!(err(&DEFAULT.replace("258550", "740")), t!("settings.steamcmd.error.app", app = RUST_SERVER_APP_ID));
        assert_eq!(err(&without("%BRANCH%")), t!("settings.steamcmd.error.branch"));
        assert_eq!(err(&without("%VALIDATE%")), t!("settings.steamcmd.error.validate"));
        assert_eq!(err(&without("+quit")), t!("settings.steamcmd.error.quit"));
        assert_eq!(err(&format!("{DEFAULT} +app_update 258550")), t!("settings.steamcmd.error.quit"));
        assert_eq!(err(&format!("\"{DEFAULT}")), t!("settings.steamcmd.error.quote"));
        assert_eq!(err(&format!("{DEFAULT} {}", "x".repeat(MAX_LEN))), t!("settings.steamcmd.error.long", max = MAX_LEN));
    }

    #[test]
    fn nothing_that_means_something_to_cmd() {
        for bad in ["& del x", "| more", "> out.txt", "< in", "^", "(a)", "!x!", "'x'", "`x`", ";x", "*", "?", "x\u{7}"] {
            assert_eq!(err(&DEFAULT.replace("+quit", &format!("{bad} +quit"))), t!("settings.steamcmd.error.chars"), "{bad}");
        }
        for bad in ["%TEMP%", "%DIR%\\.", "x%", "-beta=%BRANCH%"] {
            assert_eq!(err(&DEFAULT.replace("+quit", &format!("{bad} +quit"))), t!("settings.steamcmd.error.percent"), "{bad}");
        }
        // Not even in quotes.
        assert_eq!(err(&DEFAULT.replace("+quit", "\"a & b\" +quit")), t!("settings.steamcmd.error.chars"));
        assert_eq!(err(&DEFAULT.replace("+quit", "\"a\nb\" +quit")), t!("settings.steamcmd.error.chars"));
        // Line breaks between arguments are only spaces: never a second line in the .bat.
        let command = UpdateCommand::parse(&DEFAULT.replace(" +quit", "\r\n+quit")).unwrap();
        assert!(command.is_default());
    }

    #[test]
    fn a_broken_setting_falls_back_to_the_default() {
        assert!(UpdateCommand::from_setting(None).is_default());
        assert!(UpdateCommand::from_setting(Some("+quit")).is_default());
        let own = DEFAULT.replace("+quit", "-language english +quit");
        assert_eq!(UpdateCommand::from_setting(Some(&own)).text(), own);
    }
}
