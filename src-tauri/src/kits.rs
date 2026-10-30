//! Kits: sets of items (what to wear, the belt, the inventory) given to players on the running
//! server over RCON. Each kit is a file of its own in DropLaunch's data folder (Kits/<name>.json,
//! marked `"droplaunchKit": 1`), so a kit can be shared: dropping one on DropLaunch adds it; the
//! built-in kits (Basic Build Kit) are added there once. With the DropLaunch plugin loaded each
//! item goes into its own slot as one stack (`droplaunch.givekit`); without it Rust's own
//! `inventory.giveto` hands them over and Rust picks where they go. The item list comes from the
//! server (the plugin's `droplaunch.items`, kept in items.json) or the one built into DropLaunch;
//! item pictures come from a public Rust wiki the first time they're shown and are kept in
//! ItemIcons/.
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Where the kits were kept before they were files of their own (moved over once).
pub const KITS_FILE: &str = "kits.json";
pub const KITS_DIR: &str = "Kits";
/// What makes a .json file a kit (and its format's version).
const KIT_MARK: u32 = 1;
/// Kits that come with DropLaunch: added to the Kits folder once (a deleted one stays gone).
const BUILT_IN: [&str; 1] = [include_str!("kits/basic-build-kit.json")];
/// The built-in kits already added (their ids, one per line).
pub const BUILT_IN_FILE: &str = "builtin-kits.txt";
/// The item list the server last told (newer than the built-in one after a Rust update).
pub const ITEMS_FILE: &str = "items.json";
pub const ICONS_DIR: &str = "ItemIcons";
/// Where pictures come from, in order (`{}`: the item's short name).
const ICON_SOURCES: [&str; 2] = ["https://wiki.rustclash.com/img/items180/{}.png", "https://cdn.rusthelp.com/images/public/{}.png"];
/// An item no source has a picture for isn't asked about again for this long.
const MISSING_RETRY: Duration = Duration::from_secs(7 * 24 * 3600);
const MAX_ICON: u64 = 2 * 1024 * 1024;

/// Rust's containers: what a player wears (7 slots), the belt (6) and the inventory (24).
pub const WEAR_SLOTS: u32 = 7;
pub const BELT_SLOTS: u32 = 6;
pub const MAIN_SLOTS: u32 = 24;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KitItem {
    /// Rust's short name (`rifle.ak`).
    pub item: String,
    pub amount: u32,
    pub slot: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Kit {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub wear: Vec<KitItem>,
    #[serde(default)]
    pub belt: Vec<KitItem>,
    #[serde(default)]
    pub main: Vec<KitItem>,
}

/// One item of Rust's list (short keys: the list is long).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CatalogItem {
    pub s: String,
    pub n: String,
    pub c: String,
    pub st: u32,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Player {
    pub id: String,
    pub name: String,
}

/// A kit as a file: the kit and the mark that says it's one.
#[derive(Serialize, Deserialize)]
struct KitFile {
    #[serde(rename = "droplaunchKit")]
    mark: u32,
    #[serde(flatten)]
    kit: Kit,
}

/// The kits, by name.
pub fn load(data: &Path) -> Vec<Kit> {
    load_files(data).into_iter().map(|(kit, _)| kit).collect()
}

/// The kits and their files (the old kits.json becomes files the first time).
pub fn load_files(data: &Path) -> Vec<(Kit, PathBuf)> {
    let dir = data.join(KITS_DIR);
    let old = data.join(KITS_FILE);
    if old.is_file() {
        if let Some(kits) = std::fs::read_to_string(&old).ok().and_then(|text| serde_json::from_str::<Vec<Kit>>(&text).ok()) {
            if save(data, &kits).is_ok() {
                let _ = std::fs::rename(&old, old.with_extension("json.old"));
            }
        }
    }
    add_built_in(data);
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut kits: Vec<(Kit, PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("json")))
        .filter_map(|p| read_kit(&p).ok().map(|kit| (kit, p)))
        .collect();
    kits.sort_by_key(|(k, _)| k.name.to_lowercase());
    kits
}

/// The ids of the kits that come with DropLaunch (they get a "Built in" tag).
pub fn built_in_ids() -> Vec<String> {
    BUILT_IN.iter().filter_map(|text| serde_json::from_str::<KitFile>(text).ok()).map(|f| f.kit.id).collect()
}

/// The built-in kits not added before go into the Kits folder (unless a kit with their id is
/// there already).
fn add_built_in(data: &Path) {
    let marker = data.join(BUILT_IN_FILE);
    let added = std::fs::read_to_string(&marker).unwrap_or_default();
    let fresh: Vec<Kit> = BUILT_IN
        .iter()
        .filter_map(|text| serde_json::from_str::<KitFile>(text).ok())
        .map(|file| clean(file.kit))
        .filter(|kit| !added.lines().any(|id| id == kit.id))
        .collect();
    if fresh.is_empty() {
        return;
    }
    let dir = data.join(KITS_DIR);
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let existing: Vec<(String, PathBuf)> = std::fs::read_dir(&dir)
        .map(|entries| entries.flatten().map(|e| e.path()).filter_map(|p| read_kit(&p).ok().map(|k| (k.id, p))).collect())
        .unwrap_or_default();
    let mut ids = added.clone();
    for kit in fresh {
        if !existing.iter().any(|(id, _)| *id == kit.id) {
            let stem = file_stem(&kit.name);
            let path = (1..)
                .map(|n| dir.join(if n == 1 { format!("{stem}.json") } else { format!("{stem}-{n}.json") }))
                .find(|p| !p.exists())
                .expect("a free name");
            let Ok(text) = serde_json::to_string_pretty(&KitFile { mark: KIT_MARK, kit: kit.clone() }) else { continue };
            if std::fs::write(&path, text).is_err() {
                continue;
            }
        }
        if !ids.is_empty() && !ids.ends_with('\n') {
            ids.push('\n');
        }
        ids.push_str(&kit.id);
        ids.push('\n');
    }
    let _ = std::fs::write(&marker, ids);
}

/// A kit file (dropped, or one of DropLaunch's own).
pub fn read_kit(path: &Path) -> Result<Kit> {
    let text = std::fs::read_to_string(path)?;
    let file: KitFile = serde_json::from_str(&text).with_context(|| format!("{}: not a kit", path.display()))?;
    if file.mark < 1 {
        bail!("{}: not a kit", path.display());
    }
    Ok(clean(file.kit))
}

/// A kit's file name: its name in plain letters (`Raid kit` -> `raid-kit.json`).
fn file_stem(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out: String = out.trim_end_matches('-').chars().take(40).collect();
    if out.is_empty() {
        "kit".into()
    } else {
        out
    }
}

/// A kit file dropped on DropLaunch: added (or, with the same id, put in place of that kit).
pub fn import(data: &Path, path: &Path) -> Result<Kit> {
    let mut kit = read_kit(path)?;
    if kit.id.trim().is_empty() {
        kit.id = crate::startfile::random_password(16);
    }
    if kit.name.is_empty() {
        kit.name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Kit".into());
    }
    let mut kits = load(data);
    kits.retain(|k| k.id != kit.id);
    kits.push(kit.clone());
    save(data, &kits)?;
    Ok(kit)
}

/// What the plugin prints when a player saves their inventory as a kit (/droplaunch kit).
const GAME_KIT_MARKER: &str = "DLKITSAVE1 ";

/// The JSON of a kit saved in the game, in a server log line (printed by the plugin itself: not
/// a chat message that types the marker).
pub fn game_kit_json(line: &str) -> Option<&str> {
    let at = line.find(GAME_KIT_MARKER)?;
    if !line[..at].trim_end().ends_with("[DropLaunch]") || line.contains("[CHAT]") {
        return None;
    }
    let json = line[at + GAME_KIT_MARKER.len()..].trim();
    json.starts_with('{').then_some(json)
}

/// A kit as the plugin prints it: the player's wear, belt and inventory.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GameKit {
    id: String,
    #[serde(default)]
    name: String,
    /// The player who saved it.
    #[serde(default)]
    by: String,
    #[serde(default)]
    wear: Vec<KitItem>,
    #[serde(default)]
    belt: Vec<KitItem>,
    #[serde(default)]
    main: Vec<KitItem>,
}

impl GameKit {
    /// The name it gets: the one typed, else "<player>'s inventory".
    fn kit(self) -> Kit {
        let name = if self.name.trim().is_empty() { crate::t!("kits.fromGame.name", name = self.by.trim()) } else { self.name };
        clean(Kit { id: format!("game-{}", self.id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(48).collect::<String>()), name, wear: self.wear, belt: self.belt, main: self.main })
    }
}

/// The console's line for a kit saved in the game.
pub fn describe_game_kit(json: &str) -> String {
    match serde_json::from_str::<GameKit>(json) {
        Ok(k) => {
            let by = k.by.clone();
            let kit = k.kit();
            crate::t!("kits.fromGame.line", name = kit.name, by, n = item_count(&kit))
        }
        Err(_) => crate::t!("kits.fromGame.lineRaw", json),
    }
}

/// A kit saved in the game goes into the Kits folder (once: the same save again changes nothing).
pub fn from_game(data: &Path, json: &str) -> Result<Kit> {
    let kit = serde_json::from_str::<GameKit>(json).context("not a kit")?.kit();
    if item_count(&kit) == 0 {
        bail!("an empty kit");
    }
    let mut kits = load(data);
    if let Some(same) = kits.iter().find(|k| k.id == kit.id) {
        return Ok(same.clone());
    }
    kits.push(kit.clone());
    save(data, &kits)?;
    Ok(kit)
}

/// Only what can be given: known-looking names, amounts above zero, slots that exist.
pub fn clean(mut kit: Kit) -> Kit {
    let keep = |items: &mut Vec<KitItem>, slots: u32| {
        items.retain(|i| valid_short_name(&i.item) && i.amount > 0 && i.slot < slots);
        items.sort_by_key(|i| i.slot);
        items.dedup_by_key(|i| i.slot);
    };
    keep(&mut kit.wear, WEAR_SLOTS);
    keep(&mut kit.belt, BELT_SLOTS);
    keep(&mut kit.main, MAIN_SLOTS);
    kit.name = kit.name.trim().chars().take(60).collect();
    kit
}

/// Every kit into its own file (named after the kit; a renamed kit's file follows), and the files
/// of kits that are gone removed.
pub fn save(data: &Path, kits: &[Kit]) -> Result<()> {
    let dir = data.join(KITS_DIR);
    std::fs::create_dir_all(&dir)?;
    let existing: Vec<(String, PathBuf)> = std::fs::read_dir(&dir)
        .map(|entries| entries.flatten().map(|e| e.path()).filter_map(|p| read_kit(&p).ok().map(|k| (k.id, p))).collect())
        .unwrap_or_default();
    let mut taken: Vec<PathBuf> = Vec::new();
    for kit in kits {
        let stem = file_stem(&kit.name);
        let mine = existing.iter().find(|(id, _)| *id == kit.id).map(|(_, p)| p.clone());
        // Keep its file while the name fits it; else the first free name.
        let fits = |p: &PathBuf| p.file_stem().and_then(|s| s.to_str()).is_some_and(|s| s == stem || s.strip_prefix(&format!("{stem}-")).is_some_and(|n| n.chars().all(|c| c.is_ascii_digit())));
        let path = match mine.as_ref().filter(|p| fits(p) && !taken.contains(p)) {
            Some(p) => p.clone(),
            None => (1..)
                .map(|n| dir.join(if n == 1 { format!("{stem}.json") } else { format!("{stem}-{n}.json") }))
                .find(|p| !taken.contains(p) && existing.iter().all(|(id, q)| q != p || *id == kit.id))
                .expect("a free name"),
        };
        let partial = path.with_extension("json.part");
        std::fs::write(&partial, serde_json::to_string_pretty(&KitFile { mark: KIT_MARK, kit: kit.clone() })?)?;
        std::fs::rename(&partial, &path)?;
        if let Some(old) = mine.filter(|old| *old != path) {
            let _ = std::fs::remove_file(old);
        }
        taken.push(path);
    }
    for (_, path) in existing {
        if !taken.contains(&path) {
            let _ = std::fs::remove_file(path);
        }
    }
    Ok(())
}

/// Rust's short names are lower case letters, digits, dots, dashes and underscores; nothing else
/// may reach a console command or a file name.
pub fn valid_short_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
}

/// The plugin's form: `w:0:hat.wolf:1;b:0:rifle.ak:1;m:3:ammo.rifle:128`.
pub fn spec(kit: &Kit) -> String {
    let mut parts = Vec::new();
    for (tag, items) in [("w", &kit.wear), ("b", &kit.belt), ("m", &kit.main)] {
        for i in items.iter().filter(|i| valid_short_name(&i.item) && i.amount > 0) {
            parts.push(format!("{tag}:{}:{}:{}", i.slot, i.item, i.amount));
        }
    }
    parts.join(";")
}

/// Without the plugin: Rust's own command, one per item (Rust picks the slots).
pub fn vanilla_commands(kit: &Kit, steam_id: &str) -> Vec<String> {
    kit.wear
        .iter()
        .chain(&kit.belt)
        .chain(&kit.main)
        .filter(|i| valid_short_name(&i.item) && i.amount > 0)
        .map(|i| format!("inventory.giveto {steam_id} {} {}", i.item, i.amount))
        .collect()
}

pub fn item_count(kit: &Kit) -> usize {
    kit.wear.len() + kit.belt.len() + kit.main.len()
}

/// Rust's `playerlist` (JSON with SteamID and DisplayName per player).
pub fn parse_playerlist(text: &str) -> Option<Vec<Player>> {
    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct Entry {
        #[serde(rename = "SteamID")]
        steam_id: String,
        display_name: String,
    }
    let entries: Vec<Entry> = serde_json::from_str(text.trim()).ok()?;
    Some(entries.into_iter().filter(|e| !e.steam_id.is_empty()).map(|e| Player { id: e.steam_id, name: e.display_name }).collect())
}

/// The plugin's `droplaunch.items` reply.
pub fn parse_items(text: &str) -> Option<Vec<CatalogItem>> {
    #[derive(Deserialize)]
    struct Reply {
        items: Vec<CatalogItem>,
    }
    let reply: Reply = serde_json::from_str(text.trim()).ok()?;
    let items: Vec<CatalogItem> = reply.items.into_iter().filter(|i| valid_short_name(&i.s)).collect();
    (!items.is_empty()).then_some(items)
}

pub fn load_items(data: &Path) -> Option<Vec<CatalogItem>> {
    serde_json::from_str(&std::fs::read_to_string(data.join(ITEMS_FILE)).ok()?).ok()
}

pub fn save_items(data: &Path, items: &[CatalogItem]) -> Result<()> {
    std::fs::create_dir_all(data)?;
    std::fs::write(data.join(ITEMS_FILE), serde_json::to_string(items)?)?;
    Ok(())
}

/// An item's picture (PNG): from the cache, else downloaded once and kept.
pub fn icon(data: &Path, item: &str) -> Result<Vec<u8>> {
    if !valid_short_name(item) {
        bail!("not an item: {item}");
    }
    let dir = data.join(ICONS_DIR);
    let file = dir.join(format!("{item}.png"));
    if let Ok(bytes) = std::fs::read(&file) {
        return Ok(bytes);
    }
    let missing = dir.join(format!("{item}.none"));
    let recent = |p: &PathBuf| p.metadata().and_then(|m| m.modified()).ok().and_then(|t| SystemTime::now().duration_since(t).ok()).is_some_and(|age| age < MISSING_RETRY);
    if recent(&missing) {
        bail!("no picture for {item}");
    }
    std::fs::create_dir_all(&dir)?;
    let agent = crate::http::agent(Duration::from_secs(8), Duration::from_secs(15));
    for source in ICON_SOURCES {
        let Ok(response) = agent.get(&source.replace("{}", item)).call() else { continue };
        if !response.content_type().starts_with("image/") {
            continue;
        }
        let mut bytes = Vec::new();
        response.into_reader().take(MAX_ICON).read_to_end(&mut bytes).context("download")?;
        // A PNG, not an error page.
        if bytes.starts_with(b"\x89PNG") {
            let partial = file.with_extension("part");
            std::fs::write(&partial, &bytes)?;
            std::fs::rename(&partial, &file)?;
            return Ok(bytes);
        }
    }
    let _ = std::fs::write(&missing, b"");
    bail!("no picture for {item}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kit() -> Kit {
        Kit {
            id: "k".into(),
            name: " Raid ".into(),
            wear: vec![KitItem { item: "metal.facemask".into(), amount: 1, slot: 0 }],
            belt: vec![KitItem { item: "rifle.ak".into(), amount: 1, slot: 0 }, KitItem { item: "syringe.medical".into(), amount: 2, slot: 2 }],
            main: vec![KitItem { item: "ammo.rifle".into(), amount: 128, slot: 0 }],
        }
    }

    #[test]
    fn spec_and_vanilla_commands() {
        assert_eq!(spec(&kit()), "w:0:metal.facemask:1;b:0:rifle.ak:1;b:2:syringe.medical:2;m:0:ammo.rifle:128");
        let cmds = vanilla_commands(&kit(), "76561198000000000");
        assert_eq!(cmds.len(), 4);
        assert_eq!(cmds[3], "inventory.giveto 76561198000000000 ammo.rifle 128");
    }

    #[test]
    fn nothing_odd_reaches_a_command() {
        let mut k = kit();
        k.main.push(KitItem { item: "ammo.rifle; quit".into(), amount: 1, slot: 5 });
        k.main.push(KitItem { item: "wood".into(), amount: 0, slot: 6 });
        k.belt.push(KitItem { item: "bandage".into(), amount: 1, slot: 9 });
        let k = clean(k);
        assert_eq!(k.name, "Raid");
        assert_eq!(k.main.len(), 1);
        assert_eq!(k.belt.len(), 2);
        assert!(!valid_short_name("../x") && !valid_short_name("Rifle") && valid_short_name("hat.wolf"));
    }

    #[test]
    fn kits_are_files_of_their_own() {
        let data = std::env::temp_dir().join(format!("droplaunch-kitfiles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        std::fs::create_dir_all(&data).unwrap();
        // The old single file moves over.
        std::fs::write(data.join(KITS_FILE), serde_json::to_string(&vec![kit()]).unwrap()).unwrap();
        let kits = load(&data);
        assert_eq!(kits.iter().map(|k| k.name.as_str()).collect::<Vec<_>>(), ["Basic Build Kit", "Raid"]);
        assert!(data.join(KITS_DIR).join("raid.json").is_file());
        assert!(data.join(KITS_DIR).join("basic-build-kit.json").is_file());
        assert!(!data.join(KITS_FILE).exists());
        let kits: Vec<Kit> = kits.into_iter().filter(|k| k.name == "Raid").collect();
        // Renamed: its file follows (and the built-in kit, deleted, doesn't come back); a second kit with the same name gets its own file.
        let mut renamed = kits[0].clone();
        renamed.name = "Raid kit".into();
        let mut other = kit();
        other.id = "k2".into();
        other.name = "Raid kit".into();
        save(&data, &[renamed, other]).unwrap();
        let mut names: Vec<String> = std::fs::read_dir(data.join(KITS_DIR)).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["raid-kit-2.json", "raid-kit.json"]);
        // Dropped: added; the same id replaces it; a language file isn't a kit.
        let shared = data.join("shared.json");
        let mut dropped = kit();
        dropped.id = "k2".into();
        dropped.name = "Shared".into();
        std::fs::write(&shared, serde_json::to_string(&KitFile { mark: 1, kit: dropped }).unwrap()).unwrap();
        import(&data, &shared).unwrap();
        let kits = load(&data);
        assert_eq!(kits.iter().map(|k| k.name.as_str()).collect::<Vec<_>>(), ["Raid kit", "Shared"]);
        std::fs::write(&shared, r#"{"language":{"code":"nl","name":"Nederlands"},"strings":{}}"#).unwrap();
        assert!(read_kit(&shared).is_err());
        save(&data, &[]).unwrap();
        assert_eq!(std::fs::read_dir(data.join(KITS_DIR)).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn kits_saved_in_the_game() {
        let line = r#"[DropLaunch] DLKITSAVE1 {"id":"6389-76561198000000001","name":"","by":"Ted","wear":[{"item":"hat.wolf","amount":1,"slot":0}],"belt":[{"item":"rifle.ak","amount":1,"slot":0}],"main":[{"item":"wood","amount":99999,"slot":3}]}"#;
        let json = game_kit_json(line).unwrap();
        assert_eq!(describe_game_kit(json), "Kit \"Ted's inventory\" saved in the game by Ted (3 items).");
        let data = std::env::temp_dir().join(format!("droplaunch-gamekit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        std::fs::create_dir_all(&data).unwrap();
        let kit = from_game(&data, json).unwrap();
        assert_eq!((kit.name.as_str(), kit.main[0].amount, kit.main[0].slot), ("Ted's inventory", 99999, 3));
        // The same line again (a log read twice) adds nothing.
        from_game(&data, json).unwrap();
        assert_eq!(load(&data).iter().filter(|k| k.id == kit.id).count(), 1);
        // Typed in the chat, it's not the plugin's.
        assert!(game_kit_json(r#"[CHAT] Ted[76561198000000001] : [DropLaunch] DLKITSAVE1 {"id":"x"}"#).is_none());
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn players_from_rust() {
        let text = r#"[{"SteamID":"76561198000000001","OwnerSteamID":"0","DisplayName":"Ted","Ping":3,"Address":"127.0.0.1:1","ConnectedSeconds":9,"VoiationLevel":0.0,"CurrentLevel":0.0,"UnspentXp":0.0,"Health":100.0}]"#;
        assert_eq!(parse_playerlist(text), Some(vec![Player { id: "76561198000000001".into(), name: "Ted".into() }]));
        assert_eq!(parse_playerlist("[]"), Some(vec![]));
        assert_eq!(parse_playerlist("Command not found"), None);
    }

    #[test]
    fn items_from_the_plugin() {
        let text = r#"{"items":[{"s":"rifle.ak","n":"Assault Rifle","c":"Weapon","st":1},{"s":"Bad Name","n":"x","c":"Misc","st":1}]}"#;
        let items = parse_items(text).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].n, "Assault Rifle");
    }
}
