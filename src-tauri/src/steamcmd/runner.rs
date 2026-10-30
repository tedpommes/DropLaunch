//! Running SteamCMD.
//!
//! With a plain pipe SteamCMD keeps its output buffered until it exits (measured: a 22 s
//! validate delivered nothing, then everything at once - even out of order). Inside a pseudo
//! console (ConPTY on Windows, a PTY on Linux) it believes it talks to a real terminal and
//! flushes every line immediately, so that is how long-running jobs are started here.

use crate::t;
use anyhow::{bail, Context, Result};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use super::parser::{self, Signal};
use super::vt::LineAssembler;
use super::CancelToken;

/// Wide virtual console so long lines (paths, progress) are never wrapped by the renderer.
const SCREEN_COLS: u16 = 400;
const SCREEN_ROWS: u16 = 50;
/// SteamCMD prints progress every ~2 s; this long without any output means it is stuck.
const IDLE_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// Until it has signed in, less: a slow Steam answers within a few minutes (2.5 measured, its
/// own timeouts are 2), and a sign-in that hangs is then tried again instead of waited on.
const LOGIN_IDLE_TIMEOUT: Duration = Duration::from_secs(4 * 60);
/// After the main process exits, how long to wait on quiet processes it left behind.
const LINGER_TIMEOUT: Duration = Duration::from_secs(45);
/// "Where is the cursor?" - portable-pty opens ConPTY with INHERIT_CURSOR, so conhost sends
/// this first and blocks until the terminal answers. We answer like a terminal would.
const CURSOR_QUERY: &[u8] = b"\x1b[6n";
const CURSOR_REPLY: &[u8] = b"\x1b[1;1R";

pub enum Output {
    Line(String),
    /// `partial` = recognised on a line that is still being printed.
    Signal { signal: Signal, partial: bool },
}

#[derive(Debug, Default)]
pub struct RunResult {
    pub exit_code: Option<u32>,
    pub cancelled: bool,
    pub timed_out: bool,
}

/// Raw PTY bytes -> log lines + signals. The unfinished line is checked as well, so
/// "Connecting anonymously to Steam Public..." fires seconds before its "OK" arrives.
pub struct StreamParser {
    vt: LineAssembler,
    last_partial: String,
}

impl Default for StreamParser {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamParser {
    pub fn new() -> Self {
        Self { vt: LineAssembler::new(SCREEN_ROWS), last_partial: String::new() }
    }

    pub fn push(&mut self, bytes: &[u8], on: &mut dyn FnMut(Output)) {
        let lines = self.vt.feed(bytes);
        if !lines.is_empty() {
            self.last_partial.clear();
        }
        for line in lines {
            emit_line(line, on);
        }
        let partial = self.vt.partial();
        if !partial.is_empty() && partial != self.last_partial {
            if let Some(signal) = parser::parse(&partial) {
                on(Output::Signal { signal, partial: true });
            }
            self.last_partial = partial;
        }
    }

    pub fn finish(&mut self, on: &mut dyn FnMut(Output)) {
        if let Some(line) = self.vt.finish() {
            emit_line(line, on);
        }
    }
}

fn emit_line(line: String, on: &mut dyn FnMut(Output)) {
    let signal = parser::parse(&line);
    on(Output::Line(line));
    if let Some(signal) = signal {
        on(Output::Signal { signal, partial: false });
    }
}

/// Run `program` inside a pseudo console and stream its output to `on` as it happens.
pub fn run_in_pty(
    program: &Path,
    args: &[String],
    cwd: &Path,
    cancel: &CancelToken,
    on: &mut dyn FnMut(Output),
) -> Result<RunResult> {
    let pair = native_pty_system()
        .openpty(PtySize { rows: SCREEN_ROWS, cols: SCREEN_COLS, pixel_width: 0, pixel_height: 0 })
        .context(t!("error.pty.create"))?;
    let mut cmd = CommandBuilder::new(program);
    cmd.args(args);
    cmd.cwd(cwd);
    let mut child = pair.slave.spawn_command(cmd).with_context(|| t!("error.startProgram", program = program.display()))?;
    drop(pair.slave);

    // SteamCMD may re-launch itself after a self-update; the job keeps track of the whole tree.
    #[cfg(windows)]
    let job = {
        let job = crate::platform::ProcessJob::new().context(t!("error.job"))?;
        if let Some(pid) = child.process_id() {
            let _ = job.assign_pid(pid);
        }
        job
    };

    let mut reader = pair.master.try_clone_reader().context(t!("error.pty.read"))?;
    let mut writer = pair.master.take_writer().context(t!("error.pty.write"))?;
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 16 * 1024];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });

    // Signed in yet? (Until then a shorter silence counts as stuck.)
    let signed_in = std::cell::Cell::new(false);
    let mut on = |out: Output| {
        if matches!(out, Output::Signal { signal: Signal::LoggedIn, .. }) {
            signed_in.set(true);
        }
        on(out);
    };
    let mut stream = StreamParser::new();
    let mut result = RunResult::default();
    let mut last_output = Instant::now();
    let mut exited_at: Option<Instant> = None;
    let mut query_tail: Vec<u8> = Vec::new();

    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(bytes) => {
                last_output = Instant::now();
                if contains_cursor_query(&mut query_tail, &bytes) {
                    let _ = writer.write_all(CURSOR_REPLY);
                    let _ = writer.flush();
                }
                stream.push(&bytes, &mut on);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break, // Unix: EOF once every process is gone
        }

        let stuck = last_output.elapsed() > if signed_in.get() { IDLE_TIMEOUT } else { LOGIN_IDLE_TIMEOUT };
        if (cancel.is_cancelled() && !result.cancelled) || (stuck && !result.timed_out) {
            result.cancelled |= cancel.is_cancelled();
            result.timed_out |= stuck;
            #[cfg(windows)]
            job.terminate();
            let _ = child.kill();
        }

        if exited_at.is_none() {
            if let Ok(Some(status)) = child.try_wait() {
                result.exit_code = Some(status.exit_code());
                exited_at = Some(Instant::now());
            }
        }
        if let Some(exited) = exited_at {
            #[cfg(windows)]
            {
                // ConPTY never reports EOF by itself, so finish when the job is empty.
                if job.active_processes() == 0 {
                    break;
                }
                if exited.elapsed() > LINGER_TIMEOUT && last_output.elapsed() > LINGER_TIMEOUT {
                    job.terminate();
                    break;
                }
            }
            #[cfg(not(windows))]
            if exited.elapsed() > LINGER_TIMEOUT && last_output.elapsed() > LINGER_TIMEOUT {
                break;
            }
        }
    }

    // Closing the pseudo console flushes its last frame and ends the reader with EOF.
    drop(writer);
    drop(pair.master);
    let deadline = Instant::now() + Duration::from_secs(3);
    while let Ok(bytes) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        stream.push(&bytes, &mut on);
    }
    stream.finish(&mut on);
    if result.exit_code.is_none() {
        if let Ok(Some(status)) = child.try_wait() {
            result.exit_code = Some(status.exit_code());
        }
    }
    Ok(result)
}

/// True if `bytes` (joined to the end of the previous chunk) contain a cursor position query.
fn contains_cursor_query(tail: &mut Vec<u8>, bytes: &[u8]) -> bool {
    tail.extend_from_slice(bytes);
    let found = tail.windows(CURSOR_QUERY.len()).any(|w| w == CURSOR_QUERY);
    let keep = CURSOR_QUERY.len() - 1;
    if found {
        tail.clear();
    } else if tail.len() > keep {
        tail.drain(..tail.len() - keep);
    }
    found
}

/// Run with ordinary pipes and return everything printed. Output only arrives at the end, so
/// this is just the fallback for queries where nothing has to be shown live.
pub fn run_piped(program: &Path, args: &[String], cwd: &Path, cancel: &CancelToken, timeout: Duration) -> Result<String> {
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(cwd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    crate::platform::hide_console(&mut cmd);
    let mut child = cmd.spawn().with_context(|| t!("error.startProgram", program = program.display()))?;
    let mut stdout = child.stdout.take().context(t!("error.noOutput"))?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let started = Instant::now();
    while child.try_wait()?.is_none() {
        if cancel.is_cancelled() || started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            bail!(if cancel.is_cancelled() { t!("error.cancelled") } else { t!("error.steamcmdTimeout") });
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let bytes = reader.join().unwrap_or_default();
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::contains_cursor_query;

    #[test]
    fn cursor_query_found_even_when_split() {
        let mut tail = Vec::new();
        assert!(!contains_cursor_query(&mut tail, b"hello \x1b["));
        assert!(contains_cursor_query(&mut tail, b"6n world"));
        assert!(!contains_cursor_query(&mut tail, b"\x1b[6"));
        assert!(contains_cursor_query(&mut tail, b"n"));
        assert!(!contains_cursor_query(&mut tail, b"no query here at all"));
    }
}
