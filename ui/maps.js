// Maps page: the map list (the procedural map, the active maps folder + subfolders, the history
// of started maps), selection, starting a server with a map, files dropped on the window or exe,
// and maps saved in the active folder (DropLaunch watches it, like v1).
import { $, el, icon, invoke, fmtBytes, fmtWhen, toast, confirmDialog, copyText, desktopNotify, onNotificationClick } from "./util.js";
import { contextMenu } from "./menu.js";
import { readPicture } from "./mapcanvas.js";
import { t, richn } from "./i18n.js";

const PROCGEN = "procgen";
const SELECT_KEY = "droplaunch.selectedMap";
const SORT_KEY = "droplaunch.mapSort";
const COLLAPSED_KEY = "droplaunch.collapsedGroups";

/** Remembered per viewer; storage may be unavailable. */
const store = {
  get(key) {
    try {
      return localStorage.getItem(key);
    } catch {
      return null;
    }
  },
  set(key, value) {
    try {
      localStorage.setItem(key, value);
    } catch {
      /* not important */
    }
  },
};

/** A language file (checked properly when it's added). */
const isLanguageFile = (path) => /\.json$/i.test(path);

export function initMaps(ctx) {
  let list = { mapsDir: "", maps: [], history: [] };
  let selected = store.get(SELECT_KEY) || PROCGEN;
  let sort = store.get(SORT_KEY) === "name" ? "name" : "date";
  /** Groups folded away: builtin, maps, history. */
  const collapsed = new Set(
    (() => {
      try {
        return JSON.parse(store.get(COLLAPSED_KEY) || "[]");
      } catch {
        return [];
      }
    })(),
  );
  let query = "";
  let loading = null;
  /** The last map saved in the active folder, until it's started or closed. */
  let saved = null;

  const procgenEntry = () => ({
    path: PROCGEN,
    name: t("maps.procgen.name"),
    procgen: true,
    folder: "",
    startFile: ctx.state?.startFile ?? null,
  });

  function allEntries() {
    return [procgenEntry(), ...list.maps, ...list.history];
  }

  /** The entry a row shows (the same map can be in the Maps folder and the history). */
  function entryFor(node) {
    const from = node.dataset.group === "history" ? list.history : node.dataset.group === "maps" ? list.maps : [procgenEntry()];
    return from.find((m) => m.path === node.dataset.path) ?? allEntries().find((m) => m.path === node.dataset.path);
  }

  function selectedEntry() {
    return allEntries().find((m) => m.path === selected) ?? procgenEntry();
  }

  async function reload() {
    if (loading) return loading;
    loading = invoke("list_maps")
      .then((result) => {
        list = result;
        if (!allEntries().some((m) => m.path === selected)) selected = PROCGEN;
        render();
        ctx.onSelection();
      })
      .catch((err) => toast(t("maps.readFailed", { error: err }), "error"))
      .finally(() => (loading = null));
    return loading;
  }

  function matches(m) {
    if (!query) return true;
    const q = query.toLowerCase();
    return m.name.toLowerCase().includes(q) || (m.folder || "").toLowerCase().includes(q);
  }

  function sorted(maps) {
    const copy = [...maps];
    if (sort === "name") copy.sort((a, b) => a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" }));
    else copy.sort((a, b) => b.modified - a.modified);
    return copy;
  }

  function running() {
    const server = ctx.server?.view();
    return server && ["booting", "ready", "stopping"].includes(server.phase) ? server : null;
  }

  function runningPath() {
    const server = running();
    if (!server) return null;
    return server.procgen ? PROCGEN : server.map;
  }

  /** The same file (Windows paths: any case, either slash). */
  function samePath(a, b) {
    const norm = (p) => p.replaceAll("/", "\\").toLowerCase();
    return Boolean(a && b) && norm(a) === norm(b);
  }

  // ---------------------------------------------------------------- map pictures as icons
  // Drawn by the backend from each .map file (once per version, kept in the app data's Thumbs);
  // a click opens the map viewer. The procedural map gets one once a server has built it.
  /** key -> image URL, or null (none yet, being made, or the file couldn't be drawn). */
  const thumbs = new Map();
  let thumbQueue = Promise.resolve();
  /** Changes when a server may have built the procedural map again. */
  let procgenStamp = 0;
  const thumbKey = (m) => (m.procgen ? `${PROCGEN}|${procgenStamp}` : `${m.path}|${m.size}|${m.modified}`);

  function requestThumb(m) {
    const key = thumbKey(m);
    if (thumbs.has(key)) return;
    thumbs.set(key, null);
    // One at a time, in the background.
    thumbQueue = thumbQueue.then(async () => {
      try {
        const url = readPicture(await invoke("map_thumbnail", { path: m.procgen ? null : m.path })).canvas.toDataURL();
        thumbs.set(key, url);
        for (const node of document.querySelectorAll(".map-icon[data-thumb]")) {
          if (node.dataset.thumb === m.path) node.replaceWith(mapIcon(m, node.classList.contains("big")));
        }
      } catch {
        // No picture (yet): the plain icon stays.
      }
    });
  }

  // Pictures are only made for the maps that come into view (each one reads its whole .map file):
  // a big Maps folder, or a folded section, doesn't keep the disk busy for minutes.
  const thumbObserver = new IntersectionObserver(
    (entries) => {
      for (const e of entries) {
        if (!e.isIntersecting) continue;
        thumbObserver.unobserve(e.target);
        if (e.target.isConnected && e.target._map) requestThumb(e.target._map);
      }
    },
    { rootMargin: "200px" },
  );

  /** The map's picture as its icon (a click shows it big), or the plain icon until there is one. */
  function mapIcon(m, big = false) {
    const key = thumbKey(m);
    const url = thumbs.get(key);
    const cls = `map-icon${big ? " big" : ""}${m.procgen ? " procgen" : ""}`;
    if (!url) {
      const node = el("span", { class: cls }, icon(m.procgen ? "terrain" : "map"));
      node.dataset.thumb = m.path;
      if (!thumbs.has(key)) {
        node._map = m;
        thumbObserver.observe(node);
      }
      return node;
    }
    return el(
      "button",
      {
        class: `${cls} map-thumb`,
        title: t("maps.showMap", { name: m.name }),
        "aria-label": t("maps.showMap", { name: m.name }),
        onclick: (e) => {
          e.stopPropagation();
          ctx.mapViewer?.open(m);
        },
      },
      el("img", { src: url, alt: "" }),
    );
  }

  function row(m, groupKey) {
    const isRunning = samePath(runningPath(), m.path);
    // Just saved or added in the active folder: marked here, with one click to (re)start it.
    const fresh = groupKey === "maps" && saved && samePath(saved.path, m.path) ? saved : null;
    const isSelected = m.path === selected;
    const history = groupKey === "history";
    const folder = m.outside ? m.folder : m.folder ? `Maps\\${m.folder}` : "Maps";
    const meta = m.procgen
      ? el("span", { class: "map-folder", text: t("maps.procgen.meta", { seed: 1184645116, size: 3000 }) })
      : history
        ? el("span", { class: "map-folder", text: [m.started ? t("maps.started", { when: fmtWhen(m.started) }) : null, folder].filter(Boolean).join(" · "), title: m.path })
        : el("span", { class: "map-folder", text: m.folder || "Maps", title: m.outside ? m.folder : "" });
    const node = el(
      "div",
      {
        class: `map-row${isSelected ? " selected" : ""}${isRunning ? " running" : ""}${fresh ? " fresh" : ""}`,
        role: "option",
        "aria-selected": String(isSelected),
        dataset: { path: m.path, group: groupKey },
        title: m.procgen ? t("maps.procgen.title") : m.path,
      },
      mapIcon(m),
      el("div", { class: "map-main" }, el("span", { class: "map-name", text: m.name }), meta),
      isRunning && ctx.server?.stuck()
        ? el("span", { class: "chip chip-stuck", text: t("maps.chip.stuck"), title: t("maps.chip.stuck.title") })
        : isRunning
          ? el("span", { class: "chip chip-run", text: running()?.prefab ? t("maps.chip.runningPrefab") : t("maps.chip.running") })
          : null,
      fresh
        ? el("span", {
            class: "chip chip-fresh",
            text: fresh.new ? t("maps.chip.new") : t("maps.chip.saved"),
            title: fresh.new ? t("maps.chip.new.title", { when: fmtWhen(m.modified) }) : t("maps.chip.saved.title", { when: fmtWhen(m.modified) }),
          })
        : null,
      (m.startFile || m.prefabFile) && !isRunning && !fresh
        ? el("span", { class: "chip", text: t("maps.chip.startFiles"), title: [m.startFile, m.prefabFile].filter(Boolean).join("\n") })
        : null,
      el("span", { class: "map-size", text: m.procgen ? "" : fmtBytes(m.size) }),
      el("span", { class: "map-date", text: m.procgen ? "" : fmtWhen(m.modified) }),
      // The buttons in a box of their own width, so sizes and dates line up in every group
      // (history rows have one more button, a running map one less).
      el(
        "span",
        { class: "row-actions" },
        history
          ? el(
              "button",
              {
                class: "icon-btn row-forget",
                title: t("maps.forget.title"),
                onclick: (e) => {
                  e.stopPropagation();
                  forget(m);
                },
              },
              icon("x"),
            )
          : null,
        m.procgen || isRunning
          ? null
          : el(
              "button",
              {
                class: "icon-btn row-prefab",
                title: t("maps.prefab.title", { name: m.name }),
                onclick: (e) => {
                  e.stopPropagation();
                  select(m.path);
                  start(m, { prefab: true });
                },
              },
              icon("flask"),
            ),
        fresh
          ? el(
              "button",
              {
                class: "btn btn-small btn-primary row-fresh",
                title: isRunning ? t("maps.fresh.restart.title", { name: m.name }) : t("maps.start.title", { name: m.name }),
                onclick: (e) => {
                  e.stopPropagation();
                  startFresh(m, isRunning);
                },
              },
              icon(isRunning ? "refresh" : "play"),
              el("span", { text: isRunning ? t("maps.fresh.restart") : t("maps.fresh.start") }),
            )
          : el(
              "button",
              {
                class: "icon-btn row-play",
                title: isRunning ? t("maps.running") : t("maps.start.title", { name: m.name }),
                disabled: isRunning ? true : null,
                onclick: (e) => {
                  e.stopPropagation();
                  select(m.path);
                  start(m);
                },
              },
              icon("play"),
            ),
        fresh
          ? el(
              "button",
              {
                class: "icon-btn row-dismiss",
                title: t("common.dismiss"),
                onclick: (e) => {
                  e.stopPropagation();
                  saved = null;
                  render();
                },
              },
              icon("x"),
            )
          : null,
      ),
    );
    return node;
  }

  /** Searching shows every match, folded groups included. */
  const isOpen = (key) => Boolean(query) || !collapsed.has(key);

  const GROUP_ICONS = { builtin: "terrain", maps: "folder", history: "history" };

  /** A section of the list: its header (folds it away) and its rows. */
  function section(key, title, count, rows, extra) {
    const open = isOpen(key);
    const head = el(
      "button",
      {
        type: "button",
        class: `map-group${open ? "" : " collapsed"}`,
        "aria-expanded": String(open),
        title: open ? t("maps.group.fold") : t("maps.group.show"),
        onclick: () => {
          if (collapsed.has(key)) collapsed.delete(key);
          else collapsed.add(key);
          store.set(COLLAPSED_KEY, JSON.stringify([...collapsed]));
          render();
        },
      },
      icon("chevron", "group-chevron"),
      icon(GROUP_ICONS[key], "group-icon"),
      el("span", { class: "group-title", text: title }),
      count != null ? el("span", { class: "map-count", text: String(count) }) : null,
      el("span", { class: "group-rule" }),
      extra ?? null,
    );
    return el("section", { class: `map-section group-${key}${open ? "" : " folded"}`, dataset: { group: key } }, head, ...(open ? rows : []));
  }

  function render() {
    const box = $("map-list");
    const scroll = box.scrollTop;
    // The rows are made again: the old ones' icons needn't be watched.
    thumbObserver.disconnect();
    const maps = sorted(list.maps).filter(matches);
    const history = list.history.filter(matches);
    const procgen = procgenEntry();
    const sections = [];
    if (matches(procgen)) sections.push(section("builtin", t("maps.group.builtin"), null, [row(procgen, "builtin")]));
    let folderRows;
    if (maps.length) folderRows = maps.map((m) => row(m, "maps"));
    else if (list.maps.length) folderRows = [el("div", { class: "map-empty", text: t("maps.noMatch", { query }) })];
    else {
      folderRows = [el("div", { class: "map-empty" }, el("strong", { text: t("maps.empty.title") }), " ", el("span", { text: t("maps.empty.text") }))];
    }
    const watching = el("span", { class: "watching", title: t("maps.watching.title") }, el("i"), el("span", { text: t("folders.state.watching") }));
    // After Reset everything no maps folder is linked until setup has run.
    const linked = ctx.state?.linked !== false;
    if (linked) sections.push(section("maps", t("maps.group.folder"), list.maps.length, folderRows, watching));
    if (history.length) sections.push(section("history", t("maps.group.history"), history.length, history.map((m) => row(m, "history"))));
    box.replaceChildren(...sections);
    box.scrollTop = scroll;

    const count = list.maps.length;
    const sub = $("maps-sub");
    $("btn-maps-folder").hidden = !linked;
    if (!linked) sub.replaceChildren(t("maps.notLinked"));
    else sub.replaceChildren(...richn("maps.count", count, { dir: el("span", { class: "path", text: list.mapsDir, title: t("maps.dir.title", { dir: list.mapsDir }) }) }));
    $("btn-maps-sort").title = sort === "name" ? t("maps.sort.name") : t("maps.sort.date");
    $("btn-maps-sort").classList.toggle("active", sort === "name");
  }

  /** What the list shows about the server (which map runs, prefab test mode, stuck). */
  let serverKey = "";

  /** The server changed: draw the list again only when what it shows of it did (not for every
   * boot step, or the player count every few seconds). */
  function serverChanged() {
    const server = running();
    const key = server ? `${runningPath()}|${server.prefab}|${ctx.server?.stuck()}|${server.phase === "ready"}` : "";
    if (key === serverKey) return;
    // A procedural map server that got ready has built its map: its picture can be made now.
    if (server?.procgen && server.phase === "ready") procgenStamp++;
    serverKey = key;
    render();
  }

  // ---------------------------------------------------------------- saved maps
  /** The saved map's Start (or Restart with it, when that map runs). */
  async function startFresh(m, isRunning) {
    saved = null;
    if (isRunning) {
      render();
      await invoke("restart_server").catch((err) => toast(String(err), "error"));
      return;
    }
    select(m.path);
    await start(m);
  }

  /** The watcher saw the active folder change (or a map in it saved). */
  async function onFolderChange(change) {
    await reload();
    if (!change?.saved || ctx.state?.notifySaves === false) return;
    saved = change.saved;
    // Its group opens and the row comes into view.
    if (collapsed.delete("maps")) store.set(COLLAPSED_KEY, JSON.stringify([...collapsed]));
    render();
    $("map-list").querySelector(".map-row.fresh")?.scrollIntoView({ block: "nearest" });
    // DropLaunch is in the background: flash its taskbar button, and a Windows notification
    // starts the map with one click (like v1).
    if (!document.hasFocus()) {
      window.__TAURI__.window?.getCurrentWindow?.().requestUserAttention?.(2).catch(() => {});
      const isRunning = samePath(runningPath(), saved.path);
      desktopNotify({
        tag: "saved",
        title: saved.new ? t("maps.notify.added", { name: saved.name }) : t("maps.notify.saved", { name: saved.name }),
        body: isRunning ? t("maps.notify.restart") : t("maps.notify.start"),
        action: isRunning ? "restart" : "start",
        buttons: isRunning ? [] : [[t("server.prefabMode"), "prefab"]],
      });
    }
  }

  /** A click on the "map saved" notification: what the notice's buttons do. */
  onNotificationClick("saved", async (action) => {
    if (!saved) return;
    const m = list.maps.find((x) => samePath(x.path, saved.path)) ?? { path: saved.path, name: saved.name };
    if (action === "restart" && samePath(runningPath(), m.path)) {
      saved = null;
      render();
      await invoke("restart_server").catch((err) => toast(String(err), "error"));
      return;
    }
    select(m.path);
    await start(m, { prefab: action === "prefab" });
  });

  /** Select a map the list shows (false: it doesn't). */
  function select(path) {
    const entry = allEntries().find((m) => samePath(m.path, path));
    if (!entry) return false;
    path = entry.path;
    if (selected === path) return true;
    selected = path;
    store.set(SELECT_KEY, path);
    for (const node of $("map-list").querySelectorAll(".map-row")) {
      const on = node.dataset.path === path;
      node.classList.toggle("selected", on);
      node.setAttribute("aria-selected", String(on));
      if (on) node.scrollIntoView({ block: "nearest" });
    }
    ctx.onSelection();
    return true;
  }

  async function forget(m) {
    try {
      await invoke("forget_map", { path: m.path });
      await reload();
    } catch (err) {
      toast(String(err), "error");
    }
  }

  /** Start a server with `m`, normally or in prefab test mode (asks before replacing a running
   * server or ignoring Oxide). */
  async function start(m, { prefab = false, replace = false, ignoreOxide = false } = {}) {
    const state = ctx.state;
    if (!state) return;
    if (state.setupNeeded) {
      ctx.openSetup(m.procgen ? null : m);
      return;
    }
    // SteamCMD may be busy with another branch's folder: this one can start meanwhile.
    if (ctx.startBlocked()) {
      toast(t("maps.blocked"));
      return;
    }
    const target = m.procgen ? { kind: "procgen" } : { kind: "map", path: m.path };
    prefab = prefab && !m.procgen;
    const name = prefab ? t("maps.namePrefab", { name: m.name }) : m.name;
    let reply;
    try {
      reply = await invoke("start_server", { target, prefab, replace, ignoreOxide });
    } catch (err) {
      toast(String(err), "error");
      ctx.server?.note(String(err));
      return;
    }
    if (saved && samePath(saved.path, m.path)) saved = null;
    switch (reply.result) {
      case "busy": {
        const saving = ctx.server?.view()?.phase === "ready" && ctx.server.view().rcon;
        const ok = await confirmDialog({
          title: t("maps.busy.title", { title: reply.title }),
          text: t(saving ? "maps.busy.textSaving" : "maps.busy.text", { title: reply.title, name }),
          ok: t("maps.busy.ok", { name: m.name }),
        });
        if (ok) await start(m, { prefab, replace: true, ignoreOxide });
        break;
      }
      case "oxideInactive": {
        const choice = await confirmDialog({
          title: t("mods.oxide.title"),
          text: t("maps.oxide.text"),
          choices: [
            { label: t("maps.oxide.start"), value: "start" },
            { label: t("maps.oxide.reapply"), value: "reapply", primary: true },
          ],
        });
        if (choice === "start") await start(m, { prefab, replace, ignoreOxide: true });
        else if (choice === "reapply") {
          if (await ctx.reapplyOxide()) await start(m, { prefab, replace });
        }
        break;
      }
      case "queued":
        toast(t("maps.queued", { title: reply.title, name }));
        break;
      case "started":
        ctx.server?.apply(reply.server);
        break;
    }
    await ctx.refresh();
    await reload();
  }

  /** Files dropped on the window, or the exe was opened with (kit files go to the kits,
   * language files to Settings). */
  async function openFiles(all) {
    let json = (all ?? []).filter(isLanguageFile);
    const paths = (all ?? []).filter((p) => !isLanguageFile(p));
    if (paths.length) await openMaps(paths);
    if (json.length) json = await ctx.kits.importFiles(json);
    if (json.length) await ctx.settings.addLanguages(json);
  }

  async function openMaps(paths) {
    let result;
    try {
      result = await invoke("import_files", { paths });
    } catch (err) {
      toast(String(err), "error");
      return;
    }
    if (result.problems.length) toast(result.problems.join(" · "), result.maps.length ? "" : "error");
    // What else a .zip held: its kits and language files are added (not switched to), and its
    // start file's settings go into the map's own start files.
    if (result.files?.length) {
      const rest = await ctx.kits.importFiles(result.files);
      if (rest.length) await ctx.settings.addLanguages(rest, { use: false });
    }
    if (result.startFiles?.length && result.maps.length) {
      toast(t("maps.zipStartFile", { file: result.startFiles[0].split(/[\\/]/).pop() }), "ok");
    }
    if (!result.maps.length) return;
    ctx.setTab("maps");
    await reload();
    const first = allEntries().find((m) => samePath(m.path, result.maps[0])) ?? {
      path: result.maps[0],
      name: result.maps[0].split(/[\\/]/).pop().replace(/\.map$/i, ""),
    };
    select(first.path);
    if (ctx.state?.setupNeeded) {
      // Fresh install: set everything up and start this map, no questions.
      ctx.openSetup(first, { auto: true });
      return;
    }
    if (result.maps.length > 1) {
      toast(t("maps.added", { n: result.maps.length }));
      return;
    }
    // A re-export of the map that's running starts the same way it runs now.
    const now = running();
    await start(first, { prefab: Boolean(now?.prefab && samePath(now.map, first.path)) });
  }

  // ---------------------------------------------------------------- right-click menu
  async function reveal(path) {
    try {
      await invoke("reveal", { path });
    } catch (err) {
      toast(String(err), "error");
    }
  }

  function openMenu(m, x, y) {
    const isRunning = samePath(runningPath(), m.path);
    const files = [m.startFile, m.prefabFile].filter(Boolean);
    contextMenu(x, y, [
      { label: isRunning ? t("maps.running") : t("server.start"), icon: "play", kind: "start", disabled: isRunning, run: () => start(m) },
      !m.procgen && { label: t("server.startPrefab"), icon: "flask", kind: "prefab", disabled: isRunning, run: () => start(m, { prefab: true }) },
      "sep",
      !m.procgen && { label: t("server.showMapFile"), icon: "map", run: () => reveal(m.path) },
      files.length
        ? { label: files.length > 1 ? t("maps.menu.showStartFiles") : t("maps.menu.showStartFile"), icon: "folder", run: () => reveal(files[0]) }
        : { label: m.procgen ? t("maps.menu.createStartFile") : t("maps.menu.createStartFiles"), icon: "file", disabled: ctx.state?.setupNeeded, run: () => ctx.server.makeStartFile(m) },
      !m.procgen && { label: t("maps.menu.copyPath"), icon: "copy", run: () => copyText(m.path, t("maps.menu.pathWhat")) },
      m.started != null && "sep",
      m.started != null && { label: t("maps.menu.forget"), icon: "x", run: () => forget(m) },
    ]);
  }

  $("map-list").addEventListener("contextmenu", (e) => {
    const node = e.target.closest(".map-row");
    if (!node) return;
    e.preventDefault();
    select(node.dataset.path);
    const m = entryFor(node);
    if (m) openMenu(m, e.clientX, e.clientY);
  });

  // ---------------------------------------------------------------- wiring
  $("map-list").addEventListener("click", (e) => {
    const node = e.target.closest(".map-row");
    if (node) select(node.dataset.path);
  });
  $("map-list").addEventListener("dblclick", (e) => {
    const node = e.target.closest(".map-row");
    if (!node || e.target.closest("button")) return;
    const m = entryFor(node);
    if (m && !samePath(runningPath(), m.path)) start(m);
  });
  $("map-list").addEventListener("keydown", (e) => {
    const rows = [...$("map-list").querySelectorAll(".map-row")];
    const index = rows.findIndex((r) => r.dataset.path === selected);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const next = rows[Math.max(0, Math.min(rows.length - 1, index + (e.key === "ArrowDown" ? 1 : -1)))];
      if (next) select(next.dataset.path);
    } else if (e.key === "Enter") {
      e.preventDefault();
      start(selectedEntry());
    } else if (e.key === "ContextMenu" || (e.key === "F10" && e.shiftKey)) {
      e.preventDefault();
      const row = $("map-list").querySelector(".map-row.selected")?.getBoundingClientRect();
      if (row) openMenu(selectedEntry(), row.left + 40, row.bottom - 6);
    }
  });
  $("map-search").addEventListener("input", (e) => {
    query = e.target.value.trim();
    render();
  });
  $("btn-maps-sort").addEventListener("click", () => {
    sort = sort === "date" ? "name" : "date";
    store.set(SORT_KEY, sort);
    render();
  });
  $("btn-maps-refresh").addEventListener("click", reload);
  $("btn-maps-folder").addEventListener("click", () => invoke("open_folder", { path: list.mapsDir }).catch((err) => toast(String(err))));
  // Pick up maps exported while DropLaunch was in the background.
  window.addEventListener("focus", () => reload());

  // Drag & drop onto the window (Tauri delivers real file paths).
  const overlay = $("drop-overlay");
  const webview = window.__TAURI__.webview?.getCurrentWebview?.();
  webview?.onDragDropEvent((event) => {
    const { type, paths } = event.payload;
    if (type === "enter" || type === "over") {
      if (type === "enter" && paths) {
        const usable = paths.filter((p) => /\.(map|zip)$/i.test(p) || !/\.[^\\/]+$/.test(p));
        const languages = paths.filter(isLanguageFile);
        if (languages.length && !usable.length) {
          $("drop-title").textContent = t("drop.language");
          $("drop-detail").textContent = t("drop.language.detail");
        } else {
          $("drop-title").textContent = usable.length ? (usable.length > 1 ? t("drop.many", { n: usable.length }) : t("drop.start")) : t("drop.nope");
          $("drop-detail").textContent = usable.length ? t("drop.detail") : t("drop.nope.detail");
        }
        overlay.classList.toggle("nope", !usable.length && !languages.length);
      }
      overlay.hidden = false;
    } else if (type === "leave") {
      overlay.hidden = true;
    } else if (type === "drop") {
      overlay.hidden = true;
      openFiles(paths);
    }
  });
  window.__TAURI__.event.listen("open-files", (e) => openFiles(e.payload));
  // (Not while no maps folder is linked, after Reset everything.)
  window.__TAURI__.event.listen("maps-changed", (e) => ctx.state?.linked !== false && onFolderChange(e.payload));

  return {
    reload,
    render,
    serverChanged,
    icon: mapIcon,
    /** The playtest page drew the running map: the procedural map's icon can be made too. */
    pictureMade() {
      if (!running()?.procgen) return;
      procgenStamp++;
      render();
    },
    start,
    openFiles,
    selected: selectedEntry,
    select,
    /** The list shows this map (in the active folder or the history). */
    has: (path) => allEntries().some((m) => samePath(m.path, path)),
  };
}
