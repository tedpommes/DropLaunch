# DropLaunch v2

Rust map testing made simple, rebuilt in **Rust + Tauri 2**: one portable exe (~12 MB), no installer,
no .NET, no extra runtimes (it uses the WebView2 that ships with Windows 10/11).

> **Status:** setup, SteamCMD, server files, mods, the map list (with a watched maps folder),
> drop-to-launch, server control, the game's map cache cleanup,
> Windows notifications and self-updates work. Next up: map previews and bug reports from testers.

## Using it

Put `DropLaunch.exe` in a **new folder**, or in the folder that holds your **SteamCMD** folder
(v1 layout). The exe is the only file DropLaunch needs; what it downloads goes next to it:

```
DropLaunch.exe
SteamCMD\                downloaded automatically
Server_Files\            the Rust dedicated server (~6 GB), the branch installed first
Server_Files_staging\    another branch, installed next to it (one folder per branch)
  update_server.bat      update the server without DropLaunch (re-applies Oxide)
  start_procgen_server.bat
  start_<map>.bat        normal server, one per map you started
  start_<map>_prefab.bat the same map in prefab test mode
  logs\droplaunch_*.log  the server logs DropLaunch follows
Maps\                    the active maps folder: your .map files (subfolders work too), watched
```

Settings live in the user's app data, `%LOCALAPPDATA%\DropLaunch\settings.json` (one entry per
folder DropLaunch ran from, so two copies in two folders keep their own folders and history). A
DropLaunch started from a folder it never ran from (new, moved, copied, or an exe sent to someone)
starts like a new DropLaunch: default settings, the welcome screen, and only the SteamCMD, server
and maps folders it finds next to itself (else the default ones there, which setup downloads
into). Nothing comes along from another folder's settings: not the owner, options, history or
folders, and Dropped assets only lists what was dropped on that folder. Kits, bug reports and added
languages are kept per Windows user (in the app data) and shared by DropLaunch folders on the
same PC; on another PC there are none.
Entries of folders that are gone are removed, but not while their drive (a USB stick, a network
share) isn't there.
Older versions kept `droplaunch.json` next to the exe: it's moved in on the first start and
removed. (`DROPLAUNCH_HOME` or `DROPLAUNCH_DATA` keep a dev build's settings in their own folder.)

A fresh DropLaunch first shows a welcome screen ("Rust server setup and custom map testing made
easy"): **Set up a server** with what it takes (download SteamCMD, download the Rust server files;
SteamCMD shows as there already when it's found), and "DropLaunch will create 3 folders" with the
count in green (hovering it names them: SteamCMD, Server_Files, Maps; only those not there yet). Run
from a drive's root, the Desktop, Downloads, Documents or the user's own folder (Windows' own
locations for them, OneDrive included), a yellow line suggests an empty folder first, since it
creates folders and downloads files next to itself. Then **Show me around** (the guide, below). **Set up** opens the setup window. There's no "not now" button: a click
next to the welcome (or Esc) asks **Skip setting up?**, saying DropLaunch needs SteamCMD and the
server files before any map can start (**Set up** carries on, **Skip for now** leaves the Maps page's
Setup needed bar). It shows at every start until setup is done (SteamCMD and the server files are
there), also after it was skipped or didn't finish; a `.map` dropped on the exe of a fresh install
skips it.

**Download sizes** say whether they fit: wherever DropLaunch shows one before downloading (the
welcome screen, the setup window's SteamCMD and server rows, adding a branch, the guide's tips),
it's green when its drive has room and red when it doesn't, then with how much is free ("about
6 GB, only 3.1 GB free"); the tooltip names the drive. The server files count as fitting with
6.5 GB free, SteamCMD with 256 MB.

If SteamCMD or the server files are missing, a **setup window** explains what's needed and
**Fix everything** does it in one go (a `.map` dropped on the exe of a fresh install starts it right
away, no questions, and then starts that map; the mod framework, RustEdit and owner options can
still be changed while the files download): SteamCMD, the server files (branch of your choice), Carbon or
Oxide plus the RustEdit extension, `start_procgen_server.bat` + `update_server.bat`, optionally
makes you the server owner, and optionally starts the server. A click on the SteamCMD or server
files folder picks another one (an existing SteamCMD or server is found there, an empty folder gets
the download). The options come in groups: branch, mod framework and **Make me the server
owner** (admin on every test server); then the RustEdit extension and **Add DropLaunch Plugin**
(the bug report plugin), **Whitelist the server** (on by default: only the owner, and the players
added in Settings, can join; it needs the owner and Carbon or Oxide, and says so when one is
missing) and **Back up maps before removing them from Rust's cache** (on by default); then
**Enable Carbon profiler** and **Run test map server** (the procedural map: if it works and a map
later doesn't, it's the map). When a dropped map is waiting (a `.map` dropped on a fresh install:
the title is then **Getting DropLaunch Ready**), that last option starts it, with **Start it in
prefab test mode** under it for the map's first start. Options that need a mod
framework (the RustEdit extension, the DropLaunch plugin) are
unticked and greyed out with **None**, and come back as they were with Carbon or Oxide; **Carbon's
profiler** shows with Carbon only and starts unticked: once on, Carbon tracks every call of Rust's
code, the plugins and extensions from the server's start (a little slower, and it colours
performance numbers), so it's only for when you want to record. The bug report plugin is only
added when its box is ticked. Before anything is
downloaded DropLaunch checks the drive: the server files (about 6 GB) need 6.5 GB free (minus what
a download that didn't finish left there). With less, the server row's size is red with how much
is free, and **Fix everything** says so and offers **Choose another folder** instead of starting.

**Server owner**: **Detect my Steam IDs** reads who plays on this PC from Steam's own files (the
registry for the Steam folder and the signed-in account, `config/loginusers.vdf` for the SteamID64
and name, `config/avatarcache` for their pictures; no Steam API, no network), only when pressed,
and shows the accounts as cards to pick the owner (admin) of every test server from. It adds
`ownerid <steamid> "<name>" "added by DropLaunch"` to `server/<identity>/cfg/users.cfg`, which Rust
reads at startup, so double-clicked start files have it too; a running server gets it over RCON. A
Steam64 ID or `steamcommunity.com/profiles/…` link can be typed instead. The box is never filled in
before that: an owner saved earlier is only named under it, and stays the owner while it's left
empty (untick it to remove the owner).

DropLaunch opens in a **simple view** at its smallest window size, with just the map list: start a
map from its row (play, or the flask for prefab test mode), by double-clicking it or from its
right-click menu. While a server runs, a card under the list shows its boot progress, Join, Restart
and Stop (while it boots Restart and Stop sit in the card's top line, between the mods and the
time, so the server's output starts right under the progress); drag the bar above it to make it
taller and it shows the server's output too.
A strip above the list only shows up when the server files need something (setup, an update with
**Update newest**, a repair, Oxide) or while SteamCMD works. **More options** widens the window (to 1240, or the screen's width) and
adds the tabs, the server panel, the console, the Server files page, mods, scripts and folders.

**Guide** (for people new to hosting servers and making maps): on with **Show me around** on the
first start's welcome screen (ticked by default), or **Show tips** in **Settings → Guide** (it turns it off again too;
the tab also shows how far Getting started is, and **Start over**).
- **Tips**: when a step comes up, a tip points at what it's about and says what it is, what to do
  and why: the setup window (the two downloads, the mod framework, the owner, Fix everything, the
  download running), the map list, starting a map, booting, Ready and Join, being on the server,
  Stop, a start that failed, More options, a server update. One at a time, once
  each (**Got it**, or doing what it's about); **Turn off tips** in every tip. A tip waits while a
  window covers what it points at, and follows it when things move.
- **Getting started** (top of the Maps page, the side column with more options): install the
  server, start a map, join it in Rust, send a test bug report. Steps tick
  themselves off when they happen (also with the guide off); **Show me** goes there and points at
  it. The chevron shows every step; **Start over** shows every tip and step again; the ✕ turns the
  guide off.
- Saved in the settings (`guide`: on, tips read, steps done).

### Maps page

- **The list**: three sections, each one folds away: **Built in** (the procedural map), the
  **Active folder** (every `.map` in the active maps folder and its subfolders, newest first or by
  name, with search) and the **History** of started maps (kept in the settings).
- **Watched folder** (like v1): DropLaunch watches the active maps folder, and only that one. Maps
  added, removed or renamed there show up right away, and when one is saved (a RustEdit save or
  export) or added, its row in **Active folder** is marked (**saved** / **new**) with **Start** (or
  **Restart with it** when that map is running) and opens into view. When DropLaunch is in the
  background the taskbar button flashes and a Windows notification does the same with one click,
  like v1 (its **Prefab test mode** button starts it in prefab test mode). Both can be switched off
  under the active maps folder in Folders. Maps DropLaunch unpacks from a `.zip` don't count as saves.
- **Start a server**: double-click a map, press its play button, or select it and press Start.
  **Start in prefab test mode** (the flask button) is the same map for unfinished maps.
  **Drop** `.map` files, `.zip` files with maps or folders on the window, or on `DropLaunch.exe`
  (it's single-instance: a drop on the exe goes to the open window). Dropped maps are used where
  they are, like v1, so a re-export is picked up on the next start; maps in a `.zip` are unpacked
  into `Maps\<zip name>\`. What else the zip holds is used too: a kit file is added to the kits, a
  DropLaunch language file is added (not switched to; Settings has it), and a start file (a .bat or
  .sh that runs RustDedicated) is put next to the map, where its settings go into the map's own
  start files (other files stay in the zip).
- **The game's map cache** (v1's client cleanup): Rust keeps every map it loaded from a server in
  `<Rust>\maps\` as `<name>_<number>.map` and never removes any, so a map saved again under the
  same name can be loaded from an old copy. Before a map starts, DropLaunch removes the game's
  copies of it (the console says how many; v1 tried this with the exact file name, which never
  matched; switched in Settings). **Rust Game Maps** on the Server files page shows the cache's size and clears all of it
  (Rust downloads or builds a map again when it needs it). Where Rust is comes from Steam's
  `libraryfolders.vdf` and `appmanifest_252490.acf`.
- **Start files**: the first start of a map writes two files, `Server_Files\start_<map>.bat` and
  `start_<map>_prefab.bat`, so every server also starts with a double-click, without DropLaunch.
  DropLaunch runs the one you picked. Each start loads the current map file into a fresh world (the
  server keeps its own copy of the map, which would otherwise go stale). DropLaunch reads the file
  back before starting it: your edits (ports, password, max players…) are kept, and it's only
  rewritten when the map moved or it came from an older DropLaunch. A start file that came with the
  map (next to it, like one from the same `.zip`: named after the map, or the only one there) adds
  its settings: every `+setting value` DropLaunch doesn't set itself (max players, decay, PvE…; not
  the map, name, ports, RCON or log, and only plain values) replaces DropLaunch's own or is added
  (`rem Also uses the settings of start.bat`); prefab test mode keeps its own. Newer than the map's
  start files, it has them rewritten once.
- **Prefab test mode** (v1's prefab start file): no events, cargo ship, boat paths, ocean patrols or
  terrain protection. Some maps hang for many minutes on ocean patrol paths without it. Both files
  share the server identity, so either one gives the same fresh world; each has its own log.
- **Server panel**: boot progress read live from the server's log, as a path (server started →
  game files → Carbon/Oxide → RCON connected → map → entities → checks → ready, in the console's
  colours), then players, fps, entities and uptime over WebRCON, **Join in Rust**, **Copy connect**,
  **Restart** (same start file) and **Stop** (`quit` over RCON so it saves, ending it only if
  needed). Another map while one runs: DropLaunch asks, stops it, starts the new one. A server that
  restarts itself (e.g. the RustEdit extension updating) is started again. Port 28015/28016 already
  taken: DropLaunch says so instead of starting a broken server.
- **The right Rust game**: a client only joins a server of its own build. Rust (app 252490) joins
  public and release servers (and `last-month`, with Rust set to that beta in Steam); staging
  and Facepunch's other test branches (aux01-03, debug) need **Rust - Staging Branch** (app
  700580, free for Rust owners, its own folder `RustStaging`). Both run as `RustClient.exe`; the
  folder tells them apart. The button says which one it opens (**Join in Rust** / **Join in Rust
  Staging**). When the other game is open, Join offers **Close it and start …** (straight into
  the server); when the right one isn't installed, **Install it in Steam** (`steam://install`);
  when Rust is set to another beta than the server (public vs last-month), a note says which to
  pick under Properties → Betas. **Join automatically** (Settings) starts the right game too, and only says
  so in the console when another Rust is open. The map cache cleanup covers both games' caches.
- **Join in Rust**: with Rust closed, DropLaunch starts it with `+connect 127.0.0.1:28015`
  (`steam://run`, no server query involved). With Rust open it asks: **Restart Rust and join**
  (closes Rust like its X button, waits until it's gone, starts it again straight into the server)
  or **Switch to Rust** (brought to the front, press F1 and paste). Rust only takes a server to
  join when it starts; `steam://connect` can't move a running Rust to a server on the same PC:
  Steam asks the server which game it runs first, and that fails (127.0.0.1 or the network
  address, game port 28015 or query port 28017) with "app id specified by server is invalid". The
  F1 command is copied every time, as the sure way in.
- **Simple view**: the server card's title line has the F1 command on the right
  (`F1 → client.connect 127.0.0.1:28015`). The splitter under the maps shares the height with the
  server card; made taller, the server card shows the server's output. Double-click the splitter
  to go back to the default.
- **Stuck boots**: Rust prints a world step only once it's done, so DropLaunch knows which step is
  running. When the ocean patrol paths take more than 30 s (normally a few seconds; some maps hang
  there for many minutes) the server shows as **Stuck?**: an amber banner with **Keep waiting** and
  a pulsing **Restart in prefab test mode** next to it (also in the simple view), an amber progress
  bar, a "stuck?" chip on the map (after **Keep waiting** the prefab test mode restart stays, left of
  Restart), a toast, and a flashing taskbar button when DropLaunch is in the background. Cargo ship
  or event errors and antihack terrain kicks in the log get a quieter note with the same offer,
  since prefab test mode turns those off. Other steps get a note after 3 minutes.
- **Bug reports (playtest)**: testers type `/bug <what's wrong>` in Rust's chat, and the report
  shows up in DropLaunch right away, pinned on a picture of the map where they stood. **Bug
  reports** on the server card opens the page (with how many are open):
  - **The map**: drawn from the .map file itself (terrain, biomes, water, roads, rails and
    monuments, like the in-game map; the procedural map once the server built it), with Rust's
    grid (the plugin measures where Rust puts it: 146 m squares from the corner), zoom (wheel,
    +/-) and drag, double-click for the whole map. Pins are red (open) or green (fixed); a
    selected report shows which way the tester looked. Testers on the server are live dots with
    their names (every 2 s, click one to teleport to them); **Where testers went** draws every spot
    they walked through this run. **Right-click** a spot on the map to teleport one tester or
    everyone there (the plugin puts them on the ground, a roof or the water there; several players
    land side by side).
  - **The list**: newest open reports first: what they wrote, who, the grid square, when, which way
    they looked and what at (the prefab or "terrain" their view ended on, and how far). **Teleport**
    takes you (the server owner, in the game) to the spot, facing the same way; **Fixed** /
    **Reopen**; delete. Reports are kept per map across its versions (`Map_v19.map` and
    `Map_v20.map` share one list, in `%LOCALAPPDATA%\DropLaunch\Playtest\<map>.json`) and say
    which version they were reported on and fixed in. A new report while DropLaunch runs: a toast,
    a Windows notification in the background, and the tester gets "bug #3 is on the host's list"
    in chat. **Export report** saves one web page with the map, the pins and the list.
  - How: DropLaunch puts a small plugin (**DropLaunch**, `DropLaunchPlaytest.cs`, in Carbon's or Oxide's plugins
    folder) in the server while **Bug reports on** is ticked; both frameworks load it by
    themselves, also while the server runs. It prints each report to the server's log
    (`DLBUG1 {json}`, which DropLaunch already reads, so reports logged while DropLaunch was closed
    are picked up when it finds the server again) and answers `droplaunch.players`,
    `droplaunch.tp`, `droplaunch.tpto`, `droplaunch.tell`, `droplaunch.items` and `droplaunch.givekit`
    (Kits, below) over RCON; those only work from the
    server's console and RCON, not for players. It needs Carbon or Oxide (the page says so otherwise).
    It's only added when chosen: in
    setup, with **Add DropLaunch Plugin** in the Mods card, or **Bug reports on** on its page
    (put straight into the server folder, not only at the next start). Bug reports are on exactly
    while the plugin file is in the server folder (the running server's, else the one in use):
    a plugin that's there, put in by hand or for the whitelist, turns them on, one that's gone turns
    them off. While the whitelist is on they can't be turned off (it uses the same plugin).
  - **Monument names**: the plugin also tells the names the in-game map shows
    (`droplaunch.monuments`, once per server run); they're kept with the map's reports (per
    version of the map) and drawn on the map (switch **Monument names**), on the playtest page and in
    the map viewer. The map file itself only has the monuments' prefab ids, so there are no names
    (and no dots) before the map ran with bug reports on.
- **Server performance** (more options): under the server's numbers, a live graph of the last 5
  minutes: **All** (every line on its own scale), FPS, network in and out, memory (the server
  process) or players (`serverinfo` every 2 s). Under each graph: which colour is which, with the
  latest values; the mouse shows the values at any moment.
- **Carbon's profiler** (Carbon only; **Enable Carbon profiler** in setup, or **Carbon's profiler** in the Mods card): switches
  on Carbon's built-in Mono profiler (`carbon/config.profiler.json`, read at server start; it then
  watches Rust's own code, every plugin and every extension, so the RustEdit extension too). With
  more options the server card gets **Record 30 s** / **Record 2 min** (`c.profile <seconds> -c -t`:
  calls with their timings) and **Stop and save**. Carbon takes a few seconds more than asked to
  finish; DropLaunch waits for that (`c.profilestatus`, asked quietly), saves it
  (`c.profiler.print`, which answers with the file) and shows **Show profile**. **Profiles** opens
  `carbon/profiles` (the `.cprf` files), **Profiler Panel** Carbon's viewer at
  carbonmod.gg/tools/profiler-panel (load a `.cprf` there).
  It starts folded to its title line; the arrow opens it (kept between starts).
- **Map pictures**: a map's icon in the list is a picture of it, drawn from the .map file in the
  background (once per version of the file, kept in `%LOCALAPPDATA%\DropLaunch\Thumbs`; the
  procedural map's once a server has built it). Click it for the big map: zoom, drag, the grid, and
  the bug reports kept for that map as pins. One picture at a time (a big map takes a few hundred
  MB while it's read), and only the parts of the file a picture uses are unpacked (heights, ground
  types, water, roads/rivers/rails: about 60% of it), so a 6K map's icon takes ~50 ms the first time.
- **Console**: the server's log as it's written, plus commands over RCON. Each line is tagged and
  lightly coloured by who wrote it: the Rust server, map loading, Carbon or Oxide (and their
  plugins), RustEdit, RCON or DropLaunch (SteamCMD's output likewise). The bar between the maps and
  the console sets how the space is shared; the console and the SteamCMD output go full screen
  (Esc to go back). In the simple view the server card shows the console and its command line
  once it's dragged taller.
  - **Autocomplete**: typing shows the matching commands with what they do (`se` → `server.seed`,
    `server.save`, …; `seed` finds `server.seed` too), and the commands typed before. Tab takes
    the first (or the one picked with ↑/↓), Enter sends, Esc closes the list; while typing the
    arguments the command's description stays above the line. The list comes from the installed
    server itself, nothing is kept up to date by hand: Rust's generated command table
    (ConsoleGen) keeps every command's name, full name and description as string literals in
    `RustDedicated_Data/Managed/Assembly-CSharp.dll`, which DropLaunch reads from the .NET
    metadata (the #US heap; about 2,400 commands and settings, ~15 ms, cached per build). Carbon's
    come from its command attributes (#Blob heap of `carbon/managed/Carbon.Common.dll` and its
    modules, ~150), Oxide's from its DLLs plus its well-known permission and plugin commands. So
    each branch and each framework gets its own list.
  - **History**: ↑/↓ go through the commands typed before (newest first), kept between sessions in
    `%LOCALAPPDATA%\DropLaunch\console-history.txt` (the last 200; a repeat moves to the top).
- **Resizing**: DropLaunch opens at 640 × 950 (its smallest width), tall enough for the setup window with every option ticked and for Settings, Rent a server without scrolling (smaller on a smaller screen, so it always fits); the
  cards use the whole window, full screen too
  (the side column grows with big windows until it's dragged). With more options, the bar between
  the main column and the side column sets the side's width (one width for both pages;
  double-click to reset). In the simple view the server card shows the server's output as soon as
  it's dragged taller than its own part (room for the command line first, the lines as it grows).
- **Server files page**: the Rust server files card starts folded (its line says the build,
  branch and size; the buttons, and what a running job does, stay) so SteamCMD's output gets the
  rest of the page; the chevron shows the tiles. The Mods & scripts, Rust Game Maps and Folders
  cards fold too (the chevron; remembered). The side column gives way there when it was dragged wide on the Maps page, so the
  server files' buttons stay on one line.
- **Room for what's shown**: when a card grows (an update's progress, a hint about the server)
  it takes the room it needs from the map list (or SteamCMD's output) instead of
  getting a scroll bar, also when the bars were dragged; it gives the room back when it shrinks.
  The map list itself scrolls.
- **Windows notifications** while DropLaunch is in the background: a map was saved (click: start
  it) and the server is ready (click: join it). Switch them off in Settings. DropLaunch registers its name and icon for them under
  `HKCU\Software\Classes\AppUserModelId\gg.droplaunch.desktop` (no installer or Start menu
  shortcut needed); the icon file goes to `%LOCALAPPDATA%\DropLaunch\`.
- Servers keep running when DropLaunch closes. When it opens again, or a start file is
  double-clicked, DropLaunch finds the running server and follows it again.

### Server files page

- **Detected build** (an available update shows in the same tile), **Update server files** (only
  when there's an update; **Repair server files** when needed), **Check for server updates**,
  **Verify files**, with live progress, and the SteamCMD output as it's printed.
- **Missing files**: Carbon deletes three of Rust's own DLLs (`Microsoft.CodeAnalysis.dll`, its
  CSharp part, `System.Collections.Immutable.dll`) every time its server starts (it says so in
  `carbon/logs/Carbon.Startup.log`). A plain update of the next Rust build then stops with
  "Missing game files" (`state is 0x426`), because it can't patch files that aren't there, and Steam
  marks the install *Files Missing*. Only from then on does checking every file (`validate`) get
  them back: with an update waiting, `validate` on the first run does nothing (SteamCMD skips the
  check and still plans to patch the missing file). So an update or repair that fails because files
  are missing always runs once more with `validate` (the console says why, naming Carbon when it's
  installed), which downloads them again; `update_server.bat` does the same when Steam's manifest
  doesn't say fully installed (StateFlags 4) after its first run.
- **Branches side by side**: every branch gets its own folder, so testing a map on public and on
  staging needs no download in between. The server folder holds the branch installed first;
  another branch goes next to it as `Server_Files_<branch>` (`branchDirs` in the settings remembers
  where each one is; folders named like that are found too). Picking a branch that's installed
  (the Branch dropdown here, or **Branch** in the Maps card, shown once two are installed) switches
  right away: servers start from that folder, with its own start files, mods, `update_server.bat`
  and server identity. Picking a new one asks, then opens the setup window as **Add the …
  branch** (same mod framework as the one in use, the download goes to its own folder). **Hide**
  keeps it going in the background (the Maps page shows the progress, **Show** brings the window
  back): the branch in use stays in use until the new one is installed, so its servers can be
  started meanwhile; cancelled or failed, nothing changes. **Installed branches** lists them with
  their build, whether an update is out, the folder and **Use**. Updates, repairs and the free
  space check are per branch, and only a server from that branch's own folder holds them up (one
  SteamCMD job at a time, and starting a server waits only while SteamCMD changes its folder). A
  server from any branch's folder is found and followed; the server card says which branch it
  runs. Restart (and a restart the server asks for itself) starts it again from its own folder;
  to try the map on the other branch, pick that branch and start the map.
- **Carbon / Oxide with server updates**: when an update brings a new Rust build, DropLaunch then
  installs the newest Carbon (or Oxide) over the old one (a new Rust build usually needs it; plugins,
  their configs and Carbon's profiler setting stay, since the release only holds Carbon's own
  files). The Maps page's update card asks with **Also update Carbon** next to **Update server
  files**; it's the same switch as **Settings → Servers → Update Carbon or Oxide with every server
  update**, on by default (`updateFramework`). Not for verify/repair or when the build didn't change.
- **Oxide**: an update or verify puts Rust's original files back, which switches Oxide off. DropLaunch
  re-applies it right after; the warning and **Re-apply** button cover updates done elsewhere, and
  starting a server with Oxide switched off asks first. `update_server.bat` does the same check
  (`findstr` for Oxide's patch, then `curl` + `tar`, both built into Windows).
- **Mods & scripts**, **Rust Game Maps** (the game's map cache, see above) and **Folders** (one
  **Server files · <branch>** row per installed branch, each with its own change and open buttons;
  moving the first branch's folder keeps the others where they are).

### Kits

**Give items** is a card under the server card (with more options; the simple view has no kits or
bug reports), and **Bug reports** one under that; all three fold to their title line with the
arrow on the right (kept between starts; the server card keeps Start, or Restart and Stop).
Give items holds just a kit to pick and **Give to** (or **Make a kit** when there are none): a
small window with the players on the server, each with **Give**, and **Give to all (N)** under
them (Esc or a click elsewhere closes it).
Its expand button takes it full screen (Back at the top left, or Esc, brings it back), laid out
for the window: the kits, the chosen one's contents and who gets it side by side when it's wide,
stacked when it isn't:
- **Your kits**, each with **Edit** and **Give** (the Give to window) between its name and the
  pictures of what's in it (as many as fit). A click picks a kit; clicking the picked kit's name
  renames it (Enter keeps it, Esc doesn't). The chosen one's contents like Rust shows an
  inventory: **Wear** (7 slots), **Belt** (6) and **Inventory** (24, up to its last used row), each
  item with its amount. Edit, duplicate, delete. **Basic Build Kit** comes with DropLaunch (added
  to the Kits folder once, `builtin-kits.txt` remembers it, so a deleted one stays gone).
- **Give to**: the players on the server (Rust's own `playerlist`, every 5 s), each with **Give**,
  and **Give to all (N)**.
- **New kit** / edit opens the editor in the same full screen: Rust's items by category (All,
  Weapons, Ammunition, Attire, Medical, Food, Tools, …) on the left; the search looks in the chosen
  category (each category says how many it finds, and **Show all N found** when the chosen one
  has none) and picking a category keeps the search. A click asks how many (×1, ×10, ×100 fill in
  the box for **Add to kit**; a full stack and **Stack (99999)**, for anything that stacks, go in
  right away) and puts it in its usual place (clothes in Wear, weapons, tools and medical on the Belt, the rest in the
  Inventory), or drag it onto a slot of the kit and it asks how many and goes right there (what
  was there moves to a free slot). The kit on the right: drag items between slots (only clothing
  and armour can be worn), click one to change its amount or remove it. **Save kit** puts it in
  the list. A slider next to the search sets the size of the item pictures (kept). **Back** (top
  left, or Esc) leaves the editor for the kits (asking first when something changed), then leaves
  full screen. Dragging uses pointer events: the window takes the system's drag and drop for
  dropped files (.map), which keeps the page's own HTML drag and drop from ever starting.

**/droplaunch kit** (or `/droplaunch kit Raid kit`) in Rust's chat saves what that player wears,
has in the belt and in the inventory as a kit (admins only, like the server owner; the plugin
prints `DLKITSAVE1 {json}` to the log, DropLaunch adds it to the Kits folder once, named "<player>'s
inventory" when no name was typed, picks it in Give items and says so).

How it's given, over RCON: with the DropLaunch plugin loaded, `droplaunch.givekit <steamid>
w:0:metal.facemask:1;b:0:rifle.ak:1;m:0:ammo.rifle:128` puts every item in its own slot as one
stack of the whole amount, like `inventory.giveto` (99999 wood is one stack, above Rust's stack
size; a taken slot or a full inventory falls back to Rust's own placement, dropping at their feet
when there's no room). Without it, Rust's
`inventory.giveto <steamid> <item> <amount>` per item (Rust picks the slots, so clothes land in the
inventory; the drawer says so). Each kit is a file of its own, `Kits/<name>.json` in
DropLaunch's data folder (`"droplaunchKit": 1`, the kit's id, name, wear, belt, main; the old
`kits.json` moves over once): send one to share it, and dropping a kit file on DropLaunch or its
exe adds it (the same id replaces that kit). The **Bug reports** card shows how reports stand
(live, off with **Turn on**, needs Carbon or Oxide), the open count and the newest three open
reports (a click opens the page on that report); its expand button and **Open bug reports** open
the page.

Rust's item list (1,134 items with names, categories and stack sizes) is built in
(`ui/kits/items.json`, read from a server with `droplaunch.items`); with the plugin loaded the
running server's own list is used and kept (`items.json` in the data folder), so new items show
after a Rust update. Item pictures come from a public Rust wiki the first time they're shown
(`wiki.rustclash.com`, `cdn.rusthelp.com` as backup; Facepunch's artwork) and are kept in
`ItemIcons/` in the data folder, so they work offline after; an item without a picture shows its
initials in its category's colour.

### Rent a server

**Settings → Rent a server** shows the hosting companies to rent a Rust server from, for testing
with friends on a server that's always online:
**BisectHosting** on top (featured, in its colour, **Rent a server**), then GameServerKings,
GameServers.com, GTX Gaming, Host Havoc, Hostinger, Shockbyte and ZAP-Hosting as logo cards (their
colour comes up on hover). A click opens the host in the browser. They're DropLaunch's affiliate
links, the same as v1's, and the tab says so. The logos are v1's (`ui/hosts/`, transparent
borders trimmed); the list is in `ui/rent.js`.

### Settings

The gear at the end of the top bar opens **Settings**: a page over the whole window that slides
down from the top (**Back** at the top left, or Esc, slides it away: every page over the window
is left the same way, like the bug reports page: Settings, Kits, the map viewer, Licenses). It
opens over everything, a full-screen card like Give items too, and its Esc closes only Settings. Tabs on the left (Servers, Notifications, Guide, Dropped assets, Language, then Rent a server and About), each in its own colour (just
their icons in a narrow window; up/down arrows move between them), and the tab's page on the right,
which scrolls: settings in a column in the middle, Rent a server across the whole width (all seven
hosts in one row when there's room). On/off settings are switches; rows that do something (open a folder, a window) end
in an icon saying what.
- **Language**: a tile per language (built in / next to DropLaunch.exe / added; a click switches),
  how to add one, **Languages folder** and **Save the English file…** (see below).
- **Servers**: **Join automatically when a server is ready** (was on the server card); **Before a
  map starts, remove Rust's old copies of it** (was on Rust Game Maps, whose note now says which
  way it's set); **Back up maps before removing them** (those copies, and Clear Rust's map cache,
  move to `ClientMapBackups\<date>\` in the data folder instead; **Map backups** opens it);
  **Update Carbon or Oxide with every server update** (see Server files). **Who can join**:
  **Whitelist** and its list: the owner (always), the players added (a Steam64 ID or profile link,
  each with a remove button). Rust has no whitelist of its own: DropLaunch writes
  `<carbon|oxide>/data/DropLaunch/whitelist.json` in every server folder and the DropLaunch plugin
  (put in while the whitelist is on, also without bug reports) reads it at every login
  (`CanUserLogin`), so changes count right away; anyone else is told the server is whitelisted.
  Off, or without an owner, everyone can join. **Bug reports**: **Clear bug reports…** (says how many
  are kept, over how many maps; asks first) puts every map's reports aside in
  `Playtest\Cleared\<date>\` in the data folder and their numbering starts at #1 again (Reset
  everything does the same). Reports are numbered per map (all versions of a map share them, the
  procedural map has its own), and a map with no reports left (all deleted) starts at #1 again.
- **Notifications**: offer to start maps saved in the maps folder, Windows notifications (both were
  under the maps folder in Folders).
- **Guide**: **Show tips**, the Getting started steps (which are done) and **Start over**.
- **Dropped assets**: everything dropped on DropLaunch or its exe, by kind: **Maps**, **Zips** (with
  the maps they held), **Language files**, **Start files** (the server folders' start_*.bat) and
  **Kits**, each with how many. What comes with DropLaunch has a **Built in** tag: the procedural map's start file, English and Basic Build Kit (also in Settings → Language and in Give items). Start files are listed by their settings: files that only start
  another map are one row ("Map start file · 3 maps: …"), different settings (port, players,
  prefab test mode, your own changes) make another; hover one for its files and settings. Click
  one to use it: a map (or a zip's, or a one-map start file's map) is selected in the map list, a language is switched to, a kit is picked in Give items; the chosen
  one is marked **In use** (for start files: the one Start uses, DropLaunch's normal file for the
  selected map on this branch; a prefab one starts with its play button, as does one whose map the
  list doesn't show, and an older start file of your own can only be shown). Row buttons: start a server with a map, show it in Explorer, take it
  off the list (the file stays). Kept in `dropped.json` in the data folder (newest first, 300);
  maps started from outside the Maps folder count as dropped too. The settings themselves stay
  where they were (Language, Give items, …).
- **Rent a server** (see above) and **About** (the version, look for a new DropLaunch when it
  starts, **Licenses**, and the settings file: **Settings file** shows it in Explorer) are a group of
  their own at the bottom. About ends with **Reset everything…** (red, asks first): DropLaunch as it was when it was
  downloaded. Every setting back to its default (options, owner, also out of the servers' users.cfg,
  whitelist, bug reports, language, guide, map history, and the window's own choices kept in the
  browser storage), the folders back to the defaults next to the exe and **nothing linked**
  (`linked: false`): no server, no maps folder, no start files, Start asks for setup, until setup
  has run again (it finds SteamCMD and the server files if they're still there, and links them).
  Nothing is deleted (a deleted Basic Build Kit comes back). The window reloads into the welcome
  screen.

Per-server things stay where they are (mods, the DropLaunch plugin, Carbon's profiler, the owner,
folders). With Windows' animation effects off the page still slides in, just quicker (nothing else
in it moves). In a narrow window with **More options**, the top bar's DropLaunch name makes room
(the logo stays), and with **Update DropLaunch** showing too the tabs keep only their icons.

### Languages

Every text DropLaunch shows is in one language file: `ui/lang/en.json`, built into the exe (nothing
extra is needed to start). That covers the window's labels, buttons, tips and messages, and the
backend's too (errors, setup and update lines, the server console's own lines, notifications);
change a text there and rebuild, and it changes everywhere. The format:

```json
{
  "language": { "code": "en", "name": "English", "about": "how to translate" },
  "strings": {
    "maps.title": "Maps",
    "maps.count.one": "{n} map in {dir}",
    "maps.count.other": "{n} maps in {dir}",
    "setup.rustedit.note": "(needed by maps that use <a href=\"rustedit\">RustEdit features</a>)"
  }
}
```

- Keys are grouped by where they show (`topbar.`, `setup.`, `maps.`, `server.`, `job.`, `error.`,
  …), with a blank line between groups.
- `{placeholders}` are filled in by DropLaunch; keys ending in `.one` / `.other` are counts (other
  languages can add their own plural forms: `.few`, `.many`, …).
- A few tags work: `<b>`, `<strong>`, `<em>`, `<code>`, `<br>` and `<a href="name">` (the address
  comes from DropLaunch, never from the file: rustedit, steamcmd, carbon, oxide). They're turned into
  elements one by one, never read as HTML, so a language file can't put anything else on the page.
- **Other languages**: a language file next to `DropLaunch.exe` is used by itself (unless another
  language was picked in Settings); one dropped on the exe or on the window is added (copied into
  `%LOCALAPPDATA%\DropLaunch\Languages\`, a newer file with the same code replaces it) and
  switched to. Settings lists them all (built in / next to DropLaunch.exe / added). A file only
  needs the lines it changes: the rest stays English. A file with code `en` next to the exe changes
  the English texts themselves, without a rebuild.
- Switching reloads the window (servers and the console keep going; not while SteamCMD works,
  since its progress would be lost). Numbers follow the language ("18,4 MB"); dates in English
  follow Windows' own format, in other languages that language's.
- **Settings → Save the English file…** writes the built-in file somewhere to start a translation.
- Not in the language file: start files and `update_server.bat` (scripts on disk; they stay
  English), what SteamCMD and the Rust server print, names (Rust, Steam, Carbon, Oxide, the hosts),
  and the bug reports' stored data.
- `src-tauri/tests/lang.rs` keeps the file and the code in step: every key the code uses (`t!` in
  Rust, `t()` / `tn()` / `rich()` in the UI, `data-t…` in index.html) must be in en.json, every
  placeholder a Rust message fills in must be in its text, and every line in the file must be used.

### Updates

DropLaunch looks for a new version on GitHub a few seconds after it starts (and every 6 hours while
it's open; Settings switches it off: `updateCheck`). What else goes online on its own is listed
under [Privacy](#privacy). A new version opens a window with its notes, **Later** and
**Update DropLaunch**; **Don't show this again** skips the window for that version
(`skippedUpdate`), and the **Update DropLaunch** button in the top bar opens it
again. Updating downloads it next to `DropLaunch.exe` with a progress bar, checks it, puts it in
place (the old one becomes `DropLaunch.exe.old` and is removed on the next start)
and starts it. A running server keeps running and the new DropLaunch finds it again. When the
folder isn't writable
(e.g. under Program Files) DropLaunch says so and links to the download page.

Every release has `latest.json` next to `DropLaunch.exe`: version, notes, size, the exe's BLAKE3
hash and an Ed25519 signature over version, size and hash. The public half of the release key is
built into DropLaunch (`updater.rs`), so a release changed on GitHub without the private key is
refused (the manifest at the check, the exe after the download). Releases are built from this
repository by GitHub Actions (`.github/workflows/build.yml`), code signed through SignPath, and
then given that update signature; only the maintainer holds the release key.

### Third-party notices

The libraries compiled into DropLaunch ask for their notices and license texts to go along with
it, so the exe carries them: **About** (the version tag in the top bar, or **Licenses** in the
footer) lists every one (238 now) with its version, license and the license files it ships,
searchable. The list is `ui/licenses.json`, made from the Windows build's dependencies:

```
cd src-tauri
cargo run --example notices
```

Run it whenever a dependency changes: the `third_party_notices_are_up_to_date` test fails until
it's done, and also when a library has no license text. A few packages leave their license files
out; `src-tauri/notices-extra/<name>/` has those from the projects' repositories (including
Microsoft's license for the WebView2 loader that's linked into the exe). Build-only libraries and
proc macros aren't in the list (they aren't in the exe). SteamCMD, the Rust server, Carbon, Oxide
and the RustEdit extension aren't part of DropLaunch: they're downloaded from their makers when
chosen, under their own terms, and are never bundled or mirrored (Carbon is GPL-3.0, the RustEdit
extension has no license).

## Code signing policy

Free code signing provided by [SignPath.io](https://signpath.io), certificate by
[SignPath Foundation](https://signpath.org).

- Committers and reviewers: [TedPommes](https://github.com/tedpommes)
- Approvers: [TedPommes](https://github.com/tedpommes)

Only `DropLaunch.exe` is signed, and only as built by this repository's GitHub Actions workflow
from its source. SteamCMD, the Rust server, Carbon, Oxide and the RustEdit extension aren't part
of DropLaunch: they're downloaded from their makers when chosen and aren't signed by this project.

## Privacy

DropLaunch has no accounts, no analytics and no telemetry. Its settings, maps, kits and bug
reports stay on your PC, and the Steam account it detects is read from Steam's files on this PC and
never sent anywhere. Its web requests carry nothing about you apart from what any connection does
(your IP address, and a user agent like `DropLaunch/<version>` that names the program).

It goes online on its own for two things:

- **Update check** (github.com, this repository's releases): a few seconds after it starts and
  every 6 hours while it's open. Settings switches it off.
- **The Rust server's branch list** (api.steamcmd.net, a public mirror of Steam's app info for the
  Rust server): once at every start, so the branch choice is there straight away.

And when you ask for it:

- **SteamCMD and the server files**: from Valve (steamcdn-a.akamaihd.net and Steam's servers).
- **Carbon, Oxide, the RustEdit extension**: from their GitHub pages, when chosen.
- **Item pictures in Kits**: from wiki.rustclash.com, or cdn.rusthelp.com when the first has
  none, kept in the `ItemIcons` folder so each is fetched once.
- **Links** (guides, the hosting companies on Rent a server) open in your browser only when
  clicked. The Rent a server links are affiliate links.

The Rust server DropLaunch starts is Facepunch's own server and goes online like any Rust server.

## License

DropLaunch is free software under the [GNU General Public License v3.0](LICENSE) or (at your
option) any later version. The libraries compiled into it keep their own licenses, listed in
**About** (see [Third-party notices](#third-party-notices)).

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
