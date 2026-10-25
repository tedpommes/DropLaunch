//! Turns the Rust server's log into boot stages and a rough progress figure, what it's busy with
//! while it builds the world (to spot a step that hangs), trouble prefab test mode avoids, and
//! where each line comes from (the console's colours).
//! Based on real logs (procedural map first boot: ~110 s; custom maps: "Loading custom map ...").

use crate::t;
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Stage {
    Starting,
    GameFiles,
    Mods,
    Map,
    Entities,
    Finishing,
    Ready,
}

/// What the server is busy with while it builds the world. Rust prints a step only once it's
/// done, so this is the step after the last one it printed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Work {
    /// Reading the custom map file ("Downloading World").
    ReadingMap,
    /// Generating the procedural map, one step after another.
    Generating,
    SpawningWorld,
    TerrainMesh,
    ProcessingWorld,
    /// The ocean patrol paths boats and the cargo ship follow. Some maps hang here for many
    /// minutes; prefab test mode skips it.
    OceanPaths,
}

impl Work {
    pub fn label(self) -> String {
        match self {
            Work::ReadingMap => t!("boot.work.readingMap"),
            Work::Generating => t!("boot.work.generating"),
            Work::SpawningWorld => t!("boot.work.spawningWorld"),
            Work::TerrainMesh => t!("boot.work.terrainMesh"),
            Work::ProcessingWorld => t!("boot.work.processingWorld"),
            Work::OceanPaths => t!("boot.work.oceanPaths"),
        }
    }

    /// What comes after a finished world step.
    fn after(step: &str, now: Option<Work>) -> Option<Work> {
        match step {
            "Downloading World" => Some(Work::SpawningWorld),
            "Spawning World" => Some(Work::TerrainMesh),
            "Terrain Mesh" => Some(Work::ProcessingWorld),
            "Processing World" => Some(Work::OceanPaths),
            "Ocean Patrol Paths" => None,
            // The procedural map's own steps (or a step this DropLaunch doesn't know).
            _ => now.or(Some(Work::Generating)),
        }
    }
}

/// Trouble in the log that prefab test mode is made to avoid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Issue {
    /// Errors from the cargo ship (it needs ocean paths an unfinished map often can't give it).
    CargoShip,
    /// Errors from server events (patrol helicopter, airdrops, ...).
    Events,
    /// A player was caught inside the terrain (antihack terrain protection).
    Terrain,
}

fn issue(line: &str) -> Option<Issue> {
    let lower = line.to_ascii_lowercase();
    let failing = lower.starts_with("at ") || lower.contains("exception") || lower.contains("error") || lower.contains("fail");
    if lower.contains("terrain") && ["violation", "antihack", "inside terrain", "insideterrain"].iter().any(|w| lower.contains(w)) {
        Some(Issue::Terrain)
    } else if failing && (lower.contains("cargoship") || lower.contains("cargo ship")) {
        Some(Issue::CargoShip)
    } else if failing && (lower.contains("triggeredevent") || lower.contains("eventschedule")) {
        Some(Issue::Events)
    } else {
        None
    }
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tracker {
    pub stage: Stage,
    pub detail: String,
    pub percent: f32,
    /// Carbon or Oxide, when the log shows one loading.
    pub framework: Option<&'static str>,
    /// The server opened WebRCON (DropLaunch's console connects right after).
    pub rcon_started: bool,
    /// What it's busy with while it builds the world (None before and after that).
    pub work: Option<Work>,
    /// Since when (ms since 1970).
    pub work_since: i64,
    /// Trouble seen in the log that prefab test mode avoids.
    pub issues: Vec<Issue>,
    /// The server announced a restart ("Restarting in 5 seconds", e.g. after a plugin update).
    #[serde(skip)]
    pub restart_announced: bool,
    #[serde(skip)]
    world_steps: u32,
    #[serde(skip)]
    custom_map: bool,
    /// Past the ocean paths, the last world step.
    #[serde(skip)]
    world_done: bool,
    /// Plugins that loaded, to tell their output ("[Name] ...") from the server's.
    #[serde(skip)]
    plugins: Vec<String>,
}

impl Default for Tracker {
    fn default() -> Self {
        Self {
            stage: Stage::Starting,
            detail: String::new(),
            percent: 2.0,
            framework: None,
            rcon_started: false,
            work: None,
            work_since: 0,
            issues: Vec::new(),
            restart_announced: false,
            world_steps: 0,
            custom_map: false,
            world_done: false,
            plugins: Vec::new(),
        }
    }
}

impl Tracker {
    pub fn ready(&self) -> bool {
        self.stage == Stage::Ready
    }

    /// Feed one log line; true when anything visible changed.
    pub fn feed(&mut self, line: &str) -> bool {
        let line = line.trim();
        if line.starts_with("Restarting in ") && (line.ends_with("seconds") || line.ends_with("second")) {
            self.restart_announced = true;
        }
        let mut changed = false;
        if let Some(found) = issue(line) {
            if !self.issues.contains(&found) {
                self.issues.push(found);
                changed = true;
            }
        }
        if let Some(name) = plugin_name(line) {
            if self.plugins.len() < 500 && !self.plugins.iter().any(|p| p == name) {
                self.plugins.push(name.to_string());
            }
        }
        if line.is_empty() || self.ready() {
            return changed;
        }
        let before = (self.stage, self.detail.clone(), self.percent as i32, self.work);

        if line.starts_with("Server startup complete") {
            self.set(Stage::Ready, "", 100.0);
        } else if line.starts_with("[Bootstrap] Opening Bundles") || line.starts_with("Loading shared/") {
            self.set(Stage::GameFiles, &t!("boot.detail.bundles"), 6.0);
        } else if line.starts_with("[Bootstrap] Loading Menu Prefabs") {
            self.set(Stage::GameFiles, &t!("boot.detail.prefabs"), 9.0);
        } else if line.starts_with("[Bootstrap] Loading Game Manifest") {
            self.set(Stage::GameFiles, &t!("boot.detail.manifest"), 12.0);
        } else if line.starts_with("[Bootstrap] Bootstrap Config") {
            self.set(Stage::GameFiles, &t!("boot.detail.config"), 14.0);
        } else if line.starts_with("[Bootstrap] Loading Items") || line.starts_with("Loading Scene") {
            self.set(Stage::GameFiles, &t!("boot.detail.items"), 16.0);
        } else if line.starts_with("Carbon ") && line.contains(" on ") {
            self.framework = Some("Carbon");
            self.set(Stage::Mods, &t!("boot.detail.framework", name = "Carbon"), 18.0);
        } else if line.starts_with("Loading Oxide Core") || line.starts_with("Loaded extension Rust") {
            self.framework = Some("Oxide");
            self.set(Stage::Mods, &t!("boot.detail.framework", name = "Oxide"), 18.0);
        } else if let Some(name) = plugin_name(line) {
            self.set(Stage::Mods, &t!("boot.detail.plugins", name), 20.0);
        } else if let Some((done, total)) = asset_warmup(line) {
            let pct = 20.0 + 8.0 * done as f32 / total.max(1) as f32;
            self.set(self.stage.max(Stage::GameFiles), &t!("boot.detail.assets", done, total), pct);
        } else if line.starts_with("WebSocket RCON Started") || line.starts_with("RCon Started") {
            self.rcon_started = true;
        } else if line.starts_with("Generating procedural map") {
            self.set(Stage::Map, &t!("boot.detail.generating"), 30.0);
            self.start_work(Some(Work::Generating));
        } else if line.starts_with("Loading custom map") {
            self.custom_map = true;
            self.set(Stage::Map, &t!("boot.detail.customMap"), 30.0);
            self.start_work(Some(Work::ReadingMap));
        } else if line.starts_with("[RustEdit] - No ocean patrol path data") {
            // The map brings no ocean path of its own, so Rust generates one now.
            if self.stage <= Stage::Map && !self.world_done && self.work != Some(Work::OceanPaths) {
                self.start_work(Some(Work::OceanPaths));
                self.detail = t!("boot.detail.working", step = Work::OceanPaths.label());
            }
        } else if let Some(step) = world_step(line) {
            // Rust prints a step with its time once it's finished.
            if self.stage <= Stage::Map && step != "asset_warmup" {
                self.world_steps += 1;
                let per_step = if self.custom_map { 6.0 } else { 1.4 };
                let pct = (30.0 + self.world_steps as f32 * per_step).min(70.0);
                let next = if self.world_done { None } else { Work::after(step, self.work) };
                self.world_done |= step == "Ocean Patrol Paths";
                self.start_work(next);
                let detail = match next {
                    Some(work) if work != Work::Generating => t!("boot.detail.working", step = work.label()),
                    _ => t!("boot.detail.stepDone", step),
                };
                self.set(Stage::Map, &detail, pct);
            }
        } else if line.starts_with("Spawning ") && line.contains(" entities") {
            self.set(Stage::Entities, &trim_dots(line), 74.0);
        } else if line.starts_with("Loading save") || line.starts_with("Couldn't load server/") {
            self.set(Stage::Entities, &t!("boot.detail.save"), 72.0);
        } else if line.starts_with("Postprocessing ") {
            self.set(Stage::Entities, &trim_dots(line), 80.0);
        } else if line.starts_with("[RustNav]") && self.stage >= Stage::Entities {
            self.set(Stage::Finishing, &t!("boot.detail.navmesh"), 86.0);
        } else if line.starts_with("Initializing ") && line.contains(" entity links") {
            self.set(Stage::Finishing, &t!("boot.detail.links"), 88.0);
        } else if line.starts_with("Preparing Occlusion Grid") {
            self.set(Stage::Finishing, &t!("boot.detail.occlusion"), 91.0);
        } else if line.starts_with("SteamServer Initialized") {
            self.set(Stage::Finishing, &t!("boot.detail.steam"), 96.0);
        }

        changed || before != (self.stage, self.detail.clone(), self.percent as i32, self.work)
    }

    /// Stages only move forward; progress never goes back.
    fn set(&mut self, stage: Stage, detail: &str, percent: f32) {
        if stage < self.stage {
            return;
        }
        self.stage = stage;
        self.detail = detail.to_string();
        self.percent = self.percent.max(percent);
        if stage > Stage::Map {
            self.work = None;
        }
    }

    /// The next world step started (the clock restarts even for the same kind of work).
    fn start_work(&mut self, work: Option<Work>) {
        self.work = work;
        self.work_since = now_ms();
    }

    /// Where a log line comes from: "server", "map" (building the world), "carbon" or "oxide"
    /// (and their plugins), "rustedit" or "rcon".
    pub fn source(&self, line: &str) -> &'static str {
        let l = line.trim_start();
        let framework = match self.framework {
            Some("Carbon") => "carbon",
            Some("Oxide") => "oxide",
            _ => "server",
        };
        if l.starts_with("[RustEdit]") {
            "rustedit"
        } else if l.starts_with("Oxide assembly conversion") {
            // Carbon converting an Oxide extension.
            framework
        } else if l.starts_with("Carbon") || l.starts_with("[Carbon") || l.contains("Batch completed! OSI") {
            "carbon"
        } else if l.starts_with("[Oxide]") || l.starts_with("Oxide") || l.starts_with("Loading Oxide") || (l.starts_with("Load") && l.contains(" extension")) {
            "oxide"
        } else if plugin_name(l).is_some() || l.starts_with("Unloaded plugin ") || l.starts_with("Calling '") || self.is_plugin_output(l) {
            framework
        } else if l.starts_with("WebSocket RCON") || l.starts_with("RCon") || l.starts_with("[RCON]") {
            "rcon"
        } else if l.starts_with("Loading custom map")
            || l.starts_with("Generating procedural map")
            || l.starts_with("Generated ocean patrol path")
            || world_step(l).is_some_and(|s| s != "asset_warmup")
            || (l.starts_with("Spawning ") && l.contains(" entities"))
            || l.starts_with("Postprocessing ")
            || l.starts_with("Couldn't load server/")
            || l.starts_with("Loading save")
            || l.starts_with("[Rust.MapCache")
        {
            "map"
        } else {
            "server"
        }
    }

    /// "[TebexPlugin] ..." from a plugin that loaded.
    fn is_plugin_output(&self, line: &str) -> bool {
        let Some((name, _)) = line.strip_prefix('[').and_then(|rest| rest.split_once(']')) else { return false };
        self.plugins.iter().any(|p| p.eq_ignore_ascii_case(name) || p.replace(' ', "").eq_ignore_ascii_case(name))
    }
}

/// "Loaded plugin TebexPlugin v3.0.0 by Tebex [979ms]" -> "TebexPlugin"
fn plugin_name(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("Preloaded plugin ").or_else(|| line.strip_prefix("Loaded plugin "))?;
    let name = rest.split(" v").next().unwrap_or(rest).trim();
    (!name.is_empty()).then_some(name)
}

/// "Asset Warmup (1234/8996)"
fn asset_warmup(line: &str) -> Option<(u32, u32)> {
    let inner = line.strip_prefix("Asset Warmup (")?.strip_suffix(')')?;
    let (done, total) = inner.split_once('/')?;
    Some((done.trim().parse().ok()?, total.trim().parse().ok()?))
}

/// "[7.1s] Terrain Erosion" -> "Terrain Erosion"
fn world_step(line: &str) -> Option<&str> {
    let rest = line.strip_prefix('[')?;
    let (secs, name) = rest.split_once("s] ")?;
    secs.parse::<f32>().ok()?;
    let name = name.trim();
    (!name.is_empty()).then_some(name)
}

fn trim_dots(line: &str) -> String {
    let line = line.trim_end_matches('.');
    let mut s = line.to_string();
    if let Some(first) = s.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    s
}

/// Lines that look alarming but are normal for every server start (the dedicated server runs
/// without a graphics card, so Unity complains about shaders it never needs).
pub fn is_noise(line: &str) -> bool {
    let line = line.trim();
    line.is_empty()
        || line.starts_with("[CrashSessionMarker]")
        || matches!(
            line,
            "Command 'batchmode' not found" | "Command 'logfile' not found" | "Command 'rcon.password' not found" | "AsyncResourceUpload failed."
        )
        || (line.starts_with("The shader ") && line.ends_with("is not supported on this platform!"))
        || line.starts_with("Missing shader in ")
        || (line.starts_with("Shader '") && line.contains("fallback shader"))
        || line.starts_with("HDR Render Texture not supported")
        || line.starts_with("3D Noise requires higher shader capabilities")
        || line.starts_with("Microsoft Media Foundation video decoding")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Level {
    Log,
    Warn,
    Error,
    Ok,
}

pub fn classify(line: &str) -> Level {
    if line.starts_with("Server startup complete") {
        Level::Ok
    } else if line.contains("Exception") || line.starts_with("Error") || line.starts_with("[Error]") || line.contains("Failed to load map") {
        Level::Error
    } else if line.starts_with("Warning") || line.starts_with("[Warning]") || line.contains("failed!") {
        Level::Warn
    } else {
        Level::Log
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn procgen_boot_log_walks_every_stage() {
        let log = r#"[Bootstrap] Opening Bundles
Loading shared/content.bundle
[Bootstrap] Loading Menu Prefabs
[Bootstrap] Loading Game Manifest
[Bootstrap] Bootstrap Config
Command 'rcon.password' not found
[Bootstrap] Loading Items
Carbon 2.0.262.0 [2026.10.01.0] 8a81d70 on Windows
Preloaded plugin RustEdit Core v1.0.1 by Chaos
Asset Warmup (1/8996)
Asset Warmup (8996/8996)
[0.5s] asset_warmup
WebSocket RCON Started on 127.0.0.1:28016
Generating procedural map of size 3000 with seed 1184645116
[1.3s] Loading Monument Prefabs
[7.1s] Terrain Erosion
[71.3s] Processing World
Couldn't load server/droplaunch_procgen/proceduralmap.3000.1184645116.289.sav - file doesn't exist
Spawning 3081 entities from map
Postprocessing 3081 entities from map
[RustNav] No navmesh loaded from save, building navmesh now
Initializing 40189 entity links
Preparing Occlusion Grid (1500, 100, 1500)
SteamServer Initialized
Server startup complete
[RustNav] Navmesh 'default' is now fully built in 56.59 seconds (79524 tiles)."#;
        let mut t = Tracker::default();
        let mut stages = vec![t.stage];
        let mut last_pct = t.percent;
        for line in log.lines() {
            t.feed(line);
            assert!(t.percent >= last_pct, "progress went back at {line}");
            last_pct = t.percent;
            if stages.last() != Some(&t.stage) {
                stages.push(t.stage);
            }
            if line.starts_with("WebSocket RCON") {
                assert!(t.rcon_started);
            }
        }
        use Stage::*;
        assert_eq!(stages, vec![Starting, GameFiles, Mods, Map, Entities, Finishing, Ready]);
        assert_eq!(t.framework, Some("Carbon"));
        assert_eq!(t.percent, 100.0);
    }

    #[test]
    fn custom_map_and_noise() {
        let mut t = Tracker::default();
        t.feed("Loading custom map from file:///C:/Maps/race map.map");
        assert_eq!((t.stage, t.detail.as_str()), (Stage::Map, "Loading the custom map"));
        assert_eq!(t.work, Some(Work::ReadingMap));
        t.feed("[0.3s] Downloading World");
        assert_eq!((t.detail.as_str(), t.work), ("Spawning the world…", Some(Work::SpawningWorld)));
        t.feed("[3.3s] Spawning World");
        t.feed("[0.7s] Terrain Mesh");
        t.feed("[0.9s] Processing World");
        t.feed("[RustEdit] - No ocean patrol path data found in map file");
        // Where some maps hang for many minutes (prefab test mode skips it).
        assert_eq!((t.detail.as_str(), t.work), ("Generating the ocean paths…", Some(Work::OceanPaths)));
        t.feed("Generated ocean patrol path with node count: 292");
        t.feed("[4.4s] Ocean Patrol Paths");
        assert_eq!(t.work, None);
        t.feed("Spawning 1301 entities from map");
        assert_eq!((t.stage, t.work), (Stage::Entities, None));
        assert!(!t.restart_announced);
        t.feed("Restarting in 5 seconds");
        assert!(t.restart_announced);
        assert!(is_noise("Missing shader in Main Camera (TOD_Scattering)"));
        assert!(is_noise("Command 'logfile' not found"));
        assert!(!is_noise("Server startup complete"));
        assert_eq!(classify("NullReferenceException: Object reference not set"), Level::Error);
    }

    #[test]
    fn issues_and_sources() {
        let mut t = Tracker::default();
        t.feed("Carbon 2.0.262.0 [2026.10.01.0] 8a81d70 on Windows");
        t.feed("Loaded plugin TebexPlugin v3.0.0 by Tebex [979ms]");
        assert_eq!(t.source("Carbon Analytics are ON."), "carbon");
        assert_eq!(t.source("Preloaded plugin RustEdit Core v1.0.1 by Chaos"), "carbon");
        assert_eq!(t.source("[TebexPlugin] Authenticated with the store."), "carbon");
        assert_eq!(t.source("[RustEdit] - No loot data found in map file"), "rustedit");
        assert_eq!(t.source("WebSocket RCON Started on 127.0.0.1:28016"), "rcon");
        assert_eq!(t.source("[1.6s] Ocean Patrol Paths"), "map");
        assert_eq!(t.source("Spawning 287 entities from map"), "map");
        assert_eq!(t.source("[RustNav] No navmesh loaded from save, building navmesh now"), "server");
        assert_eq!(t.source("[0.4s] asset_warmup"), "server");
        let oxide = Tracker { framework: Some("Oxide"), ..Tracker::default() };
        assert_eq!(oxide.source("Loaded plugin Kits v4.4.0 by k1lly0u (0.21s)"), "oxide");
        assert_eq!(oxide.source("Loaded extension CSharp v2.0.4218"), "oxide");

        assert!(t.issues.is_empty());
        t.feed("Server startup complete");
        // Spotted after the boot too.
        assert!(t.feed("  at CargoShip.UpdateMovement () [0x0001c] in <abc>:0"));
        t.feed("NullReferenceException at TriggeredEventPrefab.RunEvent");
        t.feed("Kicking 7656119xxxxxxxxxx/Player (AntiHack: Terrain Violation)");
        assert_eq!(t.issues, vec![Issue::CargoShip, Issue::Events, Issue::Terrain]);
        assert!(!t.feed("  at CargoShip.UpdateMovement () [0x0001c] in <abc>:0"), "each one once");
        assert_eq!(issue("Preloaded plugin RustEdit Ocean Path v1.0.1 by Chaos"), None);
    }
}
