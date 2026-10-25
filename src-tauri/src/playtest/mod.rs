//! The playtest kit: testers type `/bug <what's wrong>` in Rust's chat, and the report shows up
//! on DropLaunch's map of the server, live, where they stood and looked.
//!
//! A small Carbon/Oxide plugin (DropLaunchPlaytest.cs, put in the server's plugins folder while
//! the kit is on) prints each report to the server's log as `DLBUG1 {json}`, which the server
//! manager hands over. Over RCON the plugin also tells where everyone is (`droplaunch.players`),
//! teleports the host to a report (`droplaunch.tp`) and thanks the tester (`droplaunch.tell`);
//! those commands only work from the server's console and RCON, not for players.
//!
//! Reports are kept per map in the data folder (Playtest/<map>.json), across versions of the
//! map: "RaceTrack_v19.map" and "RaceTrack_v20.map" share one list.

pub mod mapview;

use crate::t;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

use crate::config::ConfigStore;
use crate::mods::{self, Framework};
use crate::server::{Phase, PlaytestServer, ServerManager};

const PLUGIN_FILE: &str = "DropLaunchPlaytest.cs";
const PLUGIN: &str = include_str!("DropLaunchPlaytest.cs");
const MARKER: &str = "DLBUG1 ";
const TICK: Duration = Duration::from_millis(500);
/// How often the plugin is asked where the testers are.
const PLAYERS_EVERY: Duration = Duration::from_secs(2);
/// A plugin that hasn't answered this long after the server got ready didn't load (a compile
/// error, or the framework isn't running).
const LOAD_GRACE: Duration = Duration::from_secs(90);
const MAX_REPORTS: usize = 500;

/// The JSON of a bug report in a server log line.
pub fn report_json(line: &str) -> Option<&str> {
    let json = line[line.find(MARKER)? + MARKER.len()..].trim();
    json.starts_with('{').then_some(json)
}

/// The console's line for a report.
pub fn describe(json: &str) -> String {
    match serde_json::from_str::<Incoming>(json) {
        Ok(r) => t!("bug.line", name = r.name, grid = if r.grid.is_empty() { "?" } else { &r.grid }, text = r.text),
        Err(_) => t!("bug.lineRaw", json),
    }
}

/// Where the plugin goes in a server folder (None: no Carbon or Oxide there).
pub fn plugin_path(server_dir: &Path) -> Option<PathBuf> {
    match mods::detect(server_dir) {
        Framework::Carbon => Some(server_dir.join("carbon").join("plugins").join(PLUGIN_FILE)),
        Framework::Oxide => Some(server_dir.join("oxide").join("plugins").join(PLUGIN_FILE)),
        Framework::None => None,
    }
}

/// Put the plugin in the server's plugins folder (Carbon and Oxide load new files there by
/// themselves, also while the server runs). None: no framework to run it.
pub fn install_plugin(server_dir: &Path) -> std::io::Result<Option<PathBuf>> {
    let Some(path) = plugin_path(server_dir) else { return Ok(None) };
    if std::fs::read_to_string(&path).is_ok_and(|text| text == PLUGIN) {
        return Ok(Some(path));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, PLUGIN)?;
    Ok(Some(path))
}

/// Take the plugin out again (the frameworks unload a removed file).
pub fn remove_plugin(server_dir: &Path) {
    for framework in ["carbon", "oxide"] {
        let _ = std::fs::remove_file(server_dir.join(framework).join("plugins").join(PLUGIN_FILE));
    }
}

/// A report as the plugin prints it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Incoming {
    id: String,
    #[serde(default)]
    steam_id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    grid: String,
    #[serde(default)]
    x: f32,
    #[serde(default)]
    y: f32,
    #[serde(default)]
    z: f32,
    #[serde(default)]
    yaw: f32,
    #[serde(default)]
    pitch: f32,
    look: Option<String>,
    look_x: Option<f32>,
    look_y: Option<f32>,
    look_z: Option<f32>,
    look_distance: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub id: String,
    /// Numbered per map: #1, #2, ...
    pub n: u32,
    pub text: String,
    pub name: String,
    pub steam_id: String,
    /// The in-game map's grid square ("G12").
    pub grid: String,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Which way they looked: degrees from north, clockwise; and up (+) or down.
    pub yaw: f32,
    #[serde(default)]
    pub pitch: f32,
    /// What they looked at (a prefab, "terrain", ...) and where that is.
    pub look: Option<String>,
    pub look_at: Option<[f32; 3]>,
    pub look_distance: Option<f32>,
    /// Milliseconds since 1970.
    pub time: i64,
    /// The map file (version) it was reported on.
    pub version: String,
    pub fixed: Option<Fixed>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fixed {
    pub time: i64,
    /// The map version it was marked fixed with.
    pub version: String,
}

/// The reports kept, over all maps.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportsSummary {
    pub reports: usize,
    pub maps: usize,
}

/// A map's reports, kept on disk, and the names of its monuments (per version of the map).
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Book {
    title: String,
    next: u32,
    reports: Vec<Report>,
    /// Newest first.
    monuments: Vec<VersionMonuments>,
}

/// A monument the in-game map names, and where it is (from the running server).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Monument {
    pub name: String,
    pub x: f32,
    pub z: f32,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct VersionMonuments {
    version: String,
    list: Vec<Monument>,
}

/// Monuments named for so many versions of a map are kept.
const MONUMENT_VERSIONS: usize = 6;

impl Book {
    /// The names for this version of the map, else the newest known (monuments rarely move).
    fn monuments_for(&self, version: &str) -> Vec<Monument> {
        self.monuments.iter().find(|m| m.version == version).or(self.monuments.first()).map(|m| m.list.clone()).unwrap_or_default()
    }
}

#[derive(Deserialize)]
struct MonumentsReply {
    #[serde(default)]
    monuments: Vec<Monument>,
}

/// What the map viewer shows on a map.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapNotes {
    pub reports: Vec<Report>,
    pub monuments: Vec<Monument>,
}

/// Where a tester is right now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Player {
    pub id: String,
    pub name: String,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    #[serde(default)]
    pub yaw: f32,
    #[serde(default)]
    pub sleeping: bool,
    #[serde(default)]
    pub dead: bool,
}

/// Where Rust's map grid lies: a cell's size and the world position of its top left corner, as
/// the plugin measured it with Rust's own grid names ("A0" or "A1" first).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GridInfo {
    pub cell: f32,
    #[serde(default)]
    pub cell_z: f32,
    pub x0: f32,
    pub z0: f32,
    #[serde(default)]
    pub first: String,
}

#[derive(Deserialize)]
struct PlayersReply {
    #[serde(default)]
    size: f32,
    grid: Option<GridInfo>,
    #[serde(default)]
    players: Vec<Player>,
}

/// The map whose reports are shown.
#[derive(Debug, Clone, PartialEq)]
struct MapRef {
    key: String,
    title: String,
    /// The file name of this version ("Procedural map" for that one).
    version: String,
    /// The custom map's file.
    file: Option<PathBuf>,
    procgen: bool,
    /// server/<identity> (the procedural map's file is made there).
    identity_dir: Option<PathBuf>,
}

/// "RaceTrack_v19" -> "RaceTrack": versions of a map share their reports.
fn without_version(stem: &str) -> &str {
    static VERSION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)[\s_.\-]*v\d+(?:[._]\d+)*$").unwrap());
    let base = VERSION.replace(stem, "");
    let base = base.trim_end_matches([' ', '_', '-', '.']);
    if base.is_empty() {
        stem
    } else {
        &stem[..base.len()]
    }
}

/// A file name for the map's reports.
fn book_key(title: &str) -> String {
    let key: String = title.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).take(80).collect();
    if key.trim_matches('_').is_empty() {
        "map".into()
    } else {
        key
    }
}

fn map_ref(server: &PlaytestServer) -> MapRef {
    match &server.map {
        Some(file) => {
            let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| server.title.clone());
            let title = without_version(&stem).to_string();
            MapRef {
                key: book_key(&title),
                title,
                version: file.file_name().map_or_else(|| stem.clone(), |n| n.to_string_lossy().into_owned()),
                file: Some(file.clone()),
                procgen: false,
                identity_dir: server.identity_dir.clone(),
            }
        }
        None => MapRef {
            key: if server.procgen { "procedural-map".into() } else { book_key(&server.title) },
            title: if server.procgen { t!("maps.procgen.name") } else { server.title.clone() },
            // Kept with the reports: the same in every language.
            version: if server.procgen { "Procedural map".into() } else { server.title.clone() },
            file: None,
            procgen: server.procgen,
            identity_dir: server.identity_dir.clone(),
        },
    }
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

fn modified_ms(path: &Path) -> Option<i64> {
    let t = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(t.duration_since(UNIX_EPOCH).ok()?.as_millis() as i64)
}

/// When the plugin made a report: its id starts with .NET ticks (100 ns since the year 1).
fn report_time(id: &str) -> Option<i64> {
    let ticks: i64 = id.split('-').next()?.parse().ok()?;
    let ms = (ticks - 621_355_968_000_000_000) / 10_000;
    (ms > 1_500_000_000_000 && ms < now_ms() + 86_400_000).then_some(ms)
}

// ---------------------------------------------------------------- what the UI gets

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaytestView {
    /// The kit is on (the plugin goes into servers' plugin folders).
    pub enabled: bool,
    /// noServer | off | noFramework | starting | loading | notLoaded | active
    pub plugin: &'static str,
    pub server: Option<ServerRef>,
    pub map: Option<MapView>,
    pub reports: Vec<Report>,
    /// The monuments' names on this version of the map (once the plugin told them).
    pub monuments: Vec<Monument>,
    pub grid: Option<GridInfo>,
    /// The world's size in metres, as the server tells it.
    pub size: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerRef {
    pub id: u64,
    pub title: String,
    /// booting | ready | stopping
    pub phase: &'static str,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapView {
    pub key: String,
    pub title: String,
    pub version: String,
    pub procgen: bool,
    /// When this version was saved (the file's time).
    pub saved: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayersEvent {
    pub server_id: u64,
    pub players: Vec<Player>,
}

// ---------------------------------------------------------------- state

#[derive(Default)]
struct State {
    map: Option<MapRef>,
    saved: Option<i64>,
    book: Book,
    server: Option<ServerRef>,
    /// The server folder the plugin was put in (or taken out of), and whether it was put in.
    plugin_dir: Option<(PathBuf, bool)>,
    framework: Option<Framework>,
    /// The plugin answered: it runs.
    answered: bool,
    /// The monuments' names were asked for (once per server run).
    monuments_asked: bool,
    ready_since: Option<Instant>,
    last_players: Option<Instant>,
    players: Vec<Player>,
    grid: Option<GridInfo>,
    size: Option<f32>,
}

#[derive(Clone)]
pub struct Playtest {
    inner: Arc<Mutex<State>>,
    server: ServerManager,
    store: Arc<Mutex<ConfigStore>>,
    dir: PathBuf,
    app: Arc<OnceLock<AppHandle>>,
}

fn phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::Booting => "booting",
        Phase::Ready => "ready",
        Phase::Stopping => "stopping",
        Phase::Stopped => "stopped",
        Phase::Crashed => "crashed",
    }
}

impl Playtest {
    pub fn new(store: Arc<Mutex<ConfigStore>>, server: ServerManager) -> Self {
        let dir = store.lock().unwrap().data.join("Playtest");
        Self { inner: Arc::default(), server, store, dir, app: Arc::new(OnceLock::new()) }
    }

    pub fn run(&self, app: AppHandle) {
        let _ = self.app.set(app.clone());
        let me = self.clone();
        std::thread::Builder::new()
            .name("playtest".into())
            .spawn(move || {
                let mut last: Option<PlaytestView> = None;
                let mut last_players: Option<PlayersEvent> = None;
                loop {
                    std::thread::sleep(TICK);
                    me.tick();
                    let view = me.view();
                    if last.as_ref() != Some(&view) {
                        let _ = app.emit("playtest", &view);
                        last = Some(view);
                    }
                    let players = me.players_event();
                    if players != last_players {
                        if let Some(p) = &players {
                            let _ = app.emit("playtest-players", p);
                        }
                        last_players = players;
                    }
                }
            })
            .expect("spawn playtest thread");
    }

    fn enabled(&self) -> bool {
        self.store.lock().unwrap().config.playtest
    }

    /// Bug reports are on exactly when the DropLaunch plugin is in the server folder (the running
    /// server's, else the one in use): the setting follows the file, so a plugin that's there
    /// (put in by hand, for the whitelist, by an older DropLaunch) means bug reports, and one
    /// that's gone means none.
    fn follow_plugin_file(&self) {
        let live = self.server.live_dir();
        let mut store = self.store.lock().unwrap();
        if !store.config.linked {
            return;
        }
        let dir = live.unwrap_or_else(|| store.server_dir());
        let present = plugin_path(&dir).is_some_and(|p| p.is_file());
        if store.config.playtest != present {
            store.config.playtest = present;
            let _ = store.save();
        }
    }

    /// The plugin goes into servers for bug reports, or for the whitelist it checks.
    fn plugin_wanted(&self) -> bool {
        self.store.lock().unwrap().config.wants_plugin()
    }

    /// The plugin in or out of the running server's folder, as the settings say now.
    pub fn sync_plugin(&self) {
        let server = self.server.playtest_server();
        self.follow(server.as_ref(), self.plugin_wanted());
    }

    fn tick(&self) {
        let server = self.server.playtest_server();
        self.follow(server.as_ref(), self.plugin_wanted());
        self.follow_plugin_file();
        let enabled = self.enabled();
        self.save_game_kits();
        let reports = self.server.take_reports();
        if !reports.is_empty() {
            self.add_reports(reports, server.as_ref());
        }
        if let Some(s) = server.filter(|s| s.phase == Phase::Ready && s.rcon) {
            let due = {
                let mut st = self.inner.lock().unwrap();
                let due = enabled && st.framework.is_some_and(|f| f != Framework::None) && st.last_players.is_none_or(|t| t.elapsed() >= PLAYERS_EVERY);
                if due {
                    st.last_players = Some(Instant::now());
                }
                due
            };
            if due {
                self.poll_players(s.id);
                self.ask_monuments(s.id);
            }
        }
    }

    /// The names of the map's monuments, once per server run (kept with the map's reports).
    fn ask_monuments(&self, server_id: u64) {
        {
            let mut st = self.inner.lock().unwrap();
            if !st.answered || st.monuments_asked {
                return;
            }
            st.monuments_asked = true;
        }
        let Ok(reply) = self.server.rcon_call("droplaunch.monuments", Duration::from_secs(5)) else { return };
        // An older plugin answers "Command not found": it's replaced at the next server start.
        let Ok(found) = serde_json::from_str::<MonumentsReply>(reply.trim()) else { return };
        let mut st = self.inner.lock().unwrap();
        if st.server.as_ref().map(|s| s.id) != Some(server_id) {
            return;
        }
        let Some(version) = st.map.as_ref().map(|m| m.version.clone()) else { return };
        let list: Vec<Monument> = found.monuments.into_iter().filter(|m| !m.name.trim().is_empty()).map(|m| Monument { name: m.name.chars().take(60).collect(), ..m }).collect();
        st.book.monuments.retain(|m| m.version != version);
        st.book.monuments.insert(0, VersionMonuments { version, list });
        st.book.monuments.truncate(MONUMENT_VERSIONS);
        self.save(&st);
    }

    /// Kits saved in the game with /droplaunch kit: into the Kits folder, and the UI told.
    fn save_game_kits(&self) {
        let saved = self.server.take_game_kits();
        if saved.is_empty() {
            return;
        }
        let data = self.store.lock().unwrap().data.clone();
        for json in saved {
            match crate::kits::from_game(&data, &json) {
                Ok(kit) => {
                    if let Some(app) = self.app.get() {
                        let _ = app.emit("kit-saved", &kit);
                    }
                }
                Err(e) => eprintln!("kit from the game: {e:#}"),
            }
        }
    }

    /// Another server: its map's reports, and the plugin in its folder (or out of it).
    /// `enabled`: the plugin is wanted (bug reports or the whitelist).
    fn follow(&self, server: Option<&PlaytestServer>, enabled: bool) {
        let mut load: Option<MapRef> = None;
        let plugin_job = {
            let mut st = self.inner.lock().unwrap();
            let id = server.map(|s| s.id);
            if st.server.as_ref().map(|s| s.id) != id {
                st.answered = false;
                st.monuments_asked = false;
                st.ready_since = None;
                st.last_players = None;
                st.players.clear();
                st.framework = None;
                st.plugin_dir = None;
                if let Some(s) = server {
                    let map = map_ref(s);
                    if st.map.as_ref().map(|m| &m.key) != Some(&map.key) {
                        load = Some(map.clone());
                    }
                    st.saved = map.file.as_deref().and_then(modified_ms);
                    st.map = Some(map);
                    st.grid = None;
                    st.size = None;
                }
            }
            match server {
                Some(s) => {
                    st.server = Some(ServerRef { id: s.id, title: s.title.clone(), phase: phase_name(s.phase) });
                    if s.phase == Phase::Ready {
                        st.ready_since.get_or_insert_with(Instant::now);
                    }
                    let wanted = (s.dir.clone(), enabled);
                    (st.plugin_dir.as_ref() != Some(&wanted)).then_some(wanted)
                }
                None => {
                    st.server = None;
                    st.players.clear();
                    None
                }
            }
        };
        if let Some(map) = load {
            let book = self.read_book(&map.key);
            let mut st = self.inner.lock().unwrap();
            if st.map.as_ref().map(|m| &m.key) == Some(&map.key) {
                st.book = book;
            }
        }
        if let Some((dir, on)) = plugin_job {
            let framework = mods::detect(&dir);
            if on {
                if let Err(e) = install_plugin(&dir) {
                    eprintln!("playtest plugin: {e}");
                }
            } else {
                remove_plugin(&dir);
            }
            let mut st = self.inner.lock().unwrap();
            st.framework = Some(framework);
            st.plugin_dir = Some((dir, on));
            // Put in or taken out: it runs once it answers again.
            st.answered = false;
            st.players.clear();
        }
    }

    fn book_path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.json"))
    }

    fn read_book(&self, key: &str) -> Book {
        std::fs::read_to_string(self.book_path(key)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    fn save(&self, st: &State) {
        let Some(map) = &st.map else { return };
        let mut book = st.book.clone();
        book.title = map.title.clone();
        let result = std::fs::create_dir_all(&self.dir).and_then(|()| {
            let path = self.book_path(&map.key);
            let tmp = path.with_extension("json.tmp");
            std::fs::write(&tmp, serde_json::to_vec_pretty(&book).unwrap_or_default())?;
            std::fs::rename(&tmp, &path)
        });
        if let Err(e) = result {
            eprintln!("Could not save the playtest reports: {e}");
        }
    }

    fn add_reports(&self, raw: Vec<String>, server: Option<&PlaytestServer>) {
        if self.inner.lock().unwrap().map.is_none() {
            if let Some(s) = server {
                let map = map_ref(s);
                let book = self.read_book(&map.key);
                let mut st = self.inner.lock().unwrap();
                st.book = book;
                st.map = Some(map);
            }
        }
        let mut thanks = Vec::new();
        {
            let mut st = self.inner.lock().unwrap();
            let Some(version) = st.map.as_ref().map(|m| m.version.clone()) else { return };
            for json in raw {
                let Ok(r) = serde_json::from_str::<Incoming>(&json) else { continue };
                if r.id.is_empty() || st.book.reports.iter().any(|old| old.id == r.id) {
                    continue;
                }
                // Each map numbers its own reports; with none left (all deleted or cleared) the
                // next one is #1 again.
                if st.book.reports.is_empty() {
                    st.book.next = 0;
                }
                st.book.next += 1;
                let n = st.book.next;
                let steam_id: String = r.steam_id.chars().filter(char::is_ascii_digit).take(20).collect();
                if !steam_id.is_empty() {
                    thanks.push((steam_id.clone(), n));
                }
                st.book.reports.push(Report {
                    time: report_time(&r.id).unwrap_or_else(now_ms),
                    id: r.id,
                    n,
                    text: r.text.chars().take(300).collect(),
                    name: r.name.chars().filter(|c| !c.is_control()).take(40).collect(),
                    steam_id,
                    grid: r.grid.chars().take(8).collect(),
                    x: r.x,
                    y: r.y,
                    z: r.z,
                    yaw: r.yaw,
                    pitch: r.pitch,
                    look: r.look.filter(|l| !l.is_empty()).map(|l| l.chars().take(80).collect()),
                    look_at: match (r.look_x, r.look_y, r.look_z) {
                        (Some(x), Some(y), Some(z)) => Some([x, y, z]),
                        _ => None,
                    },
                    look_distance: r.look_distance,
                    version: version.clone(),
                    fixed: None,
                });
            }
            if thanks.is_empty() {
                return;
            }
            let extra = st.book.reports.len().saturating_sub(MAX_REPORTS);
            st.book.reports.drain(..extra);
            self.save(&st);
        }
        // The tester sees it arrived (only players on this server, through RCON).
        for (steam_id, n) in thanks {
            self.server.rcon_send(&format!("droplaunch.tell {steam_id} {}", t!("bug.thanks", n)));
        }
    }

    fn poll_players(&self, server_id: u64) {
        let reply = self.server.rcon_call("droplaunch.players", Duration::from_secs(3));
        let mut st = self.inner.lock().unwrap();
        if st.server.as_ref().map(|s| s.id) != Some(server_id) {
            return;
        }
        match reply {
            Ok(text) => match serde_json::from_str::<PlayersReply>(text.trim()) {
                Ok(r) => {
                    st.answered = true;
                    st.players = r.players;
                    if r.grid.as_ref().is_some_and(|g| g.cell > 1.0) {
                        st.grid = r.grid;
                    }
                    if r.size > 0.0 {
                        st.size = Some(r.size);
                    }
                }
                // "Command not found": not loaded (yet).
                Err(_) => {
                    st.answered = false;
                    st.players.clear();
                }
            },
            Err(_) => {}
        }
    }

    pub fn view(&self) -> PlaytestView {
        let enabled = self.enabled();
        let st = self.inner.lock().unwrap();
        let plugin = match &st.server {
            None => "noServer",
            Some(_) if !enabled => "off",
            Some(_) if st.framework == Some(Framework::None) => "noFramework",
            Some(_) if st.answered => "active",
            Some(s) if s.phase != "ready" => "starting",
            Some(_) if st.ready_since.is_some_and(|t| t.elapsed() > LOAD_GRACE) => "notLoaded",
            Some(_) => "loading",
        };
        PlaytestView {
            enabled,
            plugin,
            server: st.server.clone(),
            map: st.map.as_ref().map(|m| MapView { key: m.key.clone(), title: m.title.clone(), version: m.version.clone(), procgen: m.procgen, saved: st.saved }),
            reports: st.book.reports.clone(),
            monuments: st.map.as_ref().map(|m| st.book.monuments_for(&m.version)).unwrap_or_default(),
            grid: st.grid.clone(),
            size: st.size,
        }
    }

    fn players_event(&self) -> Option<PlayersEvent> {
        let st = self.inner.lock().unwrap();
        st.server.as_ref().map(|s| PlayersEvent { server_id: s.id, players: st.players.clone() })
    }

    /// Mark a report fixed (with the map version that runs now) or open again.
    pub fn set_fixed(&self, id: &str, fixed: bool) -> Result<PlaytestView, String> {
        {
            let mut st = self.inner.lock().unwrap();
            let version = st.map.as_ref().map(|m| m.version.clone()).unwrap_or_default();
            let report = st.book.reports.iter_mut().find(|r| r.id == id).ok_or_else(|| t!("bug.error.gone"))?;
            report.fixed = fixed.then(|| Fixed { time: now_ms(), version });
            self.save(&st);
        }
        Ok(self.view())
    }

    /// How many reports are kept, on how many maps (Settings).
    pub fn summary(&self) -> ReportsSummary {
        let (current, in_memory) = {
            let st = self.inner.lock().unwrap();
            (st.map.as_ref().map(|m| m.key.clone()), st.book.reports.len())
        };
        let mut out = ReportsSummary::default();
        for (key, book) in self.books() {
            let n = if current.as_deref() == Some(key.as_str()) { in_memory } else { book.reports.len() };
            if n > 0 {
                out.reports += n;
                out.maps += 1;
            }
        }
        out
    }

    /// Every map's book on disk: (key, book).
    fn books(&self) -> Vec<(String, Book)> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return Vec::new() };
        entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "json"))
            .filter_map(|p| {
                let key = p.file_stem()?.to_string_lossy().into_owned();
                let book = serde_json::from_str::<Book>(&std::fs::read_to_string(&p).ok()?).ok()?;
                Some((key, book))
            })
            .collect()
    }

    /// Every map's reports put aside (Playtest/Cleared/<date>/, not deleted), and their numbering
    /// back to #1 (the monuments' names stay). How many reports.
    pub fn clear_all(&self) -> Result<usize, String> {
        {
            // What's in memory first: the current map's newest reports.
            let st = self.inner.lock().unwrap();
            self.save(&st);
        }
        let cleared = self.dir.join("Cleared");
        let day = crate::client::today();
        let aside = (1..).map(|n| cleared.join(if n == 1 { day.clone() } else { format!("{day}_{n}") })).find(|p| !p.exists()).expect("a free name");
        let mut count = 0;
        for (key, mut book) in self.books() {
            if book.reports.is_empty() && book.next == 0 {
                continue;
            }
            let path = self.book_path(&key);
            let keep = std::fs::create_dir_all(&aside).and_then(|()| std::fs::copy(&path, aside.join(format!("{key}.json"))));
            keep.map_err(|e| t!("bug.error.clear", error = e))?;
            count += book.reports.len();
            book.reports.clear();
            book.next = 0;
            let text = serde_json::to_vec_pretty(&book).unwrap_or_default();
            std::fs::write(&path, text).map_err(|e| t!("bug.error.clear", error = e))?;
        }
        let mut st = self.inner.lock().unwrap();
        st.book.reports.clear();
        st.book.next = 0;
        Ok(count)
    }

    pub fn delete(&self, id: &str) -> Result<PlaytestView, String> {
        {
            let mut st = self.inner.lock().unwrap();
            let before = st.book.reports.len();
            st.book.reports.retain(|r| r.id != id);
            if st.book.reports.len() == before {
                return Err(t!("bug.error.gone"));
            }
            self.save(&st);
        }
        Ok(self.view())
    }

    /// The reports kept for a map file (None: the procedural map) and its monuments' names, as the
    /// map viewer shows them.
    pub fn notes_for(&self, map: Option<&Path>) -> MapNotes {
        let (key, version) = match map {
            Some(file) => (
                book_key(without_version(&file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default())),
                file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            ),
            None => ("procedural-map".into(), "Procedural map".into()),
        };
        let book = {
            let st = self.inner.lock().unwrap();
            st.map.as_ref().filter(|m| m.key == key).map(|_| st.book.clone())
        };
        let book = book.unwrap_or_else(|| self.read_book(&key));
        MapNotes { monuments: book.monuments_for(&version), reports: book.reports }
    }

    /// Teleport testers to a spot on the map (the plugin finds the ground there): one of them by
    /// Steam id, or everyone on the server (None). How many went.
    pub fn teleport_to(&self, x: f32, z: f32, player: Option<&str>) -> Result<u32, String> {
        if !self.inner.lock().unwrap().answered {
            return Err(t!("bug.error.noPlugin"));
        }
        let target = match player {
            Some(id) if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) => id.to_string(),
            Some(_) => return Err(t!("bug.error.notTester")),
            None => "*".into(),
        };
        let reply = self.server.rcon_call(&format!("droplaunch.tpto {target} {x:.2} {z:.2}"), Duration::from_secs(4))?;
        let reply = reply.trim();
        if let Some(n) = reply.strip_prefix("DLTP ok") {
            return Ok(n.trim().parse().unwrap_or(1));
        }
        match reply {
            "DLTP not-found" => Err(if player.is_some() { t!("bug.error.left") } else { t!("bug.error.nobody") }),
            // An older plugin (from before this command): it's replaced at the next server start.
            _ => Err(t!("bug.error.old")),
        }
    }

    /// Teleport the host (the server's owner) to a report's spot, or to a tester.
    pub fn teleport(&self, report: Option<&str>, player: Option<&str>) -> Result<(), String> {
        let owner = self.store.lock().unwrap().config.owner.clone();
        let owner = owner.filter(|o| !o.steam_id.is_empty() && o.steam_id.chars().all(|c| c.is_ascii_digit()));
        let Some(owner) = owner else {
            return Err(t!("bug.error.noOwner"));
        };
        let (x, y, z, yaw) = {
            let st = self.inner.lock().unwrap();
            if !st.answered {
                return Err(t!("bug.error.noPlugin"));
            }
            match (report, player) {
                (Some(id), _) => st.book.reports.iter().find(|r| r.id == id).map(|r| (r.x, r.y, r.z, r.yaw)).ok_or_else(|| t!("bug.error.gone"))?,
                (None, Some(id)) => st.players.iter().find(|p| p.id == id).map(|p| (p.x, p.y, p.z, p.yaw)).ok_or_else(|| t!("bug.error.left"))?,
                (None, None) => return Err(t!("bug.error.nothing")),
            }
        };
        let reply = self.server.rcon_call(&format!("droplaunch.tp {} {x:.2} {y:.2} {z:.2} {yaw:.1}", owner.steam_id), Duration::from_secs(4))?;
        match reply.trim() {
            "DLTP ok" => Ok(()),
            "DLTP not-found" => Err(t!("bug.error.notOnServer", name = owner.name)),
            "DLTP dead" => Err(t!("bug.error.dead")),
            _ => Err(t!("bug.error.noAnswer")),
        }
    }

    /// Turn the kit on or off; the running server's folder follows right away.
    pub fn set_enabled(&self, on: bool) -> Result<PlaytestView, String> {
        let live = self.server.live_dir();
        let dirs = {
            let mut store = self.store.lock().unwrap();
            // The whitelist is checked by the same plugin: it stays (and so do bug reports).
            if !on && store.config.whitelist_ids().is_some() {
                return Err(t!("bug.error.whitelist"));
            }
            store.config.playtest = on;
            store.save().map_err(|e| t!("error.saveSettings", error = e))?;
            let mut dirs = vec![store.server_dir()];
            dirs.extend(live.filter(|d| !dirs.contains(d)));
            dirs
        };
        // Into (or out of) the server folder right away, not only once a server starts: bug
        // reports are on exactly while the plugin is there.
        let mut placed = false;
        for dir in &dirs {
            if on {
                placed |= install_plugin(dir).map_err(|e| t!("job.plugin.failed", error = e))?.is_some();
            } else {
                remove_plugin(dir);
            }
        }
        if on && !placed {
            self.store.lock().unwrap().config.playtest = false;
            return Err(t!("job.plugin.needs"));
        }
        self.sync_plugin();
        Ok(self.view())
    }

    /// The picture of the map the reports are for: the custom map's file, or the procedural map
    /// the server made.
    pub fn picture(&self) -> Result<Arc<mapview::Picture>, String> {
        let map = self.inner.lock().unwrap().map.clone().ok_or_else(|| t!("bug.error.noMap"))?;
        let file = match (&map.file, &map.identity_dir) {
            (Some(file), _) => file.clone(),
            (None, Some(dir)) => mapview::newest_map(dir).ok_or_else(|| t!("bug.error.notBuilt"))?,
            (None, None) => return Err(t!("bug.error.whichMap")),
        };
        mapview::picture(&file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_are_found_in_log_lines() {
        let line = r#"[DropLaunch] DLBUG1 {"id":"1","name":"Ted","grid":"G12","text":"floating rock"}"#;
        let json = report_json(line).unwrap();
        assert!(json.starts_with('{'));
        assert_eq!(describe(json), "Bug report from Ted at G12: floating rock");
        assert_eq!(report_json("[CHAT] Ted : DLBUG1 hello"), None);
        assert_eq!(report_json("Loaded plugin DropLaunch v1.3.0 by DropLaunch"), None);
    }

    #[test]
    fn versions_share_their_reports() {
        assert_eq!(without_version("6KRaceDownv19"), "6KRaceDown");
        assert_eq!(without_version("RaceTrack_v20"), "RaceTrack");
        assert_eq!(without_version("my map v1.2"), "my map");
        assert_eq!(without_version("RedditMaps 4000"), "RedditMaps 4000");
        assert_eq!(without_version("v2"), "v2");
        assert_eq!(book_key("My Map: 2"), "my_map__2");
        assert_eq!(book_key("???"), "map");
    }

    #[test]
    fn report_times_come_from_the_plugin() {
        // 2026-10-01 00:00:00 UTC in .NET ticks.
        let ms = 1_790_812_800_000i64;
        let ticks = ms * 10_000 + 621_355_968_000_000_000;
        assert_eq!(report_time(&format!("{ticks}-76561198000000000")), Some(ms));
        assert_eq!(report_time("garbage"), None);
    }

    #[test]
    fn the_plugin_lands_in_the_frameworks_folder() {
        let dir = std::env::temp_dir().join(format!("droplaunch-playtest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("carbon").join("managed")).unwrap();
        assert!(install_plugin(&dir).unwrap().is_none(), "no framework yet");
        std::fs::write(dir.join("carbon").join("managed").join("Carbon.dll"), b"x").unwrap();
        let path = install_plugin(&dir).unwrap().unwrap();
        assert_eq!(path, dir.join("carbon").join("plugins").join(PLUGIN_FILE));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), PLUGIN);
        remove_plugin(&dir);
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
