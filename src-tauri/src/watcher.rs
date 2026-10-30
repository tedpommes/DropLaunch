//! Watches the active maps folder, like v1 did: the map list follows maps that are added,
//! removed or renamed there, and a map that's saved (a RustEdit save, a re-export) is announced
//! so it can be started with one click. Only this folder is watched, not the folders other
//! maps in the history came from.

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use tauri::{AppHandle, Emitter};

use crate::{maps, startfile};
use crate::sync::Locked;

/// A save is announced once the file has been quiet this long (big maps are written in pieces).
const SETTLE: Duration = Duration::from_millis(1500);
/// Folder changes come in bursts too.
const LIST_SETTLE: Duration = Duration::from_millis(400);
const TICK: Duration = Duration::from_millis(250);
/// Maps DropLaunch writes itself (unpacked from a .zip) aren't announced as saves.
const QUIET_FOR: Duration = Duration::from_secs(10);
/// A folder that couldn't be watched (it was missing) is tried again this often, and a watched one
/// is checked this often (deleted, moved away or made again: the watch has to start over).
const RETRY: Duration = Duration::from_secs(5);

/// The "maps-changed" event.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapsChanged {
    /// The map that was saved, when that's what happened (otherwise the list just changed).
    pub saved: Option<SavedMap>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedMap {
    pub path: String,
    pub name: String,
    /// It wasn't in the folder before.
    pub new: bool,
}

type FsEvent = notify::Result<notify::Event>;

#[derive(Clone)]
pub struct MapWatcher {
    inner: Arc<Mutex<Inner>>,
    events: Sender<FsEvent>,
    receiver: Arc<Mutex<Option<Receiver<FsEvent>>>>,
}

#[derive(Default)]
struct Inner {
    dir: Option<PathBuf>,
    watcher: Option<RecommendedWatcher>,
    retry_at: Option<Instant>,
    /// When the watched folder was made: a folder made again under the same name is another one.
    folder_created: Option<SystemTime>,
    /// When the watched folder was last checked.
    checked_at: Option<Instant>,
    /// Size and modified time of every map, to tell a real save from other events.
    known: HashMap<PathBuf, (u64, Option<SystemTime>)>,
    /// Maps with recent events, and when the last one came.
    pending: HashMap<PathBuf, Instant>,
    list_changed: Option<Instant>,
    quiet: HashMap<PathBuf, Instant>,
}

impl Default for MapWatcher {
    fn default() -> Self {
        let (events, receiver) = mpsc::channel();
        Self { inner: Arc::default(), events, receiver: Arc::new(Mutex::new(Some(receiver))) }
    }
}

impl MapWatcher {
    /// Start delivering "maps-changed" events.
    pub fn run(&self, app: AppHandle) {
        let Some(receiver) = self.receiver.locked().take() else { return };
        let me = self.clone();
        std::thread::Builder::new()
            .name("maps-watcher".into())
            .spawn(move || {
                let mut last = Instant::now();
                loop {
                    let event = match receiver.recv_timeout(TICK) {
                        Ok(event) => Some(event),
                        Err(RecvTimeoutError::Timeout) => None,
                        Err(RecvTimeoutError::Disconnected) => return,
                    };
                    // A round that went wrong is in the log; the next one goes on as usual.
                    crate::sync::survive(|| {
                        match event {
                            Some(Ok(event)) => me.note(event),
                            // The folder went away (or similar): is it still there?
                            Some(Err(_)) => me.check_folder_soon(),
                            None => {}
                        }
                        if last.elapsed() >= TICK {
                            last = Instant::now();
                            for change in me.settle() {
                                let _ = app.emit("maps-changed", change);
                            }
                        }
                    });
                }
            })
            .expect("spawn maps watcher");
    }

    /// Watch `dir` (and its subfolders) from now on.
    pub fn watch(&self, dir: PathBuf) {
        let old = {
            let mut inner = self.inner.locked();
            if inner.dir.as_deref().is_some_and(|d| startfile::same_path(d, &dir)) && inner.watcher.is_some() {
                return;
            }
            inner.known = snapshot(&dir);
            inner.pending.clear();
            inner.dir = Some(dir);
            let old = inner.watcher.take();
            self.start(&mut inner);
            old
        };
        // Dropped outside the lock (it waits for its thread).
        drop(old);
    }

    /// Something happened to the folder itself: check it at the next round.
    fn check_folder_soon(&self) {
        let mut inner = self.inner.locked();
        inner.checked_at = None;
        inner.list_changed = Some(Instant::now());
    }

    /// Maps DropLaunch just wrote itself: not announced as saves.
    pub fn quiet(&self, paths: &[String]) {
        let mut inner = self.inner.locked();
        let now = Instant::now();
        for path in paths {
            inner.quiet.insert(PathBuf::from(path), now);
        }
    }

    fn start(&self, inner: &mut Inner) {
        let Some(dir) = inner.dir.clone() else { return };
        // Not made here: the maps folder comes with the setup. Until then it's looked for again.
        if !dir.is_dir() {
            inner.retry_at = Some(Instant::now() + RETRY);
            return;
        }
        let events = self.events.clone();
        let started = notify::recommended_watcher(move |event: FsEvent| {
            let _ = events.send(event);
        })
        .and_then(|mut watcher| watcher.watch(&dir, RecursiveMode::Recursive).map(|()| watcher));
        match started {
            Ok(watcher) => {
                inner.watcher = Some(watcher);
                inner.retry_at = None;
                inner.folder_created = folder_created(&dir);
                inner.checked_at = Some(Instant::now());
            }
            Err(_) => inner.retry_at = Some(Instant::now() + RETRY),
        }
    }

    fn note(&self, event: notify::Event) {
        if matches!(event.kind, EventKind::Access(_)) {
            return;
        }
        let mut inner = self.inner.locked();
        let now = Instant::now();
        for path in event.paths {
            if maps::is_map(&path) {
                inner.pending.insert(path, now);
            } else if path.extension().is_none() {
                // A folder added, removed or renamed, maybe with maps inside.
                inner.list_changed = Some(now);
            }
        }
    }

    /// Events that have settled down, ready for the UI.
    fn settle(&self) -> Vec<MapsChanged> {
        let mut inner = self.inner.locked();
        let now = Instant::now();
        let mut out = Vec::new();
        // The watch follows the folder it started on: one that was deleted or moved away (into the
        // Recycle Bin, say), or made again, needs a new watch.
        let mut gone = None;
        if inner.watcher.is_some() && inner.checked_at.is_none_or(|t| now.duration_since(t) >= RETRY) {
            inner.checked_at = Some(now);
            let created = inner.dir.as_deref().map(folder_created);
            if created != Some(inner.folder_created) || inner.dir.as_deref().is_some_and(|d| !d.is_dir()) {
                gone = inner.watcher.take();
                inner.pending.clear();
                inner.retry_at = Some(now);
                inner.list_changed.get_or_insert(now);
            }
        }
        if inner.retry_at.is_some_and(|t| now >= t) {
            self.start(&mut inner);
            // It's there (again): what's in it now isn't news, but the list may have changed.
            if inner.watcher.is_some() {
                if let Some(dir) = inner.dir.clone() {
                    inner.known = snapshot(&dir);
                }
                inner.list_changed = Some(now);
            }
        }
        inner.quiet.retain(|_, t| now.duration_since(*t) < QUIET_FOR);
        let ready: Vec<PathBuf> = inner.pending.iter().filter(|(_, t)| now.duration_since(**t) >= SETTLE).map(|(p, _)| p.clone()).collect();
        for path in ready {
            inner.pending.remove(&path);
            let Ok(meta) = std::fs::metadata(&path).map_err(drop).and_then(|m| if m.is_file() { Ok(m) } else { Err(()) }) else {
                // Removed (or renamed away).
                if inner.known.remove(&path).is_some() {
                    inner.list_changed.get_or_insert(now);
                }
                continue;
            };
            let stamp = (meta.len(), meta.modified().ok());
            let before = inner.known.insert(path.clone(), stamp);
            if before == Some(stamp) {
                continue; // nothing was really written (attributes, a virus scan...)
            }
            let quiet = inner.quiet.keys().any(|q| startfile::same_path(q, &path));
            if meta.len() == 0 || quiet {
                inner.list_changed.get_or_insert(now);
                continue;
            }
            out.push(MapsChanged {
                saved: Some(SavedMap { path: path.display().to_string(), name: startfile::map_title(&path), new: before.is_none() }),
            });
        }
        if !out.is_empty() {
            inner.list_changed = None; // the UI reloads the list for a save too
        } else if inner.list_changed.is_some_and(|t| now.duration_since(t) >= LIST_SETTLE) {
            inner.list_changed = None;
            out.push(MapsChanged { saved: None });
        }
        drop(inner);
        // Dropped outside the lock (it waits for its thread).
        drop(gone);
        out
    }
}

/// When a folder was made (None: unknown, or it isn't there).
fn folder_created(dir: &Path) -> Option<SystemTime> {
    std::fs::metadata(dir).ok().filter(std::fs::Metadata::is_dir).and_then(|m| m.created().ok())
}

fn snapshot(dir: &Path) -> HashMap<PathBuf, (u64, Option<SystemTime>)> {
    maps::find(dir)
        .into_iter()
        .filter_map(|path| {
            let meta = std::fs::metadata(&path).ok()?;
            Some((path, (meta.len(), meta.modified().ok())))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("droplaunch-watch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Wait (up to 6 s) for the watcher to report something.
    fn wait_for(watcher: &MapWatcher, receiver: &Receiver<FsEvent>) -> Vec<MapsChanged> {
        let deadline = Instant::now() + Duration::from_secs(6);
        while Instant::now() < deadline {
            while let Ok(Ok(event)) = receiver.try_recv() {
                watcher.note(event);
            }
            let out = watcher.settle();
            if !out.is_empty() {
                return out;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Vec::new()
    }

    #[test]
    fn announces_saves_but_not_untouched_or_quiet_maps() {
        let dir = temp_dir("saves");
        std::fs::create_dir_all(dir.join("Sub")).unwrap();
        std::fs::write(dir.join("old.map"), b"old").unwrap();
        let watcher = MapWatcher::default();
        let receiver = watcher.receiver.locked().take().unwrap();
        watcher.watch(dir.clone());

        // A new map in a subfolder.
        std::fs::write(dir.join("Sub").join("new.map"), b"brand new").unwrap();
        let out = wait_for(&watcher, &receiver);
        let saved = out.iter().find_map(|c| c.saved.clone()).expect("announced");
        assert_eq!((saved.name.as_str(), saved.new), ("new", true));

        // A save over an existing map.
        std::fs::write(dir.join("old.map"), b"saved again").unwrap();
        let saved = wait_for(&watcher, &receiver).into_iter().find_map(|c| c.saved).expect("announced");
        assert_eq!((saved.name.as_str(), saved.new), ("old", false));

        // Unpacked by DropLaunch: the list changes, no save notice.
        let unpacked = dir.join("Sub").join("unpacked.map");
        watcher.quiet(&[unpacked.display().to_string()]);
        std::fs::write(&unpacked, b"from a zip").unwrap();
        let out = wait_for(&watcher, &receiver);
        assert!(!out.is_empty() && out.iter().all(|c| c.saved.is_none()), "{out:?}");

        // Removed: the list changes.
        std::fs::remove_file(dir.join("old.map")).unwrap();
        let out = wait_for(&watcher, &receiver);
        assert!(!out.is_empty() && out.iter().all(|c| c.saved.is_none()), "{out:?}");

        // The folder deleted and made again: the watch starts over on the new one.
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        watcher.check_folder_soon();
        let out = wait_for(&watcher, &receiver);
        assert!(out.iter().all(|c| c.saved.is_none()), "{out:?}");
        std::fs::write(dir.join("again.map"), b"after").unwrap();
        let saved = wait_for(&watcher, &receiver).into_iter().find_map(|c| c.saved).expect("announced");
        assert_eq!((saved.name.as_str(), saved.new), ("again", true));
        drop(watcher);
        let _ = std::fs::remove_dir_all(dir);
    }
}
