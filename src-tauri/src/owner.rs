//! Making the player the owner (admin) of DropLaunch's test servers.
//!
//! Who plays on this PC comes from Steam's own files, read-only, without the Steam API or the
//! network: the registry says where Steam is and which account is signed in right now, and
//! `config/loginusers.vdf` lists the SteamID64 and name of every account that signed in here.
//! Rust reads its owners from `server/<identity>/cfg/users.cfg` when it starts, so DropLaunch adds
//! a line there for each of its servers. The line stays, so a double-clicked start file has it too.

use crate::t;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::steamcmd::vdf;

/// SteamID64 of account id 0; account ids (the registry's ActiveUser) count up from here.
const STEAM_ID_BASE: u64 = 76561197960265728;
/// Marks the users.cfg lines DropLaunch wrote, so it can take them out again.
pub const NOTE: &str = "added by DropLaunch";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamUser {
    pub steam_id: String,
    pub name: String,
}

/// A Steam account that signed in on this PC.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalAccount {
    #[serde(flatten)]
    pub user: SteamUser,
    /// Signed in to Steam right now.
    pub active: bool,
}

/// Every Steam account that signed in on this PC, the signed-in (or most recent) one first.
pub fn local_accounts() -> Vec<LocalAccount> {
    let Some(dir) = steam_dir() else { return Vec::new() };
    let Ok(text) = std::fs::read_to_string(dir.join("config").join("loginusers.vdf")) else { return Vec::new() };
    accounts(&text, active_account_id())
}

fn accounts(text: &str, active: Option<u32>) -> Vec<LocalAccount> {
    let root = vdf::parse(text);
    let Some(users) = root.get("users") else { return Vec::new() };
    let mut found: Vec<((u8, i64), LocalAccount)> = users
        .entries()
        .iter()
        .filter_map(|(id, fields)| {
            let steam_id = parse_steam_id(id).ok()?;
            let field = |key| fields.get(key).and_then(vdf::Vdf::as_str).unwrap_or("").trim();
            let is_active = active.is_some_and(|a| a != 0 && steam_id.parse::<u64>().ok() == Some(STEAM_ID_BASE + u64::from(a)));
            let recent = field("MostRecent") == "1" || field("AutoLogin") == "1";
            let rank = if is_active { 0 } else if recent { 1 } else { 2 };
            let newest_first = -field("Timestamp").parse::<i64>().unwrap_or(0);
            // The display name; the login name stays private.
            let name = Some(field("PersonaName")).filter(|n| !n.is_empty()).map_or_else(|| t!("owner.defaultName"), String::from);
            Some(((rank, newest_first), LocalAccount { user: SteamUser { steam_id, name }, active: is_active }))
        })
        .collect();
    found.sort_by_key(|(order, _)| *order);
    found.into_iter().map(|(_, account)| account).collect()
}

/// The account's picture as Steam keeps it on this PC (config/avatarcache/<SteamID64>.png): read
/// from the disk, nothing is asked online.
pub fn avatar(steam_id: &str) -> Option<Vec<u8>> {
    let id = parse_steam_id(steam_id).ok()?;
    let bytes = std::fs::read(steam_dir()?.join("config").join("avatarcache").join(format!("{id}.png"))).ok()?;
    bytes.starts_with(b"\x89PNG").then_some(bytes)
}

/// A SteamID64, also out of a steamcommunity.com/profiles/... link.
pub fn parse_steam_id(input: &str) -> Result<String, String> {
    let input = input.trim().trim_end_matches('/');
    if input.contains("/id/") {
        return Err(t!("owner.error.custom"));
    }
    let id = input.rsplit('/').next().unwrap_or(input);
    if id.len() == 17 && id.starts_with("7656119") && id.bytes().all(|b| b.is_ascii_digit()) {
        Ok(id.to_string())
    } else {
        Err(t!("owner.error.digits"))
    }
}

#[cfg(windows)]
pub(crate) fn steam_dir() -> Option<PathBuf> {
    let from_registry = crate::platform::user_registry_string(r"Software\Valve\Steam", "SteamPath").map(PathBuf::from);
    from_registry.into_iter().chain([PathBuf::from(r"C:\Program Files (x86)\Steam")]).find(|dir| dir.is_dir())
}

#[cfg(not(windows))]
pub(crate) fn steam_dir() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    [home.join(".steam").join("steam"), home.join(".local").join("share").join("Steam")].into_iter().find(|dir| dir.is_dir())
}

/// The account signed in to Steam right now (0 or None when Steam isn't running).
#[cfg(windows)]
fn active_account_id() -> Option<u32> {
    crate::platform::user_registry_dword(r"Software\Valve\Steam\ActiveProcess", "ActiveUser")
}

#[cfg(not(windows))]
fn active_account_id() -> Option<u32> {
    None
}

// ---------------------------------------------------------------- users.cfg

/// Users.cfg names are quoted: keep them plain.
pub fn clean_name(name: &str) -> String {
    let name: String = name.chars().filter(|c| !c.is_control() && *c != '"' && *c != '\\').take(32).collect();
    if name.trim().is_empty() {
        "owner".into()
    } else {
        name.trim().to_string()
    }
}

/// `ownerid <steamid> "<name>" "<notes>"` -> (steamid, written by DropLaunch)
fn owner_line(line: &str) -> Option<(&str, bool)> {
    let mut parts = line.split_whitespace();
    if !parts.next()?.eq_ignore_ascii_case("ownerid") {
        return None;
    }
    Some((parts.next()?, line.contains(&format!("\"{NOTE}\""))))
}

/// Users.cfg with `owner` in it, and without owners DropLaunch added for someone else.
fn updated(existing: &str, owner: Option<&SteamUser>) -> Option<String> {
    let mut changed = false;
    let mut lines: Vec<String> = existing
        .lines()
        .filter(|line| match owner_line(line) {
            Some((id, true)) if owner.is_none_or(|o| o.steam_id != id) => {
                changed = true;
                false
            }
            _ => true,
        })
        .map(str::to_string)
        .collect();
    if let Some(o) = owner {
        if !lines.iter().any(|l| owner_line(l).is_some_and(|(id, _)| id == o.steam_id)) {
            lines.push(format!("ownerid {} \"{}\" \"{NOTE}\"", o.steam_id, clean_name(&o.name)));
            changed = true;
        }
    }
    changed.then(|| lines.iter().map(|l| format!("{l}\r\n")).collect())
}

/// Make `owner` the owner of the server with this identity (None: take out what DropLaunch
/// added). True when users.cfg changed.
pub fn apply(server_dir: &Path, identity: &str, owner: Option<&SteamUser>) -> std::io::Result<bool> {
    let path = server_dir.join("server").join(identity).join("cfg").join("users.cfg");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let Some(next) = updated(&existing, owner) else { return Ok(false) };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, next)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOGINUSERS: &str = r#""users"
{
	"76561198000000001"
	{
		"AccountName"		"first_login"
		"PersonaName"		"First"
		"AutoLogin"		"0"
		"Timestamp"		"1700000000"
	}
	"76561198000000002"
	{
		"AccountName"		"second_login"
		"PersonaName"		"Second"
		"AutoLogin"		"1"
		"Timestamp"		"1600000000"
	}
	"not-an-id"
	{
		"PersonaName"		"Broken"
	}
}"#;

    #[test]
    fn signed_in_account_comes_first_then_auto_login() {
        let none = accounts(LOGINUSERS, None);
        assert_eq!(none.iter().map(|a| a.user.name.as_str()).collect::<Vec<_>>(), vec!["Second", "First"]);
        assert!(none.iter().all(|a| !a.active));
        // ActiveUser is the account id: SteamID64 minus the base.
        let active = accounts(LOGINUSERS, Some((76561198000000001 - STEAM_ID_BASE) as u32));
        assert_eq!(active[0].user, SteamUser { steam_id: "76561198000000001".into(), name: "First".into() });
        assert!(active[0].active);
        assert!(!format!("{active:?}").contains("first_login"));
    }

    #[test]
    fn steam_ids_and_profile_links() {
        assert_eq!(parse_steam_id(" 76561198000000001 ").unwrap(), "76561198000000001");
        assert_eq!(parse_steam_id("https://steamcommunity.com/profiles/76561198000000001/").unwrap(), "76561198000000001");
        assert!(parse_steam_id("https://steamcommunity.com/id/someone").unwrap_err().contains("custom"));
        assert!(parse_steam_id("1234").is_err());
    }

    #[test]
    fn users_cfg_gets_the_owner_once_and_loses_old_ones() {
        let me = SteamUser { steam_id: "76561198000000001".into(), name: "Me \"quoted\"".into() };
        let other = SteamUser { steam_id: "76561198000000002".into(), name: "Other".into() };
        let manual = "moderatorid 76561198000000009 \"Friend\" \"no reason\"\r\n";

        let first = updated(manual, Some(&me)).unwrap();
        assert_eq!(first, format!("{manual}ownerid 76561198000000001 \"Me quoted\" \"{NOTE}\"\r\n"));
        assert_eq!(updated(&first, Some(&me)), None, "already there");
        // Rust rewrites the file in its own words: still recognised.
        let rewritten = format!("ownerid 76561198000000001 \"Me quoted\" \"{NOTE}\"\n");
        assert_eq!(updated(&rewritten, Some(&me)), None);

        let switched = updated(&first, Some(&other)).unwrap();
        assert!(switched.starts_with(manual) && switched.contains("76561198000000002") && !switched.contains("76561198000000001"));
        assert_eq!(updated(&switched, None).unwrap(), manual);
        // An owner the user added by hand is never touched.
        let by_hand = "ownerid 76561198000000001 \"Me\" \"my notes\"\r\n";
        assert_eq!(updated(by_hand, None), None);
        assert_eq!(updated(by_hand, Some(&me)), None);
    }
}
