# Making a DropLaunch add-on

An add-on is a small web page that DropLaunch shows as a **tab** (next to Maps and Server files) or as a **card** in More options. It talks to DropLaunch through a script, `droplaunch.js`. With it, an add-on can read the server's state and console, send console commands, give kits and keep its own settings. What it may do is listed in its `addon.json`, and DropLaunch shows that list before anyone adds it.

The SDK (`_sdk/`) and `pack_addon.py` are MIT-licensed: copy them, change them, ship them with anything.

**Your add-on, your license.** DropLaunch is GPL-3.0 with an [add-on exception](../LICENSE): an add-on that works through the add-on API described here can use any license, closed-source and paid included. Only code copied from DropLaunch itself (outside this folder) would bring the GPL along.

## Start in five minutes

1. Make a folder of your own, for example `my-addon`, with an [`addon.json`](#addonjson) and the `index.html` it names (start from [The page](#the-page)).
2. In DropLaunch: **Settings → Add-ons → Add a folder you're making**, and pick the folder.
   DropLaunch uses the folder where it is, so you can keep editing it.
3. After a change, press **Reload** on your add-on in Settings → Add-ons.
4. A call that fails rejects with an `Error` saying why. While you work, show those with `droplaunch.ui.toast(err.message, "error")`, and catch errors in your own code the same way: DropLaunch has no developer tools to show them. For a folder you're making, DropLaunch also shows your page's uncaught script errors as toasts (the SDK passes them on), and a page that hasn't loaded `droplaunch.js` 5 seconds after it loaded gets a "didn't start" note with **Reload** in its place. Load your own scripts with `crossorigin` (`<script src="main.js" crossorigin></script>`) or as `type="module"`: the page has no origin of its own, so without it the browser only says "Script error." instead of the message.

## addon.json

```json
{
  "id": "my-addon",
  "name": "My add-on",
  "version": "0.1.0",
  "author": "You",
  "description": "What it does, in a sentence or two.",
  "homepage": "https://github.com/you/your-addon",
  "droplaunch": ">=2.1.0",
  "api": 1,
  "permissions": ["server:read", "console:read", "rcon", "kits"],
  "ui": [{ "kind": "tab", "title": "My tab", "page": "index.html", "icon": "terminal" }],
  "network": [],
  "links": ["steamcommunity.com"]
}
```

| Field | |
|---|---|
| `id` | 2 to 40 small letters, numbers and dashes. An add-on with the same id replaces the old one (an update); what it saved stays. |
| `name`, `version` | Shown in the add dialog and in Settings. Name up to 60 characters. |
| `author`, `description`, `homepage` | Optional. The homepage must be `https://`. |
| `droplaunch` | The oldest DropLaunch it works with, like `">=2.1.0"`. Older DropLaunch versions refuse it. |
| `api` | `1`. |
| `permissions` | What it may do (below). Ask only for what you use: people see the list. |
| `ui` | Up to 4 pages. `kind`: `"tab"` or `"card"`. `title`: up to 40 characters. `page`: an .html file in the add-on's folder. `icon`: one of the icons below. `height`: a card's height in pixels (80 to 900, 260 when left out). |
| `network` | Websites the page may connect to, like `"https://api.example.com"` or `"wss://example.com:8080"` (up to 20). Needs the `network` permission. |
| `links` | Websites `droplaunch.ui.openLink` opens without asking first, like `"steamcommunity.com"` or `"*.example.com"`. People see this list in the add dialog and in Settings. Any other link is asked about. |

Icons: `terminal`, `users`, `wrench`, `server`, `map`, `star`, `shield`, `bell`, `sliders`, `gift`, `bug`, `globe`, `flask`, `history`, `search`, `info`, `puzzle`, `bulb`.

Tabs show in the expanded view. Cards go in More options' side column, below Bug reports, and are hidden in the simple view.

## Permissions

| Permission | Lets the add-on | Calls and events |
|---|---|---|
| `server:read` | See the server: map, port, phase, players, FPS | `server.status`, `players.list`, events `server`, `players` |
| `console:read` | Read the server console and chat as they come in | `console.lines`, events `console`, `chat` |
| `rcon` | Send any console command (full control of the server) | `rcon.send`, `rcon.ask` |
| `server:control` | Stop and restart the server | `server.stop`, `server.restart` |
| `kits` | See the kits and give them to players | `kits.list`, `kits.give` |
| `maps:read` | See the map list | `maps.list` |
| `reports:read` | See the bug reports of the map on the server | `reports.list`, events `report`, `reports` |
| `reports:write` | Mark bug reports fixed, delete them, teleport you to one | `reports.setFixed`, `reports.delete`, `reports.teleport` |
| `configs:read` | Read the config files of the server, Carbon or Oxide, the plugins and Carbon's modules (they can hold passwords and keys) | `configs.list`, `configs.read` |
| `configs:write` | Change those config files (full control of the server's and the plugins' settings) | `configs.write` |
| `network` | Connect to the websites in `network` | (fetch, WebSocket, images) |

Everything else (storage, toasts, the clipboard, links, notifications, the tab's badge) needs no permission. DropLaunch checks every call in its backend against `addon.json`, not only in the window.

Those free calls have limits, so one add-on can't take over the window:

- `ui.copy`, `ui.show` and opening a `links` site without a question only work while the person is in your page: they clicked or typed there last. Do them in answer to a click, not on a timer.
- `ui.toast`: 5 in 10 seconds. `ui.notify`: 3 a minute. `ui.copy`: 10 in 10 seconds. `links` sites: 5 a minute without a question.
- A link that's asked about: one question at a time. It has a **Turn this add-on off** button, and after two "no" in a minute further ones are refused for a while.
- All calls together: 40 a second per add-on (all its pages).
- A call over its limit rejects with code `busy` ("… asked for that too often").

`players.list` leaves out the players' IP addresses (`Address`) unless the add-on has `rcon`.

## The page

```html
<!doctype html>
<html>
  <head>
    <meta charset="utf-8" />
    <link rel="stylesheet" href="../_sdk/droplaunch.css" />
    <script src="../_sdk/droplaunch.js"></script>
    <script src="my-addon.js" defer crossorigin></script>
  </head>
  <body>
    <h3 data-t="title"></h3>
  </body>
</html>
```

```js
// my-addon.js
droplaunch.ready.then(async (info) => {
  // info: { addon, app: { version, api }, language, theme, visible, calls, events }
  const server = await droplaunch.server.status();
  droplaunch.on("server", (server) => console.log(server?.phase));
  // Something newer DropLaunch versions have: ask before you use it.
  if (droplaunch.has("chat")) droplaunch.on("chat", (m) => console.log(m.name, m.text));
});
```

`droplaunch.has(name)` says whether this DropLaunch has a call or event (`"reports.list"`, `"chat"`), so one add-on works with older and newer versions. `info.calls` and `info.events` are the whole lists.

`../_sdk/droplaunch.js` and `../_sdk/droplaunch.css` are served by DropLaunch itself: don't copy them into your add-on.

## Calls

Every call returns a promise. A refused or failed call rejects with an `Error` whose message says why, in DropLaunch's language, and whose `code` says it for your code:

| `code` | |
|---|---|
| `denied` | The add-on lacks the permission, or is off |
| `noCall` | This DropLaunch doesn't have that call (check with `droplaunch.has`) |
| `invalid` | A bad argument: an empty command, a storage key over 200 bytes, storage full, a link that isn't `https://` |
| `busy` | Over a limit (above): try again later |
| `noServer` | No server runs, or its console isn't connected yet |
| `timeout` | The server didn't answer in time |
| `noPlugin` | The playtest tools aren't on the running server (`reports.teleport`) |
| `notFound` | That kit, report or player isn't there (anymore) |
| `notInUse` | Needs the person in your page (`ui.copy`) |
| `changed` | The file changed since you read it (`configs.write`): read it again |
| `failed` | Anything else: the message says what |

```js
try {
  await droplaunch.players.list();
} catch (err) {
  if (err.code === "noServer") showEmpty();
  else droplaunch.ui.toast(err.message, "error");
}
```

| Call | Answer |
|---|---|
| `droplaunch.app.info()` | `{ version, api, language }` |
| `droplaunch.server.status()` | The server (below), or `null` when none runs |
| `droplaunch.console.lines()` | The console lines DropLaunch keeps: `[{ text, kind, source, time, seq }]` |
| `droplaunch.players.list()` | Rust's `playerlist`: `[{ SteamID, OwnerSteamID, DisplayName, Ping, Address, ConnectedSeconds, VoiationLevel, Health, ... }]` (the game spells it "Voiation"). `Address` only with the `rcon` permission. DropLaunch keeps this list itself (it asks the server every 3 seconds), so calling it often costs the server nothing |
| `droplaunch.rcon.send(text)` | Sends a command like typed in DropLaunch's console: it shows there, with the server's answer |
| `droplaunch.rcon.ask(text, { timeout })` | Sends a command quietly and resolves with the server's answer (text). `timeout` in ms, 1000 to 30000, 5000 by default |
| `droplaunch.server.stop(force)`, `.restart()` | Like the buttons in DropLaunch |
| `droplaunch.kits.list()` | `[{ id, name, wear, belt, main }]` |
| `droplaunch.kits.give(kit, steamIds)` | `kit` is an id or a name. Resolves with `{ players, exact }` |
| `droplaunch.maps.list()` | DropLaunch's map list |
| `droplaunch.reports.list()` | `{ map, reports }`: the map on the server and its bug reports (below) |
| `droplaunch.reports.setFixed(id, fixed)` | Marks a report fixed (`fixed` left out: `true`) or not |
| `droplaunch.reports.delete(id)` | Deletes a report |
| `droplaunch.reports.teleport(id)` | Teleports you (the server's owner, picked on Server files) in the game to where the report was made |
| `droplaunch.configs.list()` | `{ folders }`: each server folder (one per branch, the one a server runs from first) with `id` (its branch), `name`, `framework` (`"carbon"`, `"oxide"` or `"none"`), `running`, `identity` (the running server's), `identities` (every `server/<identity>` folder) and `files` (below) |
| `droplaunch.configs.read(path, folder)` | `{ path, text, modified, size }`. `folder` is a folder's `id`; left out, the first one |
| `droplaunch.configs.write(path, text, modified, folder)` | Writes a file you read, with the `modified` read gave you; resolves with `{ path, modified, size }`. If the file changed since, it isn't written over (code `changed`). A .json file has to be valid JSON. `modified` `null` makes a `server.cfg` in an identity folder that has none (nothing else is made) |
| `droplaunch.storage.get(key, fallback)` | What the add-on saved under `key` (any JSON) |
| `droplaunch.storage.set(key, value)`, `.remove(key)`, `.keys()` | Kept in `DropLaunch-data\Add-on data\<id>.json`. 4 MB in all; `null` removes. Keys are 1 to 200 bytes. Sets made within 0.2 seconds are saved in one write (a slider that saves while it moves writes once), and `get` already sees them |
| `droplaunch.storage.flush()` | Saves the waiting sets now (they're also saved when the page goes) |
| `droplaunch.ui.toast(text, kind)` | A toast in DropLaunch, with your add-on's name in front. `kind`: `""`, `"ok"`, `"warn"`, `"error"`. A click on it shows your page (your tab, or your card) |
| `droplaunch.ui.copy(text, what)` | Copies to the clipboard and says so, with your add-on's name. Only while the person is in your page |
| `droplaunch.ui.openLink(url)` | Opens an `https://` link in the browser. Resolves with `true` when it opened. Asked first, unless its site is in `links` and the person is in your page |
| `droplaunch.ui.notify(title, body)` | A Windows notification |
| `droplaunch.ui.badge(text)` | A short text on the add-on's tab, like a count (up to 6 characters); `null` removes it |
| `droplaunch.ui.show()` | Shows the add-on's tab (`false` when the person isn't in one of your pages, a card say) |

The server (`server.status` and the `server` event) has `id`, `title`, `map`, `procgen`, `phase` (`"booting"`, `"ready"`, `"stopping"`, `"stopped"`, `"crashed"`), `rcon` (the console is connected), `port`, `connect`, `startedAt`, `readyAt`, and `info`: `{ players, maxPlayers, joining, entityCount, framerate, uptime, memory, networkIn, networkOut }`. Commands work once `phase` is `"ready"` and `rcon` is `true`.

A config file (`configs.list`) has `path` (inside the server folder, with `/`: what read and write take), `kind`, `name`, `identity`, `pluginFile`, `size` and `modified` (ms). The files there are:

| `kind` | Where | `name` |
|---|---|---|
| `server` | `server/<identity>/cfg/*.cfg` and `.json` (server.cfg, serverauto.cfg, users.cfg, bans.cfg) | the file's name; `identity` says whose |
| `framework` | `carbon/config*.json`, `oxide/*.json` | the file's name |
| `plugin` | `carbon/configs/*.json`, `oxide/config/*.json`, and one folder deeper (`carbon/configs/<plugin>/*.json`) | the plugin's (the file's name, or its folder's). `pluginFile` is its `.cs` in the plugins folder when there is one: reload it with `c.reload <name>` or `o.reload <name>` |
| `module` | `carbon/modules/<module>/config.json` | the module's (`c.loadmodule <name>` loads it again) |

Only those are listed, read or written: nothing outside them, no links or junctions that lead elsewhere, files up to 8 MB. A byte order mark at the start of a file is left out of `text` and kept when it's written. The server writes `serverauto.cfg` itself while it runs, and Carbon or Oxide read their own config at the start: say so in your page.

## Events

`droplaunch.on(name, handler)` returns a function that stops listening.

| Event | When | Data |
|---|---|---|
| `server` | Any change, including new numbers every few seconds | The server, or `null` |
| `players` | Someone joined or left (also the first list after the console connects, and everyone left when the server stops) | `{ players, joined, left }`: lists like `players.list` (without `Address`) |
| `console` | New console lines | `[{ text, kind, source, time, seq }]` |
| `chat` | A chat message on the server, one event each | `{ name, steamId, text, channel, time }`. `channel`: `"global"`, `"team"`, `"server"`, `"cards"`, `"local"`, `"clan"` or `"other"`. It comes from the server's own chat data, not from console lines, so a player can't fake one |
| `report` | A new bug report came in | A report (below) |
| `reports` | The reports changed: one came in, was fixed or deleted, or the server runs another map | `{ map, reports }`, like `reports.list` |
| `storage` | Another page of your add-on (its tab, its card) saved or removed a key | `{ key, value }` (`value` `null`: removed) |
| `theme` | The theme changed (the SDK puts its colours on the page by itself) | `{ mode, vars }` |
| `visible` | Your page can now be seen, or not: a tab while it's in front, a card while it's unfolded and on screen. Neither while the window is minimised or Settings covers it. Poll less (or not at all) while hidden | `true` or `false` (also `info.visible`) |

A report has `id`, `n` (its number on the map: #1, #2…), `text`, `name` and `steamId` (the tester), `grid` (like `"G12"`), `x`, `y`, `z`, `yaw`, `pitch`, `look` (what they looked at), `time`, `version` (the map file), `fixed` (`null`, or `{ time, version }`: when and on which map version it was marked fixed), and `repeats` with `also`: the same text sent again at the same spot (within 15 metres) while the report is open is counted on it instead of making a new one, with who sent it (`[{ name, steamId, time }]`, the last 20).

## Helpers in the page

These run inside your page and need no permission:

- `droplaunch.ui.menu(x, y, items)`: a right-click menu. Items are `{ label, run, danger, disabled, items }` (`items` makes a submenu) or `"sep"` for a line.
- `droplaunch.ui.form({ title, text, fields, ok, cancel, danger })`: a dialog that resolves with `{ name: value }` or `null`. Fields are `{ name, label, type: "text" | "select" | "textarea", value, placeholder, options: [{ value, label }] }`.
- `droplaunch.ui.confirm({ title, text, ok, cancel, danger })` resolves with `true` or `false`, and `droplaunch.ui.prompt({ title, value })` with the text or `null`.
- `droplaunch.quote(value)`: one console argument in quotes (quotes inside become `'`). Use it for names and reasons: ``rcon.ask(`kick ${droplaunch.quote(id)} ${droplaunch.quote(reason)}`)``.
- `droplaunch.parseJson(text)`: `JSON.parse` for server answers. Numbers too long for JavaScript (SteamIDs) come back as text instead of being rounded.
- `droplaunch.t(key, vars)` and `data-t`, `data-t-title`, `data-t-placeholder` attributes: texts from `lang/en.json` (`{ "key": "Text with {name}" }`), and from `lang/<code>.json` when DropLaunch uses that language. Missing keys fall back to English.

`droplaunch.css` gives the page DropLaunch's look and theme colours: `.dl-btn` (`primary`, `danger`, `ghost`, `small`), `.dl-input`, `.dl-check`, `.dl-card`, `.dl-badge` (`ok`, `warn`, `danger`), `.dl-tabs`, `.dl-table-wrap` with `.dl-table` (`th[data-sort]` with `aria-sort`, `tr.selected`, `td.num`) and `.dl-empty`. The colours are CSS variables like `var(--accent)`, `var(--text-strong)` and `rgb(var(--danger-rgb) / 0.2)`.

## What a page can't do

The page runs in a sandboxed frame with its own strict content policy:

- No scripts except files from its own folder and `../_sdk/`: no inline `<script>`, no `eval`. Inline styles are fine.
- Connections only to its own files, plus the websites in `network` with the `network` permission.
- No `localStorage`, cookies or IndexedDB (use `droplaunch.storage`), no `alert`/`confirm`/`prompt` (use `droplaunch.ui`), no popups, no workers, no frames inside it. Its frame can't leave DropLaunch's add-on pages.
- No way into DropLaunch's window, Tauri, files or Windows: only the calls above.

What that does and doesn't promise: the sandbox keeps add-ons away from the PC and from the parts of DropLaunch they didn't ask for. It doesn't make every add-on harmless. With `rcon` an add-on can do anything on the server, and the browser engine can't fully stop a page from reaching the internet (WebRTC isn't covered by content policies). Add-ons are trusted code, like Carbon and Oxide plugins.

## Sharing it

```
py pack_addon.py my-addon
```

This makes `my-addon-<version>.dladdon`: a zip with `addon.json` at the top. People drop it on DropLaunch or its exe, or use Settings → Add-ons → Add an add-on. A `.dladdon` put next to DropLaunch.exe or in the Add-ons folder is listed in Settings → Add-ons to add (a newer version of one that's added, as an update). A `.dladdon` holds at most 2000 files and 64 MB.

To update it, raise `version` and share the new file. DropLaunch shows "Update … from 0.1.0 to 0.2.0?" and keeps what the add-on saved. What the new version may do that the old one couldn't (a permission, a `network` address, a `links` site) is marked **New:** in that dialog. When the `name` or `author` isn't the same as the installed one's, the dialog warns that it replaces that add-on and gets what it saved: keep both the same in your updates.
