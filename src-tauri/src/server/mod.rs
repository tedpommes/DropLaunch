//! The Rust server DropLaunch looks after. Start files run in their own console window (exactly
//! what a double-click would do); DropLaunch follows the server's log for boot progress, uses
//! WebRCON for the console, player counts and a clean stop, and picks up servers from this
//! folder that were already running (started earlier, or by double-clicking a start file).

pub mod boot;
pub mod query;
pub mod rcon;

use crate::t;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::net::{TcpListener, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

use crate::client;
use crate::config::ConfigStore;
use crate::owner::{self, SteamUser};
use crate::playtest;
use crate::whitelist;
use crate::platform::{self, ProcessHandle, ProcessJob};
use crate::startfile::{self, Layout, StartFile, Target, LAUNCH_ENV};
use crate::steamcmd::manifest::{self, SERVER_EXECUTABLE};
use boot::Tracker;

const KEEP_LINES: usize = 1500;
const TICK: Duration = Duration::from_millis(250);
const SCAN_EVERY: Duration = Duration::from_secs(3);
/// Often enough for the performance graph.
const INFO_EVERY: Duration = Duration::from_secs(2);
const QUIT_TIMEOUT: Duration = Duration::from_secs(45);
/// How long the start file may take to start RustDedicated.
const SPAWN_TIMEOUT: Duration = Duration::from_secs(20);
const INFO_ID: i32 = 2_000_000_000;
const QUIT_ID: i32 = INFO_ID - 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Booting,
    Ready,
    Stopping,
    Stopped,
    Crashed,
}

/// What `serverinfo` answers (the parts DropLaunch shows).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    #[serde(alias = "Players", default)]
    pub players: u32,
    #[serde(alias = "MaxPlayers", default)]
    pub max_players: u32,
    #[serde(alias = "Joining", default)]
    pub joining: u32,
    #[serde(alias = "EntityCount", default)]
    pub entity_count: u64,
    #[serde(alias = "Framerate", default)]
    pub framerate: f32,
    #[serde(alias = "Uptime", default)]
    pub uptime: u64,
    /// Megabytes: the game's own count, and what the process uses.
    #[serde(alias = "Memory", default)]
    pub memory: u64,
    #[serde(alias = "MemoryUsageSystem", default)]
    pub memory_system: u64,
    /// Bytes per second.
    #[serde(alias = "NetworkIn", default)]
    pub network_in: u64,
    #[serde(alias = "NetworkOut", default)]
    pub network_out: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerView {
    pub id: u64,
    pub title: String,
    /// The map file; None for the procedural map (or an unknown server).
    pub map: Option<String>,
    pub procgen: bool,
    pub start_file: Option<String>,
    pub log_file: Option<String>,
    /// Started with the map's prefab test mode file.
    pub prefab: bool,
    pub phase: Phase,
    pub boot: Tracker,
    pub started_at: i64,
    pub ready_at: Option<i64>,
    pub ended_at: Option<i64>,
    /// The in-app console (RCON) is connected.
    pub rcon: bool,
    pub info: Option<ServerInfo>,
    pub port: u16,
    pub connect: String,
    /// Found running instead of started by this DropLaunch session.
    pub attached: bool,
    /// Stopping to start again (restart, or another map was picked).
    pub restarting: bool,
    pub message: Option<String>,
    /// The branch of the server folder it runs from.
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleLine {
    pub text: String,
    /// log | warn | error | ok | cmd | reply | app
    pub kind: &'static str,
    /// Who said it: server | map | carbon | oxide | rustedit | rcon | droplaunch
    pub source: &'static str,
    pub time: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ServerEvent {
    State { server: Option<ServerView> },
    Lines { lines: Vec<ConsoleLine> },
}

/// What happened when the UI asked for a start.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "result", rename_all = "camelCase")]
pub enum StartReply {
    Started { server: ServerView },
    /// Another server runs; ask before replacing it.
    Busy { title: String },
    /// The running server is stopping; this one starts right after.
    Queued { title: String },
    /// Oxide is installed but inactive (an update replaced its files); ask first.
    OxideInactive,
}

/// Everything needed to (re)start a server later, e.g. after a restart's stop.
#[derive(Debug, Clone)]
pub struct StartContext {
    pub base: PathBuf,
    /// The server folder it starts from (the active branch's, or a running server's own).
    pub server_dir: PathBuf,
    pub steamcmd_dir: PathBuf,
    /// Made the owner of every DropLaunch server (users.cfg).
    pub owner: Option<SteamUser>,
    /// Remove the Rust game's cached copies of a map before it starts.
    pub clean_client_maps: bool,
    /// Where those copies are moved instead of deleted (None: deleted).
    pub map_backups: Option<PathBuf>,
    /// Who may join (None: everyone; the DropLaunch plugin checks it).
    pub whitelist: Option<Vec<String>>,
}

impl StartContext {
    /// Folders and owner as they're set right now, for the active branch's server folder.
    pub fn from_store(store: &ConfigStore) -> Self {
        Self::with_dir(store, store.server_dir())
    }

    /// The same for a server folder that's known already (reads nothing from the disk).
    pub fn with_dir(store: &ConfigStore, server_dir: PathBuf) -> Self {
        Self {
            base: store.base.clone(),
            server_dir,
            steamcmd_dir: store.steamcmd_dir(),
            owner: store.config.owner.clone(),
            clean_client_maps: store.config.clean_client_maps,
            map_backups: store.config.backup_client_maps.then(|| store.data.join(client::BACKUP_DIR)),
            whitelist: store.config.whitelist_ids(),
        }
    }
}

/// A start that waits for the running server to stop.
struct Pending {
    target: Target,
    prefab: bool,
    ctx: StartContext,
}

#[derive(Clone)]
pub struct ServerManager {
    inner: Arc<Mutex<Inner>>,
    store: Arc<Mutex<ConfigStore>>,
    app: Arc<OnceLock<AppHandle>>,
}

#[derive(Default)]
struct Inner {
    live: Option<Live>,
    /// The last server that ended, shown until the next one starts.
    last: Option<ServerView>,
    lines: VecDeque<ConsoleLine>,
    outbox: Vec<ConsoleLine>,
    dirty: bool,
    next_id: u64,
    next_cmd: i32,
    last_scan: Option<Instant>,
    pending: Option<Pending>,
    /// DropLaunch is closing: stop following (and don't reconnect RCON).
    shutting_down: bool,
    /// Bug reports from the playtest plugin (their JSON), until the playtest kit takes them.
    reports: Vec<String>,
    /// Kits saved in the game (/droplaunch kit, their JSON), until they're added to the kits.
    game_kits: Vec<String>,
    /// RCON commands sent for DropLaunch itself (not the console), waiting for their reply.
    waiting: HashMap<i32, mpsc::Sender<String>>,
}

struct Live {
    id: u64,
    title: String,
    target: Option<Target>,
    prefab: bool,
    start: Option<StartFile>,
    process: Option<ProcessHandle>,
    shell_pid: Option<u32>,
    job: Option<ProcessJob>,
    spawn_deadline: Instant,
    phase: Phase,
    boot: Tracker,
    tail: Option<LogTail>,
    rcon: Option<rcon::Client>,
    rcon_connected: bool,
    info: Option<ServerInfo>,
    last_info: Option<Instant>,
    started_at: i64,
    ready_at: Option<i64>,
    stop_requested: bool,
    stop_deadline: Option<Instant>,
    attached: bool,
    recent: VecDeque<String>,
    message: Option<String>,
    branch: Option<String>,
    /// The server folder it runs from (each branch has its own).
    dir: PathBuf,
}

/// A server found running that DropLaunch didn't start (this session), read back from its log.
struct Found {
    live: Live,
    pid: u32,
    /// The end of its log, for the console.
    lines: VecDeque<String>,
    /// Bug reports it logged (the playtest kit keeps each one once).
    reports: Vec<String>,
}

/// The running server, as the playtest kit needs it.
#[derive(Debug, Clone)]
pub struct PlaytestServer {
    pub id: u64,
    pub title: String,
    pub phase: Phase,
    /// The RCON console is connected (DropLaunch can ask the plugin things).
    pub rcon: bool,
    /// The server folder it runs from.
    pub dir: PathBuf,
    /// The custom map's file (None: the procedural map, or unknown).
    pub map: Option<PathBuf>,
    pub procgen: bool,
    /// server/<identity>: where the server keeps its world (the procedural map's file too).
    pub identity_dir: Option<PathBuf>,
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

impl Live {
    fn view(&self, pending: bool) -> ServerView {
        let port = self.start.as_ref().map_or(startfile::DEFAULT_PORT, |s| s.server_port);
        ServerView {
            id: self.id,
            title: self.title.clone(),
            map: self.start.as_ref().and_then(|s| s.map.as_ref()).map(|m| m.display().to_string()),
            procgen: matches!(self.target, Some(Target::Procgen)),
            start_file: self.start.as_ref().map(|s| s.path.display().to_string()),
            log_file: self.start.as_ref().and_then(|s| s.log_file.as_ref()).map(|p| p.display().to_string()),
            prefab: self.prefab,
            phase: self.phase,
            boot: self.boot.clone(),
            started_at: self.started_at,
            ready_at: self.ready_at,
            ended_at: None,
            rcon: self.rcon_connected,
            info: self.info.clone(),
            port,
            connect: format!("client.connect 127.0.0.1:{port}"),
            attached: self.attached,
            restarting: pending,
            message: self.message.clone(),
            branch: self.branch.clone(),
        }
    }

    fn kill(&mut self) {
        if let Some(job) = &self.job {
            job.terminate();
        } else if let Some(process) = &self.process {
            process.terminate();
        }
    }
}

impl Inner {
    /// A line from DropLaunch (or an RCON command and its reply).
    fn push(&mut self, text: impl Into<String>, kind: &'static str) {
        let source = if matches!(kind, "cmd" | "reply") { "rcon" } else { "droplaunch" };
        self.push_from(text, kind, source);
    }

    fn push_from(&mut self, text: impl Into<String>, kind: &'static str, source: &'static str) {
        let line = ConsoleLine { text: text.into(), kind, source, time: now_ms() };
        self.outbox.push(line.clone());
        self.lines.push_back(line);
        while self.lines.len() > KEEP_LINES {
            self.lines.pop_front();
        }
    }

    fn view(&self) -> Option<ServerView> {
        match &self.live {
            Some(live) => Some(live.view(self.pending.is_some())),
            None => self.last.clone(),
        }
    }
}

impl ServerManager {
    pub fn new(store: Arc<Mutex<ConfigStore>>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner { next_cmd: 1, ..Default::default() })),
            store,
            app: Arc::new(OnceLock::new()),
        }
    }

    /// The running server's port and branch (for Join).
    pub fn join_target(&self) -> (u16, String) {
        let inner = self.inner.lock().unwrap();
        let live = inner.live.as_ref();
        (
            live.and_then(|l| l.start.as_ref()).map_or(startfile::DEFAULT_PORT, |s| s.server_port),
            live.and_then(|l| l.branch.clone()).unwrap_or_else(|| "public".into()),
        )
    }

    /// Start following servers; events go to the UI as "server" events.
    pub fn run(&self, app: AppHandle) {
        let _ = self.app.set(app);
        let me = self.clone();
        std::thread::Builder::new()
            .name("server-monitor".into())
            .spawn(move || loop {
                me.tick();
                std::thread::sleep(TICK);
            })
            .expect("spawn server monitor");
    }

    /// DropLaunch is closing: say goodbye on RCON instead of dropping the connection (the
    /// server logs an exception for that). The server itself keeps running.
    pub fn shutdown(&self) {
        let client = {
            let mut inner = self.inner.lock().unwrap();
            inner.shutting_down = true;
            inner.live.as_mut().and_then(|l| l.rcon.take())
        };
        if let Some(client) = client {
            client.close(Duration::from_secs(1));
        }
    }

    /// A server from this folder is running (or starting/stopping).
    pub fn is_live(&self) -> bool {
        self.inner.lock().unwrap().live.is_some()
    }

    /// The server folder of the server that runs (each branch has its own).
    pub fn live_dir(&self) -> Option<PathBuf> {
        self.inner.lock().unwrap().live.as_ref().map(|l| l.dir.clone())
    }

    /// A server from `dir` runs (its files are in use).
    pub fn runs_from(&self, dir: &Path) -> bool {
        self.live_dir().is_some_and(|d| startfile::same_path(&d, dir))
    }

    pub fn status(&self) -> (Option<ServerView>, Vec<ConsoleLine>) {
        let inner = self.inner.lock().unwrap();
        (inner.view(), inner.lines.iter().cloned().collect())
    }

    pub fn start(&self, target: Target, prefab: bool, ctx: StartContext, replace: bool) -> Result<StartReply, String> {
        let mut inner = self.inner.lock().unwrap();
        if let Some(live) = inner.live.as_mut() {
            let title = live.title.clone();
            if !replace {
                return Ok(StartReply::Busy { title });
            }
            let next = if prefab && matches!(target, Target::Map(_)) { t!("serverlog.prefabTitle", title = target.title()) } else { target.title() };
            request_stop(live, false);
            inner.pending = Some(Pending { target, prefab, ctx });
            inner.push(t!("serverlog.stoppingFor", title, next), "app");
            inner.dirty = true;
            return Ok(StartReply::Queued { title });
        }
        let live = self.launch(&mut inner, target, prefab, &ctx)?;
        let view = live.view(false);
        inner.live = Some(live);
        inner.last = None;
        inner.dirty = true;
        Ok(StartReply::Started { server: view })
    }

    pub fn stop(&self, force: bool) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap();
        inner.pending = None;
        let Some(live) = inner.live.as_mut() else { return Err(t!("error.noServer")) };
        let graceful = request_stop(live, force);
        let title = live.title.clone();
        inner.push(
            if graceful { t!("serverlog.stoppingSave", title) } else { t!("serverlog.stopping", title) },
            "app",
        );
        inner.dirty = true;
        Ok(())
    }

    /// Stop and start the same start file again, from the server's own folder (also when another
    /// branch was picked meanwhile).
    pub fn restart(&self, ctx: StartContext) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap();
        let Some(live) = inner.live.as_mut() else { return Err(t!("error.noServer")) };
        let Some(target) = live.target.clone() else {
            return Err(t!("error.cantRestart"));
        };
        let prefab = live.prefab;
        let ctx = StartContext { server_dir: live.dir.clone(), ..ctx };
        request_stop(live, false);
        let title = live.title.clone();
        inner.pending = Some(Pending { target, prefab, ctx });
        inner.push(t!("serverlog.restarting", title), "app");
        inner.dirty = true;
        Ok(())
    }

    pub fn command(&self, text: &str) -> Result<(), String> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(());
        }
        let mut inner = self.inner.lock().unwrap();
        let id = inner.next_cmd;
        inner.next_cmd = if id >= QUIT_ID - 1 { 1 } else { id + 1 };
        let Some(live) = inner.live.as_mut() else { return Err(t!("error.noServer")) };
        match (&live.rcon, live.rcon_connected) {
            (Some(client), true) => client.send(id, text),
            _ => return Err(t!("error.consoleNotYet")),
        }
        inner.push(format!("> {text}"), "cmd");
        Ok(())
    }

    /// Send a command over RCON for DropLaunch itself (not shown in the console) and wait for the
    /// reply (`tick` hands it over, so it takes up to a quarter second more than the server).
    pub fn rcon_call(&self, text: &str, timeout: Duration) -> Result<String, String> {
        let (tx, rx) = mpsc::channel();
        let id = {
            let mut inner = self.inner.lock().unwrap();
            let id = inner.next_cmd;
            inner.next_cmd = if id >= QUIT_ID - 1 { 1 } else { id + 1 };
            let Some(live) = inner.live.as_ref() else { return Err(t!("error.noServer")) };
            match (&live.rcon, live.rcon_connected) {
                (Some(client), true) => client.send(id, text),
                _ => return Err(t!("error.consoleNotConnected")),
            }
            inner.waiting.insert(id, tx);
            id
        };
        let reply = rx.recv_timeout(timeout).map_err(|_| t!("error.noAnswer"));
        if reply.is_err() {
            self.inner.lock().unwrap().waiting.remove(&id);
        }
        reply
    }

    /// The same without waiting for the reply.
    pub fn rcon_send(&self, text: &str) {
        let mut inner = self.inner.lock().unwrap();
        let id = inner.next_cmd;
        inner.next_cmd = if id >= QUIT_ID - 1 { 1 } else { id + 1 };
        if let Some(Live { rcon: Some(client), rcon_connected: true, .. }) = inner.live.as_ref() {
            client.send(id, text);
        }
    }

    /// Bug reports the server logged since the last call (the playtest plugin's JSON).
    pub fn take_reports(&self) -> Vec<String> {
        std::mem::take(&mut self.inner.lock().unwrap().reports)
    }

    /// Kits saved in the game since the last call (only from lines logged while DropLaunch
    /// follows the server: an older log read again doesn't bring a deleted kit back).
    pub fn take_game_kits(&self) -> Vec<String> {
        std::mem::take(&mut self.inner.lock().unwrap().game_kits)
    }

    /// The running server, for the playtest kit.
    pub fn playtest_server(&self) -> Option<PlaytestServer> {
        let inner = self.inner.lock().unwrap();
        let live = inner.live.as_ref()?;
        let start = live.start.as_ref();
        Some(PlaytestServer {
            id: live.id,
            title: live.title.clone(),
            phase: live.phase,
            rcon: live.rcon_connected,
            dir: live.dir.clone(),
            map: start.and_then(|s| s.map.clone()),
            procgen: matches!(live.target, Some(Target::Procgen)),
            identity_dir: start.map(|s| live.dir.join("server").join(&s.identity)),
        })
    }

    fn launch(&self, inner: &mut Inner, target: Target, prefab: bool, ctx: &StartContext) -> Result<Live, String> {
        if !ctx.server_dir.join(SERVER_EXECUTABLE).is_file() {
            return Err(t!("error.notInstalled"));
        }
        if let Target::Map(map) = &target {
            if !map.is_file() {
                return Err(t!("error.mapMissing", path = map.display()));
            }
        }
        let layout = Layout { base: &ctx.base, server_dir: &ctx.server_dir, steamcmd_dir: &ctx.steamcmd_dir };
        let (start, rewritten) =
            startfile::ensure(&layout, &target, prefab).map_err(|e| t!("error.startFile", error = e))?;
        if let Some(problem) = port_problem(start.server_port, start.rcon_port) {
            return Err(problem);
        }
        if start.made_by_droplaunch() {
            if let Err(e) = owner::apply(&ctx.server_dir, &start.identity, ctx.owner.as_ref()) {
                inner.push(t!("job.ownerFailed", error = e), "warn");
            }
        }
        // The whitelist (the plugin that checks it goes in before the server loads its plugins).
        if let Err(e) = whitelist::write(&ctx.server_dir, ctx.whitelist.as_deref()) {
            inner.push(t!("serverlog.whitelistFailed", error = e), "warn");
        }
        if let Some(ids) = &ctx.whitelist {
            match playtest::install_plugin(&ctx.server_dir) {
                Ok(Some(_)) => inner.push(if ids.len() == 1 { t!("serverlog.whitelist.one") } else { t!("serverlog.whitelist.other", n = ids.len()) }, "app"),
                Ok(None) => inner.push(t!("serverlog.whitelistNoFramework"), "warn"),
                Err(e) => inner.push(t!("serverlog.whitelistFailed", error = e), "warn"),
            }
        }
        if let Some(log) = &start.log_file {
            match std::fs::remove_file(log) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(t!("error.logInUse", path = log.display())),
            }
        }
        // An older copy in the game's map cache could be loaded instead of this file.
        if let (Target::Map(map), true) = (&target, ctx.clean_client_maps) {
            let cleaned = client::clean_copies(map, ctx.map_backups.as_deref());
            if cleaned.removed > 0 {
                let (copies, title) = (cleaned.describe(), target.title());
                inner.push(if ctx.map_backups.is_some() { t!("serverlog.cacheBackedUp", copies, title) } else { t!("serverlog.cacheRemoved", copies, title) }, "app");
            }
            if cleaned.in_use > 0 {
                let (n, title) = (cleaned.in_use, target.title());
                inner.push(if n == 1 { t!("serverlog.cacheInUse.one", title) } else { t!("serverlog.cacheInUse.other", n, title) }, "warn");
            }
        }
        let launched = platform::launch_console(&ctx.server_dir, &start.file_name(), &[(LAUNCH_ENV, "1")])
            .map_err(|e| t!("error.startFailed", file = start.file_name(), error = e))?;

        inner.next_id += 1;
        let title = target.title();
        inner.push(
            if rewritten { t!("serverlog.created", file = start.file_name(), title) } else { t!("serverlog.started", file = start.file_name(), title) },
            "app",
        );
        Ok(Live {
            id: inner.next_id,
            title,
            prefab: startfile::is_prefab_file(&ctx.server_dir, &target, &start.path),
            target: Some(target),
            tail: start.log_file.clone().map(LogTail::new),
            start: Some(start),
            process: None,
            shell_pid: Some(launched.shell_pid),
            job: Some(launched.job),
            spawn_deadline: Instant::now() + SPAWN_TIMEOUT,
            phase: Phase::Booting,
            boot: Tracker::default(),
            rcon: None,
            rcon_connected: false,
            info: None,
            last_info: None,
            started_at: now_ms(),
            ready_at: None,
            stop_requested: false,
            stop_deadline: None,
            attached: false,
            recent: VecDeque::new(),
            message: None,
            branch: manifest::read_installed(&ctx.server_dir).map(|i| i.branch),
            dir: ctx.server_dir.clone(),
        })
    }

    fn tick(&self) {
        // A look first: what this round needs from the settings and the disk.
        let scan = {
            let mut inner = self.inner.lock().unwrap();
            if inner.shutting_down {
                return;
            }
            let idle = inner.live.is_none() && inner.pending.is_none();
            let scan = idle && inner.last_scan.is_none_or(|t| t.elapsed() >= SCAN_EVERY);
            if scan {
                inner.last_scan = Some(Instant::now());
            }
            scan
        };
        // No disk access every round: the server folders are only listed for the scan.
        let (settings, auto_join, scan_dirs) = {
            let store = self.store.lock().unwrap();
            let scan_dirs = scan.then(|| {
                let mut dirs: Vec<PathBuf> = store.installs().into_iter().map(|(dir, _)| dir).collect();
                if dirs.is_empty() {
                    dirs.push(store.server_dir());
                }
                dirs
            });
            (StartContext::with_dir(&store, PathBuf::new()), store.config.auto_join, scan_dirs)
        };
        // The slow part outside the lock: reading back the log of a server that was already running.
        let found = scan_dirs.and_then(|dirs| find_running(&dirs));

        let mut inner = self.inner.lock().unwrap();
        if inner.shutting_down {
            return;
        }
        if inner.live.is_some() {
            self.tick_live(&mut inner, auto_join, &settings);
        } else if let Some(Pending { target, prefab, ctx }) = inner.pending.take() {
            match self.launch(&mut inner, target, prefab, &ctx) {
                Ok(live) => {
                    inner.live = Some(live);
                    inner.last = None;
                }
                Err(e) => {
                    inner.push(e.clone(), "error");
                    if let Some(last) = inner.last.as_mut() {
                        last.message = Some(e);
                        last.restarting = false;
                    }
                }
            }
            inner.dirty = true;
        } else if let Some(Found { mut live, pid, lines, reports }) = found {
            inner.reports.extend(reports);
            for line in lines {
                let (kind, source) = (level_kind(&line), live.boot.source(&line));
                inner.push_from(line, kind, source);
            }
            inner.push(t!("serverlog.found", title = live.title, pid), "app");
            inner.next_id += 1;
            live.id = inner.next_id;
            inner.live = Some(live);
            inner.last = None;
            inner.dirty = true;
        }

        let state = std::mem::take(&mut inner.dirty).then(|| inner.view());
        let lines = std::mem::take(&mut inner.outbox);
        drop(inner);
        if let Some(app) = self.app.get() {
            if !lines.is_empty() {
                let _ = app.emit("server", ServerEvent::Lines { lines });
            }
            if let Some(server) = state {
                let _ = app.emit("server", ServerEvent::State { server });
            }
        }
    }

    /// `settings`: owner and folders for a restart the server asks for itself (its own folder).
    fn tick_live(&self, inner: &mut Inner, auto_join: bool, settings: &StartContext) {
        let live = inner.live.as_mut().expect("live server");
        // (text, kind, source)
        let mut out: Vec<(String, &'static str, &'static str)> = Vec::new();
        let mut dirty = false;

        // 1. Find RustDedicated.exe once the start file has started it.
        if live.process.is_none() {
            if let Some(shell) = live.shell_pid {
                if let Some(found) = platform::find_processes(SERVER_EXECUTABLE).into_iter().find(|p| p.parent == shell) {
                    live.process = ProcessHandle::open(found.pid);
                }
            }
        }

        // 2. Follow the log.
        if let Some(tail) = live.tail.as_mut() {
            for line in tail.read_lines() {
                if live.boot.feed(&line) {
                    dirty = true;
                }
                if boot::is_noise(&line) {
                    continue;
                }
                if let Some(json) = playtest::report_json(&line) {
                    out.push((playtest::describe(json), "ok", "droplaunch"));
                    inner.reports.push(json.to_string());
                    continue;
                }
                if let Some(json) = crate::kits::game_kit_json(&line) {
                    out.push((crate::kits::describe_game_kit(json), "ok", "droplaunch"));
                    inner.game_kits.push(json.to_string());
                    continue;
                }
                live.recent.push_back(line.clone());
                if live.recent.len() > 12 {
                    live.recent.pop_front();
                }
                let (kind, source) = (level_kind(&line), live.boot.source(&line));
                out.push((line, kind, source));
            }
        }
        if live.phase == Phase::Booting && live.boot.ready() {
            live.phase = Phase::Ready;
            live.ready_at = Some(now_ms());
            let connect = live.view(false).connect;
            out.push((t!("server.readyToast", title = live.title, connect), "ok", "droplaunch"));
            if auto_join && !live.attached {
                let port = live.start.as_ref().map_or(startfile::DEFAULT_PORT, |s| s.server_port);
                let branch = live.branch.clone().unwrap_or_else(|| "public".into());
                // Looking for Rust takes a moment: not while holding the lock.
                let me = self.clone();
                std::thread::spawn(move || {
                    let text = match client::plan_join(&branch, port, true) {
                        Ok(plan) => match plan.state {
                            "started" => t!("serverlog.join.started", game = plan.game),
                            "running" => t!("serverlog.join.running", game = plan.game, connect = plan.connect),
                            "wrong" => t!("serverlog.join.wrong", running = plan.running_game.unwrap_or("Rust"), branch, game = plan.game),
                            _ => t!("serverlog.join.missing", branch, game = plan.game),
                        },
                        Err(e) => e,
                    };
                    let mut inner = me.inner.lock().unwrap();
                    inner.push(text, "app");
                    inner.dirty = true;
                });
            }
            dirty = true;
        }

        // 3. RCON: connect as soon as the server opened it, then poll serverinfo while ready.
        if live.rcon.is_none() && live.boot.rcon_started && !live.stop_requested {
            if let Some(password) = live.start.as_ref().and_then(|s| s.rcon_password.clone()) {
                live.rcon = Some(rcon::Client::start(live.start.as_ref().map_or(startfile::DEFAULT_RCON_PORT, |s| s.rcon_port), password));
            }
        }
        if let Some(client) = &live.rcon {
            for event in client.poll() {
                match event {
                    rcon::Event::Connected => {
                        live.rcon_connected = true;
                        dirty = true;
                    }
                    rcon::Event::Disconnected(_) => {
                        live.rcon_connected = false;
                        dirty = true;
                    }
                    // The log already has whatever `quit` prints.
                    rcon::Event::Reply { id: QUIT_ID, .. } => {}
                    rcon::Event::Reply { id, text } if inner.waiting.contains_key(&id) => {
                        if let Some(reply) = inner.waiting.remove(&id) {
                            let _ = reply.send(text);
                        }
                    }
                    rcon::Event::Reply { id: INFO_ID, text } => {
                        if let Ok(info) = serde_json::from_str::<ServerInfo>(&text) {
                            live.info = Some(info);
                            dirty = true;
                        }
                    }
                    rcon::Event::Reply { text, .. } => {
                        let text = text.trim_end();
                        if !text.is_empty() {
                            out.extend(text.lines().map(|l| (l.to_string(), "reply", "rcon")));
                        }
                    }
                }
            }
            if live.phase == Phase::Ready && live.rcon_connected && live.last_info.is_none_or(|t| t.elapsed() >= INFO_EVERY) {
                live.last_info = Some(Instant::now());
                client.send(INFO_ID, "serverinfo");
            }
        }

        // 4. Gone?
        let gone = match &live.process {
            Some(process) => !process.is_running(),
            // Never saw RustDedicated: the start file ended early (map missing, ...) or it's late.
            None => {
                Instant::now() > live.spawn_deadline || live.job.as_ref().is_some_and(|j| j.active_processes() == 0)
            }
        };
        if gone {
            let code = live.process.as_ref().and_then(|p| p.exit_code());
            // "Restarting in 5 seconds": a plugin (e.g. the RustEdit updater) or the restart
            // command asked for a restart, which a server host is expected to do.
            let self_restart = live.boot.restart_announced && !live.stop_requested && live.process.is_some();
            let crashed = !live.stop_requested && live.phase != Phase::Ready && !self_restart;
            if live.stop_requested || live.process.is_none() {
                // Close the console window too (an older start file may wait for a key).
                live.kill();
            }
            live.phase = if crashed { Phase::Crashed } else { Phase::Stopped };
            live.rcon = None;
            live.rcon_connected = false;
            live.message = Some(if live.process.is_none() {
                t!("serverlog.noStart", file = live.start.as_ref().map_or_else(|| t!("serverlog.theStartFile"), |s| s.file_name()))
            } else if crashed {
                let last = live.recent.iter().rev().find(|l| !l.trim().is_empty()).cloned().unwrap_or_default();
                match code {
                    Some(code) => t!("serverlog.crashedCode", code, last),
                    None => t!("serverlog.crashed", last),
                }
            } else if live.stop_requested {
                t!("serverlog.stoppedByYou")
            } else if self_restart && live.target.is_some() {
                t!("serverlog.selfRestart")
            } else {
                t!("serverlog.stopped")
            });
            if self_restart && inner.pending.is_none() {
                if let Some(target) = live.target.clone() {
                    let ctx = StartContext { server_dir: live.dir.clone(), ..settings.clone() };
                    inner.pending = Some(Pending { target, prefab: live.prefab, ctx });
                }
            }
            let message = live.message.clone().unwrap_or_default();
            out.push((message, if crashed { "error" } else { "app" }, "droplaunch"));
            let mut view = live.view(inner.pending.is_some());
            view.ended_at = Some(now_ms());
            view.rcon = false;
            inner.last = Some(view);
            inner.live = None;
            inner.last_scan = Some(Instant::now()); // give the pending start the first go
            dirty = true;
        } else if let Some(deadline) = live.stop_deadline {
            if Instant::now() > deadline {
                live.stop_deadline = None;
                live.kill();
                out.push((t!("serverlog.forceEnd"), "warn", "droplaunch"));
            }
        }

        for (text, kind, source) in out {
            inner.push_from(text, kind, source);
        }
        if dirty {
            inner.dirty = true;
        }
    }

}

/// A RustDedicated.exe from one of these server folders (every branch has its own) that
/// DropLaunch didn't start (this session). Runs without the lock: a server that ran for a long
/// time has a long log to read back.
fn find_running(dirs: &[PathBuf]) -> Option<Found> {
    let servers = platform::find_processes(SERVER_EXECUTABLE);
    let (server_dir, pid) = dirs.iter().find_map(|dir| {
        let exe = dir.join(SERVER_EXECUTABLE);
        servers.iter().find(|p| startfile::same_path(&p.path, &exe)).map(|p| (dir.as_path(), p.pid))
    })?;
    let process = ProcessHandle::open(pid)?;
    let started = process.created();
    let start = identify_start_file(server_dir, started);

    // Catch up on everything it logged so far; the console gets the end of it.
    let mut boot = Tracker::default();
    let mut recent = VecDeque::new();
    let mut lines = VecDeque::new();
    let mut reports = Vec::new();
    let mut tail = start.as_ref().and_then(|s| s.log_file.clone()).map(LogTail::new);
    if let Some(tail) = tail.as_mut() {
        loop {
            let read = tail.read_lines();
            if read.is_empty() {
                break;
            }
            for line in read {
                boot.feed(&line);
                if boot::is_noise(&line) {
                    continue;
                }
                if let Some(json) = playtest::report_json(&line) {
                    reports.push(json.to_string());
                    lines.push_back(playtest::describe(json));
                    continue;
                }
                recent.push_back(line.clone());
                if recent.len() > 12 {
                    recent.pop_front();
                }
                lines.push_back(line);
                if lines.len() > KEEP_LINES / 2 {
                    lines.pop_front();
                }
            }
        }
    }
    let target = start.as_ref().map(|s| match &s.map {
        Some(map) => Target::Map(map.clone()),
        None => Target::Procgen,
    });
    let title = target.as_ref().map_or_else(|| t!("serverlog.rustServer"), Target::title);
    let prefab = target.as_ref().zip(start.as_ref()).is_some_and(|(t, s)| startfile::is_prefab_file(server_dir, t, &s.path));
    let ready = boot.ready() || start.as_ref().is_none_or(|s| s.log_file.is_none());
    let live = Live {
        id: 0,
        title,
        target,
        prefab,
        message: start.is_none().then(|| t!("serverlog.outside")),
        start,
        process: Some(process),
        shell_pid: None,
        job: None,
        spawn_deadline: Instant::now(),
        phase: if ready { Phase::Ready } else { Phase::Booting },
        boot,
        tail,
        rcon: None,
        rcon_connected: false,
        info: None,
        last_info: None,
        started_at: started.and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or_else(now_ms, |d| d.as_millis() as i64),
        ready_at: None,
        stop_requested: false,
        stop_deadline: None,
        attached: true,
        recent,
        branch: manifest::read_installed(server_dir).map(|i| i.branch),
        dir: server_dir.to_path_buf(),
    };
    Some(Found { live, pid, lines, reports })
}

fn level_kind(line: &str) -> &'static str {
    match boot::classify(line) {
        boot::Level::Log => "log",
        boot::Level::Warn => "warn",
        boot::Level::Error => "error",
        boot::Level::Ok => "ok",
    }
}

/// Ask a server to stop: `quit` over RCON when it's up (it saves first), otherwise end it.
/// Returns true when it was asked nicely.
fn request_stop(live: &mut Live, force: bool) -> bool {
    live.stop_requested = true;
    live.phase = Phase::Stopping;
    let graceful = !force && live.rcon_connected && live.boot.ready();
    match (&live.rcon, graceful) {
        (Some(client), true) => {
            client.send(QUIT_ID, "quit");
            live.stop_deadline = Some(Instant::now() + QUIT_TIMEOUT);
            true
        }
        _ => {
            live.kill();
            false
        }
    }
}

/// Which start file a running server came from: the one whose log it's writing.
fn identify_start_file(server_dir: &Path, started: Option<SystemTime>) -> Option<StartFile> {
    let since = started.map(|t| t - Duration::from_secs(10));
    startfile::list(server_dir)
        .into_iter()
        .filter_map(|sf| {
            let modified = std::fs::metadata(sf.log_file.as_ref()?).and_then(|m| m.modified()).ok()?;
            (since.is_none_or(|s| modified >= s)).then_some((modified, sf))
        })
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, sf)| sf)
}

/// Something already listens on the server's ports (another server, usually).
fn port_problem(server_port: u16, rcon_port: u16) -> Option<String> {
    let other = || {
        let mut servers = platform::find_processes(SERVER_EXECUTABLE);
        servers.retain(|p| !p.path.as_os_str().is_empty());
        servers.first().map(|p| format!(" {}", t!("serverlog.otherServer", path = p.path.display()))).unwrap_or_default()
    };
    // Windows' table of open ports; elsewhere (or when it can't be read) trying the port on this PC
    // only: opening it on every network would make Windows ask the firewall question for DropLaunch.
    let taken = |udp: bool, port: u16| {
        platform::port_in_use(udp, port).unwrap_or_else(|| {
            if udp {
                UdpSocket::bind(("127.0.0.1", port)).is_err()
            } else {
                TcpListener::bind(("127.0.0.1", port)).is_err()
            }
        })
    };
    if taken(true, server_port) {
        return Some(format!("{}{}", t!("error.portGame", port = server_port), other()));
    }
    if taken(false, rcon_port) {
        return Some(format!("{}{}", t!("error.portRcon", port = rcon_port), other()));
    }
    None
}

/// Reads what a log file gained since the last call.
struct LogTail {
    path: PathBuf,
    offset: u64,
    partial: Vec<u8>,
}

impl LogTail {
    fn new(path: PathBuf) -> Self {
        Self { path, offset: 0, partial: Vec::new() }
    }

    fn read_lines(&mut self) -> Vec<String> {
        let Ok(mut file) = File::open(&self.path) else { return Vec::new() };
        let len = file.metadata().map_or(0, |m| m.len());
        if len < self.offset {
            // Started over (a new run overwrote the file).
            self.offset = 0;
            self.partial.clear();
        }
        if len == self.offset || file.seek(SeekFrom::Start(self.offset)).is_err() {
            return Vec::new();
        }
        let mut buf = Vec::new();
        let Ok(n) = file.take(1024 * 1024).read_to_end(&mut buf) else { return Vec::new() };
        self.offset += n as u64;
        self.partial.extend_from_slice(&buf);
        // One pass, and what's left of an unfinished line moves to the front once (not after
        // every line: a long log is read back in megabytes).
        let mut lines = Vec::new();
        let mut start = 0;
        while let Some(pos) = self.partial[start..].iter().position(|&b| b == b'\n') {
            let text = String::from_utf8_lossy(&self.partial[start..start + pos]);
            lines.push(text.trim_end_matches('\r').to_string());
            start += pos + 1;
        }
        self.partial.drain(..start);
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_tail_reads_whole_lines_and_handles_restarts() {
        let path = std::env::temp_dir().join(format!("droplaunch-tail-{}.log", std::process::id()));
        std::fs::write(&path, "one\r\ntwo\r\nthr").unwrap();
        let mut tail = LogTail::new(path.clone());
        assert_eq!(tail.read_lines(), vec!["one", "two"]);
        std::fs::write(&path, "one\r\ntwo\r\nthree\r\n").unwrap();
        assert_eq!(tail.read_lines(), vec!["three"]);
        std::fs::write(&path, "new\n").unwrap(); // a new run truncates the file
        assert_eq!(tail.read_lines(), vec!["new"]);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn serverinfo_parses() {
        let json = r#"{"Hostname":"x","MaxPlayers":10,"Players":1,"Queued":0,"Joining":0,"EntityCount":31337,"Framerate":256.0,"Uptime":42}"#;
        let info: ServerInfo = serde_json::from_str(json).unwrap();
        assert_eq!((info.players, info.max_players, info.entity_count, info.uptime), (1, 10, 31337, 42));
    }
}
