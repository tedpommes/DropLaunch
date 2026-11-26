//! Long-running SteamCMD jobs, reported as a stream of events the UI (or a CLI) can render.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::hookscript;
use crate::mods::{self, Framework};
use crate::startfile::{self, Chosen, Layout, Target, Use};
use crate::sync::Locked;
use crate::steamcmd::appinfo::{self, Branch};
use crate::steamcmd::parser::{self, Signal};
use crate::steamcmd::runner::{self, Output};
use crate::steamcmd::command::UpdateCommand;
use crate::steamcmd::{install, manifest, CancelToken};
use crate::t;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum JobEvent {
    Phase { phase: Phase },
    Status { text: String },
    Progress { text: String, percent: f64, done: u64, total: u64 },
    Line { text: String, source: LineSource },
    /// One row of the setup checklist changed.
    Task { task: &'static str, state: TaskState, detail: String },
    /// The setup downloaded what it needs and waits for Next in the setup window (`hold`).
    Held,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TaskState {
    Active,
    Done,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Steamcmd,
    Login,
    Files,
    Done,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LineSource {
    Steamcmd,
    App,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobSummary {
    pub ok: bool,
    pub cancelled: bool,
    pub title: String,
    pub detail: String,
    /// Done, but something needs a look (Oxide couldn't be re-applied after an update).
    pub attention: bool,
    pub buildid: Option<String>,
    pub seconds: f64,
}

pub struct UpdateRequest {
    pub base_dir: PathBuf,
    /// DropLaunch's data folder (which hook scripts you OK'd).
    pub data_dir: PathBuf,
    pub steamcmd_dir: PathBuf,
    pub server_dir: PathBuf,
    pub branch: String,
    pub validate: bool,
    /// When the build changed: Carbon (or Oxide) to its newest version too.
    pub update_framework: bool,
    /// What SteamCMD runs (Settings → Servers).
    pub command: UpdateCommand,
}

impl UpdateRequest {
    fn layout(&self) -> Layout<'_> {
        Layout { base: &self.base_dir, server_dir: &self.server_dir, steamcmd_dir: &self.steamcmd_dir }
    }
}

const MAX_ATTEMPTS: u32 = 3;
const RETRY_PAUSE: Duration = Duration::from_secs(10);

/// Wait, unless the job is cancelled meanwhile (true: it was).
fn wait_or_cancel(time: Duration, cancel: &CancelToken) -> bool {
    let until = Instant::now() + time;
    while Instant::now() < until {
        if cancel.is_cancelled() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    cancel.is_cancelled()
}

enum Outcome {
    Ok { title: String, detail: String, attention: bool, buildid: Option<String> },
    Failed(String),
    Cancelled,
}

/// Turns SteamCMD output into UI events and remembers how the run ended.
struct Session<'a> {
    emit: &'a mut dyn FnMut(JobEvent),
    phase: Option<Phase>,
    last_status: String,
    after_login: String,
    success: Option<bool>,
    error: Option<String>,
}

impl<'a> Session<'a> {
    fn new(emit: &'a mut dyn FnMut(JobEvent), after_login: String) -> Self {
        Self { emit, phase: None, last_status: String::new(), after_login, success: None, error: None }
    }

    fn phase(&mut self, phase: Phase) {
        if self.phase != Some(phase) {
            self.phase = Some(phase);
            (self.emit)(JobEvent::Phase { phase });
        }
    }

    /// A line is often seen twice (while printing and when finished): only report changes.
    fn status(&mut self, text: impl Into<String>) {
        let text = text.into();
        if text != self.last_status {
            self.last_status = text.clone();
            (self.emit)(JobEvent::Status { text });
        }
    }

    fn progress(&mut self, text: impl Into<String>, percent: f64, done: u64, total: u64) {
        self.last_status.clear();
        (self.emit)(JobEvent::Progress { text: text.into(), percent, done, total });
    }

    fn line(&mut self, text: impl Into<String>, source: LineSource) {
        (self.emit)(JobEvent::Line { text: text.into(), source });
    }

    fn task(&mut self, task: &'static str, state: TaskState, detail: impl Into<String>) {
        self.last_status.clear();
        (self.emit)(JobEvent::Task { task, state, detail: detail.into() });
    }

    /// Mark a checklist row as failed with the reason from `outcome`.
    fn task_failed(&mut self, task: &'static str, outcome: &Outcome) {
        let detail = match outcome {
            Outcome::Cancelled => t!("job.cancelled"),
            Outcome::Failed(message) => message.clone(),
            Outcome::Ok { .. } => return,
        };
        self.task(task, TaskState::Failed, detail);
    }

    fn output(&mut self, out: Output) {
        match out {
            // SteamCMD echoes the +@ options we pass ("@NoPromptForPassword" = "1"): just noise.
            Output::Line(text) if text.starts_with("\"@") => {}
            Output::Line(text) => self.line(text, LineSource::Steamcmd),
            Output::Signal { signal, .. } => self.signal(signal),
        }
    }

    fn signal(&mut self, signal: Signal) {
        match signal {
            Signal::SelfUpdate { text, percent, bytes } => {
                self.phase(Phase::Steamcmd);
                match (percent, bytes) {
                    (Some(p), Some((done, total))) => self.progress(t!("job.steamcmd.updating"), p, done, total),
                    _ if text.starts_with("Checking for available update") => self.status(t!("job.steamcmd.checking")),
                    _ => self.status(t!("job.steamcmd.status", text = text.trim_end_matches('.'))),
                }
            }
            Signal::SteamApi | Signal::Connecting => {
                self.phase(Phase::Login);
                self.status(t!("job.connecting"));
            }
            Signal::Connected => {
                self.phase(Phase::Login);
                self.status(t!("job.signingIn"));
            }
            Signal::Waiting => {
                self.phase(Phase::Login);
                self.status(t!("job.waitingSteam"));
            }
            Signal::LoggedIn => {
                self.phase(Phase::Files);
                self.status(self.after_login.clone());
            }
            Signal::LoginFailed(reason) => self.error = Some(t!("job.loginFailed", reason)),
            Signal::Progress { label, percent, done, total, .. } => {
                self.phase(Phase::Files);
                self.progress(parser::friendly_label(&label), percent, done, total);
            }
            Signal::Success { up_to_date } => self.success = Some(up_to_date),
            Signal::Error(message) => self.error = Some(message),
        }
    }

    fn finish(&mut self, outcome: Outcome, started: Instant) -> JobSummary {
        let seconds = started.elapsed().as_secs_f64();
        match outcome {
            Outcome::Ok { title, detail, attention, buildid } => {
                self.phase(Phase::Done);
                self.line(format!("{title}: {detail}"), LineSource::App);
                JobSummary { ok: true, cancelled: false, title, detail, attention, buildid, seconds }
            }
            Outcome::Cancelled => {
                self.phase(Phase::Failed);
                self.line(t!("job.cancelledLine"), LineSource::App);
                let (title, detail) = (t!("job.cancelled"), t!("job.cancelled.detail"));
                JobSummary { ok: false, cancelled: true, title, detail, attention: false, buildid: None, seconds }
            }
            Outcome::Failed(message) => {
                self.phase(Phase::Failed);
                self.line(&message, LineSource::Error);
                let title = t!("job.failed");
                JobSummary { ok: false, cancelled: false, title, detail: message, attention: false, buildid: None, seconds }
            }
        }
    }
}

/// Download SteamCMD if it's missing and let it update itself once.
fn ensure_steamcmd(dir: &Path, cancel: &CancelToken, s: &mut Session) -> Result<(), Outcome> {
    if install::is_installed(dir) {
        install::tidy(dir);
        return Ok(());
    }
    s.phase(Phase::Steamcmd);
    s.line(t!("job.steamcmd.missing", dir = dir.display()), LineSource::App);
    s.status(t!("job.steamcmd.downloadingStatus"));
    let label = t!("job.steamcmd.downloading");
    let result = install::install(dir, cancel, &mut |done, total| {
        s.progress(label.as_str(), percent(done, total), done, total.unwrap_or(0));
    });
    if cancel.is_cancelled() {
        return Err(Outcome::Cancelled);
    }
    result.map_err(|e| Outcome::Failed(t!("job.steamcmd.installFailed", error = format!("{e:#}"))))?;

    s.line(t!("job.steamcmd.downloaded"), LineSource::App);
    s.status(t!("job.steamcmd.setup"));
    // The first run often exits with a non-zero code after updating; that's normal.
    let run = runner::run_in_pty(&install::executable(dir), &["+quit".to_string()], dir, cancel, &mut |o| s.output(o))
        .map_err(|e| Outcome::Failed(format!("{e:#}")))?;
    if run.cancelled {
        return Err(Outcome::Cancelled);
    }
    install::tidy(dir);
    s.line(t!("job.steamcmd.ready"), LineSource::App);
    Ok(())
}

/// Install, update, switch branch or verify the Rust dedicated server; afterwards Carbon or Oxide
/// is brought up to date when the build changed (if that's on), Oxide is put back if the update
/// replaced its files, and update_server.bat follows the branch (the SteamCMD folder's script is
/// written afterwards, from every server folder).
pub fn update_server(req: &UpdateRequest, cancel: &CancelToken, emit: &mut dyn FnMut(JobEvent)) -> JobSummary {
    let started = Instant::now();
    let mut s = Session::new(emit, t!("job.signedIn.check"));
    let before = manifest::read_installed(&req.server_dir).map(|i| i.buildid);
    let mut outcome = run_update(req, cancel, &mut s);
    if let Outcome::Ok { title, detail, attention, buildid } = &mut outcome {
        // A new Rust build usually needs a new Carbon or Oxide: Rust's update replaces Oxide's
        // patched files anyway, and an older Carbon can fail to load on a newer Rust.
        let changed = before.is_some() && *buildid != before;
        let framework = mods::detect(&req.server_dir);
        if changed && req.update_framework && framework != Framework::None {
            match update_framework(framework, &req.server_dir, cancel, &mut s) {
                Ok(()) => detail.push_str(&format!(" · {}", t!("job.framework.done", framework = framework.label()))),
                Err(_) if cancel.is_cancelled() => return s.finish(Outcome::Cancelled, started),
                Err(message) => {
                    *title = t!("job.framework.attention", title = title.clone(), framework = framework.label());
                    *attention = true;
                    detail.push_str(&format!(" · {}", t!("job.framework.failed", framework = framework.label(), message)));
                }
            }
        }
        match reapply_oxide_if_needed(&req.server_dir, cancel, &mut s) {
            Some(Ok(())) => detail.push_str(&format!(" · {}", t!("job.oxide.done"))),
            Some(Err(message)) => {
                *title = t!("job.oxide.attention", title = title.clone());
                *attention = true;
                detail.push_str(&format!(" · {}", t!("job.oxide.failed", message)));
            }
            None => {}
        }
        write_update_script(&req.layout(), &req.branch, req.update_framework, &req.command, &mut s);
        if let Some(problem) = run_after_update(&req.data_dir, &req.server_dir, &req.branch, &mut s) {
            detail.push_str(&format!(" · {problem}"));
        }
    }
    s.finish(outcome, started)
}

/// Your after_update.bat in the server folder, when there is one: its output in the log. What
/// went wrong with it, for the summary (None: fine, or there's none). Only as you OK'd it
/// (Settings › Advanced): a new or changed one isn't run.
fn run_after_update(data_dir: &Path, server_dir: &Path, branch: &str, s: &mut Session) -> Option<String> {
    let name = hookscript::AFTER_UPDATE;
    let not_ok = |s: &mut Session| {
        let problem = t!("hook.notOkUpdate", name);
        s.line(problem.clone(), LineSource::Error);
        Some(problem)
    };
    match hookscript::trust(data_dir, server_dir, name) {
        hookscript::Trust::None => return None,
        hookscript::Trust::Ok => {}
        _ => return not_ok(s),
    }
    s.line(t!("hook.running", name), LineSource::App);
    s.status(t!("hook.running", name));
    let env = [("DROPLAUNCH_BRANCH", branch.to_string())];
    let ran = match hookscript::run_checked(data_dir, server_dir, name, &env, hookscript::AFTER_UPDATE_TIMEOUT) {
        hookscript::Checked::Ran(ran) => ran,
        hookscript::Checked::Missing => return None,
        // (Changed just now.)
        hookscript::Checked::NotOk(_) => return not_ok(s),
    };
    match ran {
        Ok(ran) => {
            for line in &ran.lines {
                s.line(line.clone(), LineSource::Steamcmd);
            }
            let problem = ran.problem(name);
            s.line(problem.clone().unwrap_or_else(|| t!("hook.done", name)), if problem.is_some() { LineSource::Error } else { LineSource::App });
            problem
        }
        Err(e) => {
            let problem = t!("hook.cantRun", name, error = e);
            s.line(problem.clone(), LineSource::Error);
            Some(problem)
        }
    }
}

/// The RustEdit extension again after Carbon or Oxide was updated, when the server has it (a
/// download that fails only gets a line: the one there stays).
fn refresh_rustedit(server_dir: &Path, cancel: &CancelToken, s: &mut Session) {
    if !mods::has_rustedit(server_dir) {
        return;
    }
    s.status(t!("job.rustedit.status"));
    let label = t!("job.rustedit.downloading");
    let result = mods::refresh_rustedit(server_dir, cancel, &mut |done, total| {
        s.progress(label.as_str(), percent(done, total), done, total.unwrap_or(0));
    });
    match result {
        Ok(Some(_)) => s.line(t!("job.rustedit.refreshed"), LineSource::App),
        Ok(None) => {}
        Err(e) => s.line(t!("job.rustedit.refreshFailed", error = format!("{e:#}")), LineSource::Error),
    }
}

/// Carbon or Oxide's newest release over the installed one (settings and plugins stay).
fn update_framework(framework: Framework, server_dir: &Path, cancel: &CancelToken, s: &mut Session) -> Result<(), String> {
    let name = framework.label();
    s.line(t!("job.framework.updating", framework = name), LineSource::App);
    s.status(t!("job.framework.updating", framework = name));
    let label = t!("job.framework.downloading", framework = name);
    mods::install(framework, server_dir, cancel, &mut |done, total| {
        s.progress(label.as_str(), percent(done, total), done, total.unwrap_or(0));
    })
    .map_err(|e| format!("{e:#}"))?;
    s.line(t!("job.framework.done", framework = name), LineSource::App);
    refresh_rustedit(server_dir, cancel, s);
    Ok(())
}

/// Oxide patches Rust files that every update or verify puts back.
fn reapply_oxide_if_needed(server_dir: &Path, cancel: &CancelToken, s: &mut Session) -> Option<Result<(), String>> {
    if !mods::oxide_needs_reapply(server_dir) {
        return None;
    }
    s.line(t!("job.oxide.reapplying"), LineSource::App);
    Some(reapply_oxide_now(server_dir, cancel, s))
}

fn reapply_oxide_now(server_dir: &Path, cancel: &CancelToken, s: &mut Session) -> Result<(), String> {
    s.status(t!("files.status.oxide"));
    let label = t!("job.oxide.downloading");
    mods::install(Framework::Oxide, server_dir, cancel, &mut |done, total| {
        s.progress(label.as_str(), percent(done, total), done, total.unwrap_or(0));
    })
    .map_err(|e| format!("{e:#}"))?;
    if mods::oxide_needs_reapply(server_dir) {
        return Err(t!("job.oxide.notPatched"));
    }
    s.line(t!("files.oxideDone"), LineSource::App);
    refresh_rustedit(server_dir, cancel, s);
    Ok(())
}

/// The "Re-apply Oxide" button.
pub fn reapply_oxide(server_dir: &Path, cancel: &CancelToken, emit: &mut dyn FnMut(JobEvent)) -> JobSummary {
    let started = Instant::now();
    let mut s = Session::new(emit, String::new());
    s.phase(Phase::Files);
    let outcome = match reapply_oxide_now(server_dir, cancel, &mut s) {
        Ok(()) => Outcome::Ok { title: t!("job.oxide.done"), detail: t!("job.oxide.doneDetail"), attention: false, buildid: None },
        Err(_) if cancel.is_cancelled() => Outcome::Cancelled,
        Err(message) => Outcome::Failed(t!("job.oxide.failed", message)),
    };
    s.finish(outcome, started)
}

fn write_update_script(layout: &Layout, branch: &str, update_framework: bool, command: &UpdateCommand, s: &mut Session) {
    match startfile::write_update_script(layout, branch, update_framework, command) {
        Ok(path) => s.line(t!("job.wrote", file = path.display(), branch), LineSource::App),
        Err(e) => s.line(t!("job.writeFailed", file = startfile::UPDATE_SCRIPT, error = e), LineSource::Error),
    }
}

fn run_update(req: &UpdateRequest, cancel: &CancelToken, s: &mut Session) -> Outcome {
    if let Err(outcome) = ensure_steamcmd(&req.steamcmd_dir, cancel, s) {
        return outcome;
    }
    if let Err(e) = std::fs::create_dir_all(&req.server_dir) {
        return Outcome::Failed(t!("job.createFailed", dir = req.server_dir.display(), error = e));
    }

    let before = manifest::read_installed(&req.server_dir);
    let mut validate = req.validate;
    // Files an update has to patch can be gone: Carbon deletes three of Rust's own DLLs
    // (Microsoft.CodeAnalysis, its CSharp part and System.Collections.Immutable) whenever its
    // server starts, and an antivirus or a person can remove others. The update then stops with
    // Missing game files (state 0x426), and Steam marks the install Files Missing. Only from then
    // on does checking every file download them again: `validate` on the first run doesn't help
    // while an update is waiting (SteamCMD skips the check and plans to patch the missing file
    // anyway). So a files-missing failure always gets one more run, with validate.
    let mut missing_retried = false;
    let args = |validate: bool| req.command.args(&req.server_dir, &req.branch, validate);

    let (branch, dir) = (&req.branch, req.server_dir.display());
    s.line(
        match &before {
            _ if req.validate => t!("job.action.verify", branch, dir),
            Some(b) if b.branch != req.branch => t!("job.action.switch", branch, dir),
            Some(_) => t!("job.action.update", branch, dir),
            None => t!("job.action.install", branch, dir),
        },
        LineSource::App,
    );
    if !req.command.is_default() {
        s.line(t!("job.ownCommand", command = req.command.text()), LineSource::App);
    }
    let exe = install::executable(&req.steamcmd_dir);
    for attempt in 1..=MAX_ATTEMPTS {
        s.success = None;
        s.error = None;
        if attempt > 1 {
            s.line(t!("job.retrying", attempt, max = MAX_ATTEMPTS), LineSource::App);
            s.status(t!("job.retrying", attempt, max = MAX_ATTEMPTS));
            // A slow Steam often needs a moment more than a few seconds (Cancel works meanwhile).
            if wait_or_cancel(RETRY_PAUSE, cancel) {
                return Outcome::Cancelled;
            }
        }
        s.phase(Phase::Steamcmd);
        s.status(t!("files.status.starting"));
        let run = match runner::run_in_pty(&exe, &args(validate), &req.steamcmd_dir, cancel, &mut |o| s.output(o)) {
            Ok(run) => run,
            Err(e) => return Outcome::Failed(format!("{e:#}")),
        };
        if run.cancelled {
            return Outcome::Cancelled;
        }
        if run.timed_out && s.error.is_none() {
            s.error = Some(parser::STOPPED_RESPONDING.into());
        }

        // Ground truth is Steam's manifest, not the console text.
        let after = manifest::read_installed(&req.server_dir);
        if let Some(inst) = after.filter(|i| i.fully_installed && i.branch == req.branch) {
            if s.success.is_some() || (s.error.is_none() && run.exit_code == Some(0)) {
                let changed = before.as_ref().is_none_or(|b| b.buildid != inst.buildid || b.branch != inst.branch);
                let title = match () {
                    _ if req.validate => t!("job.done.verified"),
                    _ if before.is_none() => t!("job.done.installed"),
                    _ if changed => t!("job.done.updated"),
                    _ => t!("job.done.upToDate"),
                };
                let detail = t!("job.done.detail", build = inst.buildid, branch = inst.branch);
                return Outcome::Ok { title, detail, attention: false, buildid: Some(inst.buildid) };
            }
        }

        let message = s.error.clone().unwrap_or_else(|| match run.exit_code {
            Some(code) if code != 0 => t!("job.exitCode", code),
            _ => t!("job.notConfirmed"),
        });
        let missing = !missing_retried && parser::files_missing(&message);
        if attempt < MAX_ATTEMPTS && (missing || parser::is_transient(&message)) {
            if missing {
                validate = true;
                missing_retried = true;
                let carbon = mods::detect(&req.server_dir) == Framework::Carbon;
                s.line(if carbon { t!("job.filesMissing.carbon") } else { t!("job.filesMissing") }, LineSource::App);
            } else {
                s.line(parser::describe_error(&message), LineSource::Error);
            }
            continue;
        }
        return Outcome::Failed(parser::describe_error(&message));
    }
    unreachable!("the last attempt always returns")
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupOptions {
    pub branch: String,
    pub framework: Framework,
    pub rustedit: bool,
    pub start_server: bool,
    /// Start this map instead of the procedural one (a map was dropped before setup).
    #[serde(default)]
    pub start_map: Option<String>,
    /// The bug report plugin (playtest kit), only when ticked.
    #[serde(default)]
    pub playtest: bool,
    /// Carbon's profiler (Carbon only).
    #[serde(default)]
    pub profiler: bool,
    /// The dropped map's first start is in prefab test mode.
    #[serde(default)]
    pub prefab: bool,
    /// Only the owner (and the players added in Settings) can join.
    #[serde(default)]
    pub whitelist: bool,
    /// The setup window is open with Continue when done unticked: once the downloads are done,
    /// setup waits here until it's false (Next), so the options and the owner can still change
    /// and nothing is installed or started before that.
    #[serde(default)]
    pub hold: bool,
}

/// Starts a server through DropLaunch's server manager; returns a short note.
pub type Launcher = Box<dyn Fn(Target, bool) -> Result<String, String> + Send>;

/// What the setup does after the framework step (see `extras` below).
pub type Extras = Box<dyn Fn(&SetupOptions) -> Vec<String> + Send>;

pub struct SetupRequest {
    pub base_dir: PathBuf,
    /// DropLaunch's data folder (which hook scripts you OK'd).
    pub data_dir: PathBuf,
    pub steamcmd_dir: PathBuf,
    pub server_dir: PathBuf,
    pub maps_dir: PathBuf,
    /// The UI can still change the options (not the branch or the map) while SteamCMD downloads:
    /// each step reads what's chosen when it begins.
    pub options: Arc<Mutex<SetupOptions>>,
    /// The start file of yours the procedural map's takes its settings from.
    pub procgen_settings: Option<PathBuf>,
    /// The server owner as set right now (it can change during the download too).
    pub owner: Box<dyn Fn() -> Option<crate::owner::SteamUser> + Send>,
    /// Puts the bug report plugin and Carbon's profiler setting in place as chosen (after the
    /// framework step); lines for the log.
    pub extras: Extras,
    pub launcher: Launcher,
    /// For update_server.bat: Carbon or Oxide with every server update.
    pub update_framework: bool,
    /// What SteamCMD runs (Settings → Servers), for the download and update_server.bat.
    pub command: UpdateCommand,
}

/// The one-click "fix": SteamCMD, server files, optional Carbon/Oxide (+ RustEdit), a procedural
/// start file and (optionally) starting it. Steps that are already done are skipped.
pub fn setup_all(req: &SetupRequest, cancel: &CancelToken, emit: &mut dyn FnMut(JobEvent)) -> JobSummary {
    let started = Instant::now();
    let mut s = Session::new(emit, t!("job.signedIn.get"));
    let outcome = run_setup(req, cancel, &mut s);
    s.finish(outcome, started)
}

fn run_setup(req: &SetupRequest, cancel: &CancelToken, s: &mut Session) -> Outcome {
    let options = || req.options.locked().clone();
    let opts = &options();

    s.task("steamcmd", TaskState::Active, "");
    let had_steamcmd = install::is_installed(&req.steamcmd_dir);
    if let Err(outcome) = ensure_steamcmd(&req.steamcmd_dir, cancel, s) {
        s.task_failed("steamcmd", &outcome);
        return outcome;
    }
    s.task("steamcmd", TaskState::Done, if had_steamcmd { t!("job.task.alreadyInstalled") } else { t!("job.task.downloaded") });

    // The scripts come first: if the download fails or gets stuck, update_server.bat can do it
    // by hand (it also works without DropLaunch), and start_procgen_server.bat starts it after.
    let layout = Layout { base: &req.base_dir, server_dir: &req.server_dir, steamcmd_dir: &req.steamcmd_dir };
    let chosen = req.procgen_settings.as_deref().and_then(|path| Chosen::read(path, Use::Procgen));
    if std::fs::create_dir_all(&req.server_dir).is_ok() {
        let _ = startfile::ensure(&layout, &Target::Procgen, false, chosen.as_ref());
        write_update_script(&layout, &opts.branch, req.update_framework, &req.command, s);
    }

    s.task("server", TaskState::Active, "");
    let before = manifest::read_installed(&req.server_dir);
    let server_note = match before.filter(|b| b.fully_installed && b.branch == opts.branch) {
        Some(existing) => t!("job.task.serverHere", branch = existing.branch),
        None => {
            let update = UpdateRequest {
                base_dir: req.base_dir.clone(),
                data_dir: req.data_dir.clone(),
                steamcmd_dir: req.steamcmd_dir.clone(),
                server_dir: req.server_dir.clone(),
                branch: opts.branch.clone(),
                validate: manifest::read_installed(&req.server_dir).is_some(), // repair a half install
                update_framework: false, // setup installs it right after
                command: req.command.clone(),
            };
            match run_update(&update, cancel, s) {
                Outcome::Ok { .. } => t!("job.task.serverDownloaded", branch = opts.branch),
                failed => {
                    s.task_failed("server", &failed);
                    return failed;
                }
            }
        }
    };
    s.task("server", TaskState::Done, server_note);
    if wait_for_next(&req.options, cancel, s) {
        return Outcome::Cancelled;
    }
    let mut notes = Vec::new();
    let mut warnings = Vec::new();

    // Chosen now: the options could change while SteamCMD downloaded (and until Next).
    let opts = &options();
    let framework_ready = mods::detect(&req.server_dir) == opts.framework
        && !mods::oxide_needs_reapply(&req.server_dir)
        && (!opts.rustedit || mods::has_rustedit(&req.server_dir));
    if opts.framework == Framework::None {
        s.task("framework", TaskState::Skipped, t!("job.task.notSelected"));
    } else if framework_ready {
        let framework = if opts.rustedit { t!("setup.task.frameworkRustedit", framework = opts.framework.label()) } else { opts.framework.label() };
        let note = t!("job.task.frameworkHere", framework);
        s.task("framework", TaskState::Done, note.clone());
        notes.push(note);
    } else {
        s.task("framework", TaskState::Active, "");
        match install_framework(opts, &req.server_dir, cancel, s) {
            Ok(note) => {
                s.task("framework", TaskState::Done, note.clone());
                notes.push(note);
            }
            Err(_) if cancel.is_cancelled() => {
                s.task_failed("framework", &Outcome::Cancelled);
                return Outcome::Cancelled;
            }
            Err(e) => {
                // The server still works without it, so keep going.
                let message = format!("{e:#}");
                s.task("framework", TaskState::Failed, message.clone());
                s.line(t!("job.frameworkFailedLine", framework = opts.framework.label(), message = message.clone()), LineSource::Error);
                warnings.push(t!("job.frameworkFailed", framework = opts.framework.label(), message));
            }
        }
    }

    for line in (req.extras)(opts) {
        s.line(line, LineSource::App);
    }

    s.task("startfile", TaskState::Active, "");
    let scripts = startfile::ensure(&layout, &Target::Procgen, false, chosen.as_ref())
        .and_then(|(start, _)| Ok((start, startfile::write_update_script(&layout, &opts.branch, req.update_framework, &req.command)?)));
    match scripts {
        Ok((start, update)) => {
            if let Some(owner) = (req.owner)() {
                match crate::owner::apply(&req.server_dir, &start.identity, Some(&owner)) {
                    Ok(_) => {
                        s.line(t!("job.owner", name = owner.name), LineSource::App);
                        notes.push(t!("job.ownerNote"));
                    }
                    Err(e) => s.line(t!("job.ownerFailed", error = e), LineSource::Error),
                }
            }
            s.line(t!("job.startFile", path = start.path.display()), LineSource::App);
            s.line(t!("job.updateScript", path = update.display()), LineSource::App);
            s.task("startfile", TaskState::Done, format!("{} + {}", start.file_name(), startfile::UPDATE_SCRIPT));
        }
        Err(e) => {
            let message = t!("job.startFilesFailed", error = e);
            s.task("startfile", TaskState::Failed, message.clone());
            return Outcome::Failed(message);
        }
    }

    // Set up: now the maps folder is made (not on a first start, before anything is set up).
    let _ = std::fs::create_dir_all(&req.maps_dir);

    let opts = &options();
    if !opts.start_server {
        s.task("launch", TaskState::Skipped, t!("job.task.notSelected"));
    } else {
        s.task("launch", TaskState::Active, "");
        let target = match &opts.start_map {
            Some(map) => Target::Map(PathBuf::from(map)),
            None => Target::Procgen,
        };
        let prefab = opts.prefab && matches!(target, Target::Map(_));
        let title = target.title();
        match (req.launcher)(target, prefab) {
            Ok(note) => {
                s.line(t!("job.launching", title), LineSource::App);
                s.task("launch", TaskState::Done, note);
                notes.push(t!("job.launchNote", title));
            }
            Err(e) => {
                let message = t!("job.launchFailed", error = e);
                s.task("launch", TaskState::Failed, message.clone());
                warnings.push(message);
            }
        }
    }

    let title = if warnings.is_empty() { t!("job.ready") } else { t!("job.readyWarning") };
    notes.extend(warnings);
    let buildid = manifest::read_installed(&req.server_dir).map(|i| i.buildid);
    Outcome::Ok { title, detail: notes.join(" · "), attention: false, buildid }
}

/// The downloads are done: with `hold` on (the setup window is open, Continue when done
/// unticked), wait for Next before the framework, the owner and starting the server. True: it was
/// cancelled meanwhile.
fn wait_for_next(options: &Mutex<SetupOptions>, cancel: &CancelToken, s: &mut Session) -> bool {
    if !options.locked().hold {
        return false;
    }
    (s.emit)(JobEvent::Held);
    while options.locked().hold {
        if wait_or_cancel(Duration::from_millis(150), cancel) {
            return true;
        }
    }
    false
}

fn install_framework(opts: &SetupOptions, server_dir: &Path, cancel: &CancelToken, s: &mut Session) -> anyhow::Result<String> {
    let name = opts.framework.label();
    let label = t!("job.downloading", name = name.clone());
    s.status(t!("job.downloadingStatus", name = name.clone()));
    mods::install(opts.framework, server_dir, cancel, &mut |done, total| {
        s.progress(label.as_str(), percent(done, total), done, total.unwrap_or(0));
    })?;
    s.line(t!("job.frameworkInstalled", name = name.clone(), dir = server_dir.display()), LineSource::App);
    if !opts.rustedit {
        return Ok(t!("job.frameworkInstalledNote", name));
    }
    s.status(t!("job.rustedit.status"));
    let label = t!("job.rustedit.downloading");
    let path = mods::install_rustedit(server_dir, cancel, &mut |done, total| {
        s.progress(label.as_str(), percent(done, total), done, total.unwrap_or(0));
    })?;
    s.line(t!("job.rustedit.saved", path = path.display()), LineSource::App);
    Ok(t!("setup.task.frameworkRustedit", framework = name))
}

fn percent(done: u64, total: Option<u64>) -> f64 {
    total.filter(|&t| t > 0).map_or(0.0, |t| done as f64 * 100.0 / t as f64)
}

/// Ask Steam which branches exist (and their latest build ids).
pub fn fetch_branches(steamcmd_dir: &Path, cancel: &CancelToken, emit: &mut dyn FnMut(JobEvent)) -> Result<Vec<Branch>, String> {
    let mut s = Session::new(emit, t!("job.signedIn.branches"));
    match ensure_steamcmd(steamcmd_dir, cancel, &mut s) {
        Ok(()) => {}
        Err(Outcome::Cancelled) => return Err(t!("job.cancelled")),
        Err(Outcome::Failed(message)) => return Err(message),
        Err(Outcome::Ok { .. }) => {}
    }
    s.phase(Phase::Steamcmd);
    s.status(t!("files.status.branches"));

    let exe = install::executable(steamcmd_dir);
    let args = appinfo::app_info_args();
    let mut captured = Vec::new();
    let run = runner::run_in_pty(&exe, &args, steamcmd_dir, cancel, &mut |o| match o {
        // The 200+ line KeyValues dump is parsed, not shown.
        Output::Line(line) if appinfo::is_dump_line(&line) => captured.push(line),
        Output::Line(line) => {
            captured.push(line.clone());
            s.output(Output::Line(line));
        }
        other => s.output(other),
    })
    .map_err(|e| format!("{e:#}"))?;
    if run.cancelled {
        return Err(t!("job.cancelled"));
    }

    let mut branches = appinfo::parse_branches(&captured.join("\n")).filter(|b| !b.is_empty());
    if branches.is_none() {
        s.line(t!("job.branches.retry"), LineSource::App);
        let text = runner::run_piped(&exe, &args, steamcmd_dir, cancel, Duration::from_secs(180)).map_err(|e| format!("{e:#}"))?;
        branches = appinfo::parse_branches(&text).filter(|b| !b.is_empty());
    }
    match branches {
        Some(list) => {
            s.phase(Phase::Done);
            let names: Vec<&str> = list.iter().map(|b| b.name.as_str()).collect();
            s.line(t!("job.branches.list", names = names.join(", ")), LineSource::App);
            Ok(list)
        }
        None => {
            s.phase(Phase::Failed);
            Err(s.error.clone().map(|e| parser::describe_error(&e)).unwrap_or_else(|| t!("job.branches.none")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_retry_pause_ends_when_cancelled() {
        let cancel = CancelToken::default();
        let other = cancel.clone();
        let started = Instant::now();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            other.cancel();
        });
        assert!(wait_or_cancel(Duration::from_secs(10), &cancel));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(!wait_or_cancel(Duration::from_millis(50), &CancelToken::default()));
    }

    fn options(hold: bool) -> Arc<Mutex<SetupOptions>> {
        Arc::new(Mutex::new(SetupOptions {
            branch: "public".into(),
            framework: Framework::Carbon,
            rustedit: true,
            start_server: true,
            start_map: None,
            playtest: false,
            profiler: false,
            prefab: false,
            whitelist: false,
            hold,
        }))
    }

    /// Runs wait_for_next; the events it sent come back as their "kind".
    fn wait(options: &Mutex<SetupOptions>, cancel: &CancelToken) -> (bool, Vec<String>) {
        let mut kinds = Vec::new();
        let mut emit = |e: JobEvent| kinds.push(serde_json::to_value(&e).unwrap()["kind"].as_str().unwrap().to_string());
        let mut s = Session::new(&mut emit, String::new());
        let cancelled = wait_for_next(options, cancel, &mut s);
        (cancelled, kinds)
    }

    #[test]
    fn setup_waits_for_next_only_when_held() {
        // Not held (Continue when done ticked, the window hidden): straight on, nothing said.
        assert_eq!(wait(&options(false), &CancelToken::default()), (false, vec![]));

        // Held: it says so and waits until Next lets go.
        let held = options(true);
        let next = held.clone();
        let started = Instant::now();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            next.locked().hold = false;
        });
        assert_eq!(wait(&held, &CancelToken::default()), (false, vec!["held".to_string()]));
        assert!(started.elapsed() >= Duration::from_millis(300));

        // Cancelled while it waits.
        let cancel = CancelToken::default();
        let other = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            other.cancel();
        });
        assert_eq!(wait(&options(true), &cancel), (true, vec!["held".to_string()]));
    }
}
