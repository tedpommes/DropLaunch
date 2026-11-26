# DropLaunch: development

How to build DropLaunch from source, and how its parts fit together.

## How the live SteamCMD output works

SteamCMD keeps its output buffered while it writes into a pipe, so tools only ever saw it in
batches when it exited. DropLaunch runs SteamCMD inside a pseudo console instead (ConPTY on Windows,
a PTY on Linux), where it flushes every line immediately:

- `steamcmd/runner.rs`: starts SteamCMD in the pseudo console (`portable-pty`), answers conhost's
  cursor-position query, and keeps it in a Windows job object, so self-update relaunches are
  followed and Cancel / closing the app kills the whole process tree.
- `steamcmd/vt.rs`: ConPTY sends a *rendered screen*, not raw bytes; on bursts it paints lines with
  cursor jumps. This small screen model turns that back into exact lines.
- `steamcmd/parser.rs`: recognises the stages (self-update, login, `Update state (...) progress`,
  `Success!`, errors), including half-printed lines.
- `jobs.rs`: turns those into UI events, retries network hiccups, and checks Steam's own manifest
  instead of trusting console text.

## Building

Needs Rust (stable, MSVC on Windows). No Node.js: the UI is plain HTML/CSS/JS (ES modules) in `ui/`.

```
cd src-tauri
cargo build --release        # -> src-tauri/target/release/droplaunch.exe
cargo test                   # start files, map scan, boot stages, parser/screen model, SteamCMD replays,
                             # the server manager (start, restart, crash...) with stand-in processes
cargo clippy --all-targets   # no warnings now; GitHub's build runs it too (an error fails the build)
```

The Windows manifest (Common Controls 6, which Tauri's dialogs need) comes from the linker for
every exe, DropLaunch and the test exes alike (`build.rs`, `manifest()`); tauri-build is told
not to add its own, which only DropLaunch.exe would get. A test exe without it doesn't load as
soon as a test reaches the server manager (`STATUS_ENTRYPOINT_NOT_FOUND`).

The server manager's tests (`server/mod.rs`, `Harness`) run a hidden `ping` in a job in place of
a start file and its RustDedicated.exe (`Hooks`): no console window, ports, game or map cache
are touched, and Join automatically is off.

The window has a content security policy (`tauri.conf.json`, `app.security.csp`): scripts only
from `ui/` (plus the small theme script in `index.html`, whose hash Tauri adds when it builds),
pictures from `ui/`, `blob:` and `data:` (kit items, avatars, map icons), requests only to `ui/`
and Tauri's IPC, frames only from the `addon` address (add-on pages, `frame-src`), inline styles
allowed (`el()` sets style attributes; Tauri is told not to add hashes to `style-src`, which would
switch `'unsafe-inline'` off). Text reaches the page through `textContent` (`el({ text })`,
`rich()`), never as HTML. Something that loads from elsewhere needs its source added there first.

Add-ons (`addons.rs`, `ui/addons.js`, `addons/`): pages served over the `addon` custom address
(`http://addon.localhost/<id>/<file>`, registered in `lib.rs`) into sandboxed frames. Tauri counts
every registered address as the app's own, with every command, so the frames' missing origin is
the wall: the frame's `sandbox` (no `allow-same-origin`, checked by `tests/addon_frames.rs`), the
`sandbox` that starts every add-on page's own policy, and the main window refusing to show any
page but DropLaunch's (`on_navigation`, `own_page` in `lib.rs`). Calls come as messages to the
window and go to `addon_call`, which checks them against the add-on's `addon.json`. A new call
goes in `ADDON_CALLS` and `addon_permission` (a test checks both agree), and a new event in
`EVENTS` in `ui/addons.js`: add-ons see both lists (`droplaunch.has`). A failed call answers with
an `AddonError` code. After a Tauri or wry update, run the isolation checks in `ADDONS-PLAN.md`
again.

## Layout

```
ui/                    index.html, styles.css, app.js (+ i18n.js, theme.js, settings.js, kits.js, guide.js, maps.js, server.js, console.js, playtest.js, rent.js, mapcanvas.js, mapviewer.js, graph.js, game.js, updates.js, about.js, layout.js, menu.js, dropdown.js, addons.js, util.js), licenses.json (third-party notices, generated), hosts/ (hosting companies' logos), kits/items.json (Rust's item list), lang/en.json (every text: the built-in English language file), themes/ (the four themes that come with it)
src-tauri/src/
  lib.rs               app setup (single instance + opened files, dialog plugin, the addon address,
                       the window and what it may show, commands)
  commands.rs          what the UI can call (and add-ons, through addon_call)
  addons.rs            add-ons: addon.json, the Add-ons folder and linked folders, installing,
                       storage, serving their pages with their content policy
  advanced.rs          Settings › Advanced: own download links, log texts, start arguments, a
                       second watched folder
  configfiles.rs       the config files add-ons may list, read and write (configs:read/write):
                       server/<identity>/cfg, Carbon's or Oxide's own, plugins', modules'
  hookscript.rs        before_start.bat / after_update.bat in the server folder, run hidden, and
                       only as OK'd (fingerprints in the data folder's hook-scripts.json)
  lang.rs              texts: English built in (ui/lang/en.json), language files next to the exe or dropped on it, t!()
  jobs.rs              setup / install / update / verify / Oxide re-apply as event streams
  config.rs            settings in DropLaunch-data next to the exe (an entry per folder, droplaunch.json moved in), first-start folder detection, one server folder per branch
  appdata.rs           what older versions kept in the user's app data, moved into DropLaunch-data
  assets.rs            dropped assets (dropped.json): what was dropped on DropLaunch, by kind
  theme.rs             themes: the four built in, theme files dropped or saved
  log.rs               droplaunch.log (background errors and panics)
  sync.rs              locks that keep working after a panic
  http.rs              the HTTP client for downloads (Windows' own TLS)
  concmd.rs            the server's console commands for autocomplete (read from its .NET DLLs)
  startfile.rs         start_*.bat and update_server.bat: writing and reading them back
  maps.rs              map list, dropped files and zips
  watcher.rs           watches the active maps folder (new, removed and saved maps)
  kits.rs              kits: saved kits, giving them over RCON (plugin slots or inventory.giveto), item pictures
  playtest/            bug reports from Rust's chat: the server plugin (DropLaunchPlaytest.cs), reports
                       per map, testers' positions, teleport; mapview.rs draws a map from its .map file
  server/              running servers: launch, log-based boot stages and hints, RCON, Steam query,
                       stop/restart, re-attach
  mods.rs              Carbon / Oxide / RustEdit extension, Oxide patch check
  owner.rs             the Steam account on this PC, server owner in users.cfg
  whitelist.rs         who can join: whitelist.json for the DropLaunch plugin
  client.rs            the Rust game on this PC and its map cache
  toast.rs             Windows notifications
  updater.rs           self-update from GitHub releases (signed latest.json)
  archive.rs           downloads, .zip / .tar.gz
  platform.rs          OS bits (job objects, console windows, process lookup, free space, URLs); a
                       server's start file runs in its own console, created inside its job (no
                       suspended start + resume, which antivirus software reads as code injection)
  steamcmd/            runner, vt, parser, install, manifest, appinfo, vdf, command (your own
                       SteamCMD command for updates)
src-tauri/tests/       replays of real SteamCMD captures, third-party notices check, language file check,
                       add-on frames are sandboxed
addons/                the add-on SDK (_sdk/droplaunch.js and .css, served by DropLaunch),
                       README.md (how to make one) and pack_addon.py
src-tauri/examples/    release signing, notices (ui/licenses.json), SteamCMD live test, map info
src-tauri/notices-extra/  license files some packages leave out (for the notices)
```
