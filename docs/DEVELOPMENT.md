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
cargo test                   # start files, map scan, boot stages, parser/screen model, SteamCMD replays
```

## Layout

```
ui/                    index.html, styles.css, app.js (+ i18n.js, settings.js, kits.js, guide.js, maps.js, server.js, console.js, playtest.js, rent.js, mapcanvas.js, mapviewer.js, graph.js, game.js, updates.js, about.js, layout.js, menu.js, dropdown.js, util.js), licenses.json (third-party notices, generated), hosts/ (hosting companies' logos), kits/items.json (Rust's item list), lang/en.json (every text: the built-in English language file)
src-tauri/src/
  lib.rs               app setup (single instance + opened files, dialog plugin, commands)
  commands.rs          what the UI can call
  lang.rs              texts: English built in (ui/lang/en.json), language files next to the exe or dropped on it, t!()
  jobs.rs              setup / install / update / verify / Oxide re-apply as event streams
  config.rs            settings in the app data (one entry per folder, droplaunch.json moved in), first-start folder detection, one server folder per branch
  concmd.rs            the server's console commands for autocomplete (read from its .NET DLLs)
  startfile.rs         start_*.bat and update_server.bat: writing and reading them back
  maps.rs              map list, dropped files and zips
  watcher.rs           watches the active maps folder (new, removed and saved maps)
  kits.rs              kits: saved kits, giving them over RCON (plugin slots or inventory.giveto), item pictures
  playtest/            bug reports from Rust's chat: the server plugin (DropLaunchPlaytest.cs), reports
                       per map, testers' positions, teleport; mapview.rs draws a map from its .map file
  net.rs               this PC's network address
  server/              running servers: launch, log-based boot stages and hints, RCON, Steam query,
                       stop/restart, re-attach
  mods.rs              Carbon / Oxide / RustEdit extension, Oxide patch check
  owner.rs             the Steam account on this PC, server owner in users.cfg
  client.rs            the Rust game on this PC and its map cache
  toast.rs             Windows notifications
  updater.rs           self-update from GitHub releases (signed latest.json)
  archive.rs           downloads, .zip / .tar.gz
  platform.rs          OS bits (job objects, console windows, process lookup, free space, URLs); a
                       server's start file runs in its own console, created inside its job (no
                       suspended start + resume, which antivirus software reads as code injection)
  steamcmd/            runner, vt, parser, install, manifest, appinfo, vdf
src-tauri/tests/       replays of real SteamCMD captures, third-party notices check, language file check
src-tauri/examples/    release signing, notices (ui/licenses.json), SteamCMD live test, map info
src-tauri/notices-extra/  license files some packages leave out (for the notices)
```
