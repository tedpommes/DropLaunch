// DropLaunch v2 UI. Talks to the Rust side through Tauri's invoke(), Channel streams and events.
import { $, el, icon, pathChip, invoke, Channel, fmtBytes, fmtDate, fmtAgo, fmtDuration, fileName, buildNum, toast, confirmDialog, copyText, Rate, followScroll, sizeTag, fitPath, driveOf } from "./util.js";
import { Dropdown } from "./dropdown.js";
import { initMaps } from "./maps.js";
import { initServer } from "./server.js";
import { initLayout } from "./layout.js";
import { initGame } from "./game.js";
import { initUpdates } from "./updates.js";
import { initAbout } from "./about.js";
import { initPlaytest } from "./playtest.js";
import { initMapViewer } from "./mapviewer.js";
import { initGuide } from "./guide.js";
import { initSettings } from "./settings.js";
import { initKits } from "./kits.js";
import { t, tn, rich, fmtList } from "./i18n.js";

const MAX_LOG_LINES = 4000;
const STEPS = ["steamcmd", "login", "files", "done"];
const FRAMEWORK_NAMES = { none: t("framework.none"), carbon: t("framework.carbon"), oxide: t("framework.oxide") };

let state = null;
let busy = false;
/** The branch whose server folder the running SteamCMD job changes (null: none, like the branch list). */
let jobBranch = null;
/** Simple view (maps + server panel) or everything (tabs, console, server files, mods, folders). */
let expanded = false;
let jobStarted = 0;
let jobLabel = "";
let elapsedTimer = null;
/** The last job's outcome, shown in the strip on the Maps page for a little while. */
let stripResult = null;
let stripResultTimer = null;
const mainRate = new Rate();
const setupRate = new Rate();
const logScroll = followScroll($("log"));

// ---------------------------------------------------------------- shared context for the pages
const ctx = {
  get state() {
    return state;
  },
  get expanded() {
    return expanded;
  },
  refresh,
  openSetup,
  setTab,
  showFiles,
  spaceTag,
  reapplyOxide,
  /** A SteamCMD job runs. */
  busy: () => busy,
  /** SteamCMD is changing the branch in use: no server can start from it (another branch's can). */
  startBlocked: () => busy && jobBranch != null && jobBranch === state?.branch,
  /** Saved (true), or the reason shown (false). */
  async setOption(patch) {
    try {
      state = await invoke("set_options", { options: patch });
      render();
      return true;
    } catch (err) {
      toast(String(err), "error");
      return false;
    }
  },
  onSelection: () => ctx.server?.render(),
  // The splitter moved or a console went full screen.
  onResize: () => {
    ctx.server?.render();
    ctx.server?.bottom();
    logScroll.reveal();
  },
  onServerChange: () => {
    ctx.maps?.serverChanged();
    if (state) {
      applyBusy();
      renderStrip();
      ctx.guide?.render();
    }
  },
};

/**
 * A server from the branch in use runs: its files are in use. Every branch has its own folder,
 * so another branch's server doesn't hold up updating this one.
 */
function runningHere() {
  const view = ctx.server?.view();
  return Boolean(ctx.server?.isLive() && (!view?.branch || view.branch === state?.branch));
}

// ---------------------------------------------------------------- view & tabs
function setTab(name) {
  ctx.playtest?.close();
  if (!expanded) name = "maps";
  if (ctx.layout?.maximized()) ctx.layout.setMaximized(null);
  for (const tab of document.querySelectorAll(".tabs [role=tab]")) {
    tab.setAttribute("aria-selected", String(tab.dataset.tab === name));
  }
  $("view-maps").hidden = name !== "maps";
  $("view-files").hidden = name !== "files";
  if (name === "maps") ctx.server?.reveal();
  else logScroll.reveal();
}

// The simple view is a narrower window (it opens at its smallest size); each view keeps the
// width it had this session.
// Expanded: the Server files buttons (branch, check, verify, update) fit on one line.
const VIEW_WIDTH = { simple: 640, expanded: 1240 };
const VIEW_MIN = { simple: [640, 560], expanded: [900, 620] };
const viewWidths = {};
/** The smallest size a view allows, waiting until the window is no longer maximized or snapped. */
let pendingMin = null;

/**
 * Maximized, full screen, or snapped to a side or corner of the screen. The window is left alone
 * then: even a new minimum size makes Windows take it out of that (the window library sets the
 * size again to apply it).
 */
async function windowPlaced(win) {
  if ((await win.isMaximized()) || (await win.isFullscreen())) return true;
  const monitor = await window.__TAURI__.window.currentMonitor();
  const area = monitor?.workArea;
  if (!area) return false;
  const pos = await win.outerPosition();
  const size = await win.outerSize();
  // Snapped windows touch the work area's edges (give or take the invisible resize border).
  const near = (a, b) => Math.abs(a - b) <= 16;
  const left = near(pos.x, area.position.x);
  const right = near(pos.x + size.width, area.position.x + area.size.width);
  const top = near(pos.y, area.position.y);
  const bottom = near(pos.y + size.height, area.position.y + area.size.height);
  return (top && bottom && (left || right)) || ((left || right) && (top || bottom) && !(left && right));
}

async function fitWindow(on) {
  const api = window.__TAURI__;
  const win = api?.window?.getCurrentWindow?.();
  if (!win || !api.dpi) return;
  const { LogicalSize, PhysicalSize, PhysicalPosition } = api.dpi;
  const [mode, leaving] = on ? ["expanded", "simple"] : ["simple", "expanded"];
  try {
    if (await windowPlaced(win)) {
      // Applied once the window is back to a normal one.
      pendingMin = VIEW_MIN[mode];
      return;
    }
    pendingMin = null;
    const factor = await win.scaleFactor();
    const inner = await win.innerSize();
    const outer = await win.outerSize();
    const pos = await win.outerPosition();
    viewWidths[leaving] = inner.width / factor;
    const logical = viewWidths[mode] ?? (on ? Math.max(VIEW_WIDTH.expanded, viewWidths.simple) : VIEW_WIDTH.simple);
    const frame = outer.width - inner.width;
    const monitor = await api.window.currentMonitor();
    const area = monitor?.workArea ?? (monitor ? { position: monitor.position, size: monitor.size } : null);
    // Never wider than the screen.
    const width = Math.min(Math.round(logical * factor), area ? area.size.width - frame : Infinity);
    if (!on) await win.setMinSize(new LogicalSize(...VIEW_MIN.simple));
    if (width !== inner.width) {
      // Grow or shrink around the middle, and stay on the screen.
      let x = Math.round(pos.x - (width - inner.width) / 2);
      if (area) x = Math.max(area.position.x, Math.min(x, area.position.x + area.size.width - width - frame));
      await win.setPosition(new PhysicalPosition(x, pos.y));
      await win.setSize(new PhysicalSize(width, inner.height));
    }
    if (on) await win.setMinSize(new LogicalSize(...VIEW_MIN.expanded));
  } catch (err) {
    console.warn("Could not resize the window", err);
  }
}

// Back from maximized or snapped: now the view's minimum size can be applied.
window.__TAURI__?.window
  ?.getCurrentWindow?.()
  ?.onResized?.(async () => {
    if (!pendingMin) return;
    const win = window.__TAURI__.window.getCurrentWindow();
    try {
      if (await windowPlaced(win)) return;
      const min = pendingMin;
      pendingMin = null;
      await win.setMinSize(new window.__TAURI__.dpi.LogicalSize(...min));
    } catch (err) {
      console.warn("Could not set the window's minimum size", err);
    }
  })
  ?.catch(() => {});

function setExpanded(on, { resize = true } = {}) {
  if (resize && on !== expanded) fitWindow(on);
  ctx.playtest?.close();
  if (ctx.layout?.maximized()) ctx.layout.setMaximized(null);
  expanded = on;
  document.body.classList.toggle("simple", !on);
  const button = $("btn-view");
  button.setAttribute("aria-pressed", String(on));
  button.title = on ? t("topbar.simple.title") : t("topbar.more.title");
  button.replaceChildren(icon(on ? "collapse" : "expand"), el("span", { text: on ? t("topbar.simple") : t("topbar.more") }));
  if (!on) setTab("maps");
  else ctx.server?.reveal();
  ctx.server?.render();
}

/** The server files page (from the status pills). */
function showFiles() {
  setExpanded(true);
  setTab("files");
}

// ---------------------------------------------------------------- state
/** An installed branch (each has its own folder), or null. */
function installedBranch(branch) {
  return state?.installs?.find((i) => i.branch === branch) ?? null;
}

/** More than one branch installed: the UI says which one is in use. */
const multiBranch = () => (state?.installs?.length ?? 0) > 1;

function latestFor(branch) {
  return state?.branchCache?.branches.find((b) => b.name === branch) ?? null;
}

/** Detected vs newest build for the installed server. */
function buildStatus() {
  const server = state.server;
  if (!server) return { kind: "none" };
  const latest = latestFor(server.branch);
  if (!server.fullyInstalled) return { kind: "incomplete", latest };
  if (!latest) return { kind: "unknown" };
  return buildNum(latest.buildid) > buildNum(server.buildid) ? { kind: "update", latest } : { kind: "newest", latest };
}

/** What the big button does right now (null: nothing to do, the button hides). */
function primaryAction() {
  const server = state.server;
  const branch = state.branch;
  if (state.setupNeeded) return { label: installedBranch(branch) || !state.installs?.length ? t("files.action.fixSetup") : t("setup.install", { branch }), setup: true };
  if (!server.fullyInstalled) return { label: t("files.action.repair"), validate: true };
  if (buildStatus().kind === "update") return { label: t("files.action.update") };
  return null;
}

async function refresh() {
  try {
    state = await invoke("get_state");
    render();
  } catch (err) {
    toast(t("common.readStateFailed", { error: err }), "error");
  }
}

// ---------------------------------------------------------------- rendering
/** `short` is what a narrow window shows (the coloured dot says the rest). */
function setPill(node, cls, text, short) {
  node.className = `pill ${cls}`;
  node.querySelector("span").textContent = text;
  node.dataset.short = short;
}

function render() {
  if (!state) return;
  const server = state.server;
  const serverFolder = state.folders.find((f) => f.key === "server");
  const status = buildStatus();
  const selected = latestFor(state.branch);

  const steamcmdText = state.steamcmdInstalled ? t("topbar.pill.steamcmd.ready") : t("topbar.pill.steamcmd.missing");
  setPill($("pill-steamcmd"), state.steamcmdInstalled ? "ok" : "", steamcmdText, t("topbar.pill.steamcmd.short"));
  const pill = {
    none: ["", t("topbar.pill.server.none")],
    incomplete: ["warn", t("topbar.pill.server.incomplete")],
    unknown: ["ok", t("topbar.pill.server.ready")],
    update: ["warn", t("topbar.pill.server.update")],
    newest: ["ok", t("topbar.pill.server.newest")],
  }[status.kind];
  // An update always names its branch (the installed one it's for).
  const pillBranch = status.kind === "update" ? server.branch : multiBranch() ? state.branch : null;
  setPill($("pill-server"), pill[0], pillBranch ? t("topbar.pill.withBranch", { status: pill[1], branch: pillBranch }) : pill[1], multiBranch() ? state.branch : t("topbar.pill.server.short"));
  $("pill-server").title = server
    ? t("topbar.pill.server.titleBuild", { status: pill[1], build: server.buildid, branch: server.branch })
    : t("topbar.pill.titleClick", { status: pill[1] });
  $("pill-steamcmd").title = t("topbar.pill.titleClick", { status: steamcmdText });
  $("tab-files-dot").hidden = !(state.setupNeeded || status.kind === "update" || status.kind === "incomplete" || state.oxideInactive);

  let message;
  if (!state.canHost) message = t("files.sub.noHost");
  else if (!server) message = t("files.sub.none", { folder: serverFolder.path });
  else if (status.kind === "incomplete") message = t("files.sub.incomplete");
  else if (status.kind === "update") message = t("files.sub.update", { branch: server.branch });
  else if (status.kind === "newest") message = t("files.sub.newest");
  else message = t("files.sub.ready");
  $("server-sub").textContent = message;
  // Folded, the card's line also says what the tiles would.
  $("server-facts-line").textContent = server ? ` ${t("files.factsLine", { build: server.buildid, branch: server.branch, size: fmtBytes(server.sizeOnDisk) })}` : "";

  // The installed build; an available update shows in the same tile.
  $("fact-build").textContent = server ? server.buildid : "—";
  const buildNote = $("fact-build-note");
  const updateAvailable = status.kind === "update";
  buildNote.textContent = !server
    ? t("files.note.notInstalled")
    : !server.fullyInstalled
      ? t("files.note.incomplete")
      : updateAvailable
        ? t("files.note.update", { branch: server.branch, build: status.latest.buildid, date: fmtDate(status.latest.timeUpdated) })
        : status.kind === "newest"
          ? t("files.note.newestInstalled", { date: fmtDate(server.lastUpdated) })
          : t("files.note.branchInstalled", { branch: server.branch, date: fmtDate(server.lastUpdated) });
  buildNote.classList.toggle("warn", Boolean(server && (!server.fullyInstalled || updateAvailable)));
  $("fact-build-box").classList.toggle("has-update", updateAvailable);
  $("fact-build-box").title = state.branchCache ? t("files.newestFrom", { source: state.branchCache.source || "Steam", ago: fmtAgo(state.branchCache.checkedAt) }) : "";

  $("fact-branch").textContent = state.branch;
  const others = (state.installs ?? []).filter((i) => !i.active).map((i) => i.branch);
  $("fact-branch-note").textContent = others.length ? t("files.note.alsoInstalled", { branches: fmtList(others) }) : selected?.description || "";

  $("fact-size").textContent = server ? fmtBytes(server.sizeOnDisk) : "—";
  $("fact-size-note").textContent = server ? (server.executableFound ? t("files.note.exeFound") : t("files.note.exeMissing")) : "";

  $("update-badge").hidden = status.kind !== "update";

  // Setup banner (shown when the setup window was dismissed).
  $("setup-banner").hidden = !state.setupNeeded;
  $("setup-banner-text").textContent = state.linked === false ? t("files.banner.notLinked") : !state.steamcmdInstalled && !server ? t("files.banner.both") : !state.steamcmdInstalled ? t("files.banner.steamcmd") : t("files.banner.server");

  renderBranches($("branch-select"), state.branch);
  renderInstalls();
  renderMapsBranch();
  renderFolders();
  renderMods();

  const action = primaryAction();
  $("btn-primary-label").textContent = action?.label ?? "";
  $("btn-check").title = state.branchCache
    ? t("files.check.titleLast", { source: state.branchCache.source || "Steam", ago: fmtAgo(state.branchCache.checkedAt) })
    : t("files.check.title");

  $("footer-config").textContent = t("footer.settings", { file: state.configFile });
  $("footer-version").textContent = `v${state.version}`;
  applyBusy();
  renderStrip();
  ctx.server?.render();
  ctx.game?.render();
  ctx.updates?.render();
  ctx.guide?.render();
  ctx.settings?.render();
  ctx.kits?.render();
}

// ---------------------------------------------------------------- strip (Maps page)
/** What the server files need right now, in one line, with the button that does it. */
function renderStrip() {
  const box = $("strip");
  if (!state) return;
  const status = buildStatus();
  const running = runningHere();
  let s = null;
  if (busy && $("setup").hidden) {
    // A setup carries on in the background (another branch's server can be used meanwhile).
    s = setupState.running
      ? { icon: "download", title: jobLabel, detail: "", action: t("strip.show"), kind: "busy", run: showSetup }
      : { icon: "download", title: jobLabel || t("strip.working"), detail: "", action: t("common.cancel"), kind: "busy", run: cancelJob };
  } else if (stripResult) {
    s = { ...stripResult, close: true };
  } else if (state.setupNeeded) {
    const detail = state.linked === false ? t("files.banner.notLinked") : !state.steamcmdInstalled && !state.server ? t("strip.missing.both") : !state.steamcmdInstalled ? t("strip.missing.steamcmd") : t("strip.missing.server");
    s = { icon: "alert", title: t("files.setupNeeded"), detail, action: t("files.fixIt"), kind: "warn", run: () => openSetup() };
  } else if (status.kind === "incomplete") {
    s = { icon: "alert", title: t("strip.incomplete.title"), detail: t("strip.incomplete.detail"), action: t("strip.repair"), kind: "warn", run: () => startUpdate(true), blocked: running };
  } else if (state.oxideInactive) {
    s = { icon: "alert", title: t("mods.oxide.title"), detail: t("strip.oxide.detail"), action: t("mods.oxide.reapply"), kind: "warn", run: reapplyOxide, blocked: running };
  } else if (status.kind === "update") {
    s = {
      icon: "download",
      title: t("strip.update.title", { branch: state.server.branch }),
      detail: t("strip.update.detail", { build: status.latest.buildid, date: fmtDate(status.latest.timeUpdated) }),
      action: t("files.action.update"),
      kind: "update",
      run: () => startUpdate(false),
      blocked: running,
    };
  }
  box.hidden = !s;
  if (!s) return;
  if (s.blocked) s.detail = t("strip.stopFirst", { detail: s.detail });
  box.className = `card strip ${s.kind ?? ""}`.trim();
  $("strip-icon").replaceChildren(icon(s.icon));
  $("strip-title").textContent = s.title;
  $("strip-detail").textContent = s.detail;
  $("strip-detail").hidden = !s.detail;
  const action = $("strip-action");
  action.hidden = !s.action;
  action.textContent = s.action ?? "";
  action.className = `btn btn-small ${s.kind === "busy" && s.run === cancelJob ? "btn-danger" : s.kind === "busy" ? "btn-ghost" : "btn-primary"}`;
  action.disabled = Boolean(s.blocked);
  action.title = s.blocked ? t("files.stopFirst") : "";
  action.onclick = s.run ?? null;
  $("strip-close").hidden = !s.close;
  $("strip-progress").hidden = s.kind !== "busy";
  // A server update asks whether Carbon or Oxide comes along (saved: Settings has it too).
  const framework = s.kind === "update" && state.framework !== "none" ? FRAMEWORK_NAMES[state.framework] : null;
  $("strip-framework").hidden = !framework;
  if (framework) {
    $("strip-framework-label").textContent = t("strip.updateFramework", { framework });
    $("strip-framework").title = t("strip.updateFramework.title", { framework });
    $("strip-framework-on").checked = state.updateFramework !== false;
  }
}
$("strip-framework-on").addEventListener("change", (e) => ctx.setOption({ updateFramework: e.target.checked }));

function showStripResult(result) {
  clearTimeout(stripResultTimer);
  stripResult = result;
  stripResultTimer = setTimeout(() => {
    stripResult = null;
    renderStrip();
  }, 12000);
  renderStrip();
}

/** Mirrors the main progress bar into the strip. */
function stripProgress(text, percent) {
  $("strip-text").textContent = text;
  $("strip-pct").textContent = percent == null ? "" : `${percent.toFixed(1)}%`;
  $("strip-bar").classList.toggle("indeterminate", percent == null);
  $("strip-fill").style.width = percent == null ? "0%" : `${Math.min(100, Math.max(0, percent))}%`;
}

function cancelJob() {
  $("btn-cancel").disabled = true;
  $("strip-action").disabled = true;
  setStatus(t("files.stoppingSteamcmd"));
  invoke("cancel_job").finally(() => ($("btn-cancel").disabled = false));
}

function branchMeta(name) {
  const b = latestFor(name);
  const here = installedBranch(name);
  return [here ? (here.active ? t("files.branch.inUse") : t("files.branch.installed")) : null, b ? `${b.buildid} · ${fmtDate(b.timeUpdated)}` : null].filter(Boolean).join(" · ");
}

/** Every installed branch: each in its own folder, one click to use another. */
function renderInstalls() {
  const box = $("installs");
  const installs = state.installs ?? [];
  box.hidden = installs.length < 2;
  if (box.hidden) return box.replaceChildren();
  box.replaceChildren(
    el("div", { class: "installs-title", text: t("files.installs.title") }),
    ...installs.map((i) => {
      const latest = latestFor(i.branch);
      const update = latest && buildNum(latest.buildid) > buildNum(i.buildid);
      const status = !i.fullyInstalled
        ? t("files.install.incomplete")
        : update
          ? t("files.install.update", { build: latest.buildid })
          : latest
            ? t("files.install.newest")
            : t("files.install.installed", { date: fmtDate(i.lastUpdated) });
      return el(
        "div",
        { class: `install${i.active ? " active" : ""}`, title: `${i.branch}: ${i.path}` },
        el("strong", { text: i.branch }),
        i.active ? el("span", { class: "chip chip-run", text: t("files.branch.inUse") }) : null,
        el("span", { class: `install-meta${update || !i.fullyInstalled ? " warn" : ""}`, text: t("files.install.meta", { build: i.buildid, status }) }),
        fitPath(el("span", { class: "path install-path", title: i.path }), i.path),
        el("button", { class: "icon-btn", title: t("common.openExplorer"), onclick: () => openFolder(i.path) }, icon("folder")),
        i.active
          ? null
          : el("button", { class: "btn btn-small btn-ghost", title: t("files.install.use.title", { branch: i.branch }), onclick: () => useBranch(i.branch) }, el("span", { text: t("files.install.use") })),
      );
    }),
  );
}

function renderMapsBranch() {
  const select = $("maps-branch");
  $("maps-branch-box").hidden = !multiBranch();
  const names = (state.installs ?? []).map((i) => i.branch);
  if ([...select.options].map((o) => o.value).join("|") !== names.join("|")) {
    select.replaceChildren(...names.map((name) => el("option", { value: name, text: name })));
  }
  select.value = state.branch;
  select.dispatchEvent(new Event("change"));
}

/** Start servers with another installed branch: nothing to download, it has its own folder. */
async function useBranch(branch) {
  if (!state || branch === state.branch) return;
  try {
    state = await invoke("set_branch", { branch });
    render();
    ctx.maps?.reload();
    toast(t("files.branchUsed", { branch }), "ok");
  } catch (err) {
    toast(String(err), "error");
    render();
  }
}

/** A branch that isn't installed: it goes into its own folder next to the others (setup). */
async function addBranch(branch) {
  if (busy) {
    toast(t("files.addBranch.busy"));
    return;
  }
  const current = state.installs?.find((i) => i.active)?.branch ?? state.server?.branch;
  const folder = await invoke("branch_folder", { branch }).catch(() => null);
  const ok = await confirmDialog({
    title: t("files.addBranch.title", { branch }),
    text: rich("files.addBranch.text", { branch, current, size: spaceTag("server", { free: folder?.freeSpace, path: folder?.path }) }),
    ok: t("files.addBranch.ok", { branch }),
  });
  if (ok) openSetup(null, { branch });
}

function renderBranches(select, value) {
  const names = state.branchCache ? state.branchCache.branches.map((b) => b.name) : [];
  for (const extra of [value, state.server?.branch, "public"]) {
    if (extra && !names.includes(extra)) names.push(extra);
  }
  const current = [...select.options].map((o) => o.value).join("|");
  if (current !== names.join("|")) {
    select.replaceChildren(...names.map((name) => el("option", { value: name, text: name })));
  }
  select.value = value;
  select.dispatchEvent(new Event("change"));
}

function renderFolders() {
  const installs = state.installs ?? [];
  $("folders").replaceChildren(
    ...state.folders.flatMap((f) => {
      if (f.key !== "server" || installs.length < 2) return [folderRow(f)];
      // Every installed branch has its own server folder (the one in use first).
      const rows = installs.map((i) => folderRow({ key: "server", label: t("folders.serverBranch", { branch: i.branch }), path: i.path, exists: true, branch: i.branch, active: i.active }));
      // The branch in use isn't installed yet: where it goes.
      if (!installs.some((i) => i.active)) rows.unshift(folderRow({ ...f, label: t("folders.serverBranch", { branch: state.branch }), branch: state.branch, active: true }));
      return rows;
    }),
  );
}

function folderRow(f) {
  const change = el("button", { class: "icon-btn", title: t("folders.change.title"), dataset: { busyLock: "1" }, onclick: () => changeFolder(f) }, icon("folder-edit"));
  const open = el("button", { class: "icon-btn", title: t("common.openExplorer"), disabled: f.exists ? null : true, onclick: () => openFolder(f.path) }, icon("folder"));
  const stateText =
    f.key === "maps" && f.exists ? t("folders.state.watching") : f.branch && f.active && f.exists ? t("files.branch.inUse") : f.exists ? t("folders.state.found") : t("folders.state.notCreated");
  return el(
    "div",
    { class: "folder" },
    el(
      "div",
      { class: "folder-top" },
      el("span", { class: "folder-label", text: f.label }),
      el("span", { class: `folder-state${f.exists ? " ok" : ""}`, text: stateText }),
      el("span", { class: "spacer" }),
      change,
      open,
    ),
    el("div", { class: "folder-path path-color", text: f.path }),
    // The active maps folder is watched (what DropLaunch says about it is in Settings).
    f.key === "maps" ? el("div", { class: "folder-watch" }, el("p", { class: "folder-note", text: t("folders.watchNote") })) : null,
  );
}

function renderMods() {
  const fw = $("mods-framework");
  fw.textContent = state.framework === "none" ? t("framework.vanilla") : state.oxideInactive ? t("framework.inactive", { name: FRAMEWORK_NAMES[state.framework] }) : FRAMEWORK_NAMES[state.framework];
  fw.className = state.framework === "none" ? "missing" : state.oxideInactive ? "warn" : "";
  const re = $("mods-rustedit");
  re.textContent = state.framework === "none" ? t("mods.rustedit.needs") : state.rustedit ? t("mods.rustedit.installed") : t("mods.rustedit.missing");
  re.className = state.rustedit ? "" : "missing";
  const owner = $("mods-owner");
  owner.textContent = state.owner ? state.owner.name : t("mods.owner.none");
  owner.title = state.owner ? t("mods.owner.title", { id: state.owner.steamId }) : t("mods.owner.unset");
  owner.className = state.owner ? "" : "missing";
  $("oxide-warning").hidden = !state.oxideInactive;
  const none = state.framework === "none";
  $("mods-playtest").checked = !none && Boolean(state.playtest);
  $("mods-playtest").disabled = none;
  $("mods-playtest-row").classList.toggle("disabled", none);
  $("mods-playtest-note").textContent = none ? t("mods.plugin.needs") : t("mods.plugin.note");
  $("mods-profiler-row").hidden = state.framework !== "carbon";
  $("mods-profiler").checked = Boolean(state.profiler);

  const rows = [];
  const scriptRow = (name, note, path) =>
    el(
      "div",
      { class: "script-row" },
      icon("file"),
      el("div", { class: "script-text" }, el("b", { text: name }), el("span", { text: note })),
      path ? el("button", { class: "icon-btn", title: t("common.showExplorer"), onclick: () => invoke("reveal", { path }).catch((e) => toast(String(e))) }, icon("folder")) : null,
    );
  if (state.updateScript) rows.push(scriptRow(fileName(state.updateScript), t("mods.script.update", { branch: state.server?.branch ?? state.branch }), state.updateScript));
  if (state.startFile) rows.push(scriptRow(fileName(state.startFile), t("mods.script.procgen"), state.startFile));
  // Two per map (normal + prefab test mode): count the maps.
  const mapFiles = state.startFiles.filter((f) => f.map);
  const maps = [...new Set(mapFiles.map((f) => f.map))];
  if (maps.length) {
    const first = mapFiles.find((f) => !f.prefab) ?? mapFiles[0];
    rows.push(
      scriptRow(tn("mods.script.maps", maps.length), t("mods.script.mapsNote", { maps: `${maps.slice(0, 3).join(", ")}${maps.length > 3 ? "…" : ""}` }), first.path),
    );
  }
  if (!rows.length) rows.push(el("p", { class: "scripts-empty", text: t("mods.scripts.empty") }));
  $("scripts").replaceChildren(...rows);
}

function applyBusy() {
  const running = runningHere();
  for (const id of ["btn-check", "branch-select", "btn-setup-options", "btn-open-setup"]) $(id).disabled = busy;
  $("btn-primary").disabled = busy || (running && !state?.setupNeeded);
  $("btn-verify").disabled = busy || !state?.server || running;
  $("btn-reapply-oxide").disabled = busy || running;
  const why = running ? t("files.stopFirst") : "";
  $("btn-primary").title = why;
  $("btn-verify").title = why || t("files.verify.title");
  $("btn-cancel").hidden = !busy || !$("setup").hidden;
  $("btn-primary").hidden = !(state && primaryAction()) || (busy && $("setup").hidden);
  $("live").hidden = !busy;
  document.querySelectorAll("[data-busy-lock]").forEach((b) => (b.disabled = busy));
  const start = $("btn-start");
  if (start) start.disabled = ctx.startBlocked();
}

// ---------------------------------------------------------------- log
/** SteamCMD's output, and DropLaunch's own lines in between. */
function appendLog(text, source = "app") {
  const log = $("log");
  $("log-empty")?.remove();
  const from = source === "steamcmd" ? "steamcmd" : "droplaunch";
  const row = el(
    "div",
    { class: `ln ${source}${/^Success!/.test(text) ? " ok" : ""} src-${from}` },
    el("span", { class: "t", text: new Date().toLocaleTimeString([], { hour12: false }) }),
    el("span", { class: "src", text: from }),
    el("span", { class: "m", text }),
  );
  log.append(row);
  while (log.childElementCount > MAX_LOG_LINES) log.firstElementChild.remove();
  logScroll.after();
}

// ---------------------------------------------------------------- main progress
function setPhase(phase) {
  const items = [...$("steps").children];
  if (phase === "failed") {
    const active = items.find((li) => li.classList.contains("active")) ?? items[0];
    active.classList.remove("active");
    active.classList.add("error");
    $("p-bar").classList.add("failed");
    $("p-bar").classList.remove("indeterminate");
    $("p-meta").textContent = "";
    return;
  }
  const idx = STEPS.indexOf(phase);
  items.forEach((li, i) => {
    li.classList.toggle("done", i < idx || phase === "done");
    li.classList.toggle("active", i === idx && phase !== "done");
    li.classList.remove("error");
  });
  if (phase === "done") {
    const bar = $("p-bar");
    bar.classList.remove("indeterminate");
    bar.classList.add("complete");
    $("p-fill").style.width = "100%";
    // SteamCMD's last progress tick is usually below 100%, so close it off here.
    $("p-text").textContent = t("files.step.done");
    $("p-pct").textContent = "100%";
    $("p-meta").textContent = "";
  }
}

function setStatus(text) {
  $("p-text").textContent = text;
  $("p-pct").textContent = "";
  $("p-meta").textContent = "";
  $("p-bar").classList.add("indeterminate");
  mainRate.reset();
  stripProgress(text, null);
}

function setProgress({ text, percent, done, total }) {
  $("p-text").textContent = text;
  $("p-pct").textContent = `${percent.toFixed(1)}%`;
  const bar = $("p-bar");
  bar.classList.remove("indeterminate");
  bar.setAttribute("aria-valuenow", String(Math.round(percent)));
  $("p-fill").style.width = `${Math.min(100, Math.max(0, percent))}%`;
  $("p-meta").textContent = mainRate.describe(text, done, total);
  stripProgress(text, percent);
}

function resetProgress(kind) {
  $("progress").hidden = false;
  $("result").hidden = true;
  $("steps").hidden = kind === "oxide";
  $("step-files-label").textContent = kind === "branches" ? t("files.step.branches") : t("files.step.files");
  [...$("steps").children].forEach((li) => li.classList.remove("done", "active", "error"));
  $("p-bar").classList.remove("failed", "complete");
  $("p-fill").style.width = "0%";
  setStatus({ branches: t("files.status.branches"), oxide: t("files.status.oxide") }[kind] ?? t("files.status.starting"));
  $("p-elapsed").textContent = "";
}

function showResult(kind, title, detail) {
  const box = $("result");
  box.className = `result fold-keep ${kind === "ok" ? "" : kind}`.trim();
  const name = { ok: "check", info: "download", neutral: "x" }[kind] ?? "alert";
  $("result-svg").replaceChildren(icon(name).firstChild);
  $("result-title").textContent = title;
  $("result-detail").textContent = detail ? ` ${detail}` : "";
  box.hidden = false;
}

// ---------------------------------------------------------------- main jobs
function onJobEvent(ev) {
  switch (ev.kind) {
    case "line":
      appendLog(ev.text, ev.source);
      break;
    case "phase":
      setPhase(ev.phase);
      break;
    case "status":
      setStatus(ev.text);
      break;
    case "progress":
      setProgress(ev);
      break;
  }
}

/** `branch`: the branch whose server files the job changes (null: none). */
async function runJob(kind, label, call, branch = null) {
  if (busy) return null;
  busy = true;
  jobBranch = branch;
  jobLabel = label;
  clearTimeout(stripResultTimer);
  stripResult = null;
  applyBusy();
  resetProgress(kind);
  renderStrip();
  jobStarted = Date.now();
  elapsedTimer = setInterval(() => {
    $("p-elapsed").textContent = t("files.elapsed", { time: fmtDuration((Date.now() - jobStarted) / 1000) });
  }, 500);
  const channel = new Channel();
  channel.onmessage = onJobEvent;
  try {
    return await call(channel);
  } catch (err) {
    setPhase("failed");
    showResult("failed", t("common.somethingWrong"), String(err));
    showStripResult({ icon: "alert", title: t("common.somethingWrong"), detail: String(err), kind: "warn" });
    appendLog(String(err), "error");
    return null;
  } finally {
    clearInterval(elapsedTimer);
    $("p-elapsed").textContent = t("files.took", { time: fmtDuration((Date.now() - jobStarted) / 1000) });
    busy = false;
    jobBranch = null;
    await refresh();
  }
}

function showSummary(summary) {
  const took = ` · ${t("files.took", { time: fmtDuration(summary.seconds) })}`;
  // Done, but something needs a look (Oxide couldn't be re-applied).
  const attention = Boolean(summary.attention);
  if (summary.ok) showResult(attention ? "failed" : "ok", summary.title, summary.detail + took);
  else if (summary.cancelled) showResult("neutral", summary.title, summary.detail);
  else showResult("failed", summary.title, summary.detail);
  const good = summary.ok && !attention;
  showStripResult({ icon: good ? "check" : summary.cancelled ? "x" : "alert", title: summary.title, detail: summary.detail, kind: good ? "done" : summary.cancelled ? "" : "warn" });
  // Keep the step view only when something failed; otherwise the banner says it all.
  $("progress").hidden = summary.ok || summary.cancelled;
}

async function startUpdate(validate) {
  const action = validate ? { validate: true } : primaryAction();
  if (!action) return;
  if (action.setup) return openSetup();
  if (action.confirm && !(await confirmDialog(action.confirm))) return;
  const label = action.validate
    ? t("files.job.verify")
    : state.server && state.server.branch !== state.branch
      ? t("files.job.switch", { branch: state.branch })
      : t("files.job.update");
  const summary = await runJob("update", label, (onEvent) => invoke("server_update", { validate: Boolean(action.validate), onEvent }), state.branch);
  if (summary) showSummary(summary);
}

/** Re-apply Oxide (from the warning, or before starting a server); true when it worked. */
async function reapplyOxide() {
  toast(t("files.status.oxide"));
  const summary = await runJob("oxide", t("files.job.oxide"), (onEvent) => invoke("reapply_oxide", { onEvent }), state.branch);
  if (!summary) return false;
  showSummary(summary);
  toast(summary.ok ? t("files.oxideDone") : `${summary.title}: ${summary.detail}`, summary.ok ? "ok" : "error");
  return summary.ok;
}

async function checkForUpdates() {
  const snap = await runJob("branches", t("files.job.check"), (onEvent) => invoke("fetch_branches", { onEvent }));
  if (!snap) return;
  state = snap;
  render();
  $("progress").hidden = true;
  const status = buildStatus();
  const latest = latestFor(state.branch);
  if (status.kind === "update") {
    showResult("info", t("files.check.newer", { build: status.latest.buildid }), t("files.check.newerDetail", { have: state.server.buildid, date: fmtDate(status.latest.timeUpdated) }));
  } else if (status.kind === "newest") {
    showResult("ok", t("files.check.newest"), t("files.check.newestDetail", { build: state.server.buildid, branch: state.server.branch }));
  } else if (latest) {
    showResult("info", t("files.check.latest", { branch: state.branch, build: latest.buildid }), t("files.check.released", { date: fmtDate(latest.timeUpdated) }));
  } else {
    showResult("info", t("files.check.listed"), tn("files.check.branches", state.branchCache.branches.length));
  }
}

// ---------------------------------------------------------------- one-click setup
/** opened: setup was needed when the window opened (its title stays "Let's get DropLaunch ready"). */
/** adding: a branch set up next to the installed ones; folder: where the chosen branch is or goes. */
const setupState = { framework: "carbon", running: false, startMap: null, live: false, opened: false, adding: null, folder: null, dropped: false };
/**
 * What the user wants of the options that need a mod framework (or Carbon): with None they're
 * unticked and greyed out, and back as they were once Carbon or Oxide is picked again.
 */
const wanted = { rustedit: true, playtest: false, profiler: false, whitelist: true };
const COUNTDOWN_MS = 5000;
/** Free space the server download needs (about 6 GB of files and a little room); the backend checks too. */
const SERVER_SPACE = 6.5 * 1024 ** 3;
/** SteamCMD: a 45 MB download that grows a little when it updates itself. */
const STEAMCMD_SPACE = 256 * 1024 ** 2;

/**
 * A download's size in green when its drive has room for it, red when it doesn't. `free`/`path`:
 * another folder's (a branch being added), else the SteamCMD or server folder's.
 */
function spaceTag(kind, { free, path } = {}) {
  const steamcmd = kind === "steamcmd";
  return sizeTag(steamcmd ? t("units.steamcmdSize") : t("units.serverSize"), {
    free: free !== undefined ? free : steamcmd ? state?.steamcmdFreeSpace : state?.freeSpace,
    need: steamcmd ? STEAMCMD_SPACE : SERVER_SPACE,
    path: path ?? state?.folders.find((f) => f.key === kind)?.path,
  });
}
let countdownTimer = null;

/**
 * @param {object|null} pendingMap a dropped map to start once setup is done
 * @param {{auto?: boolean, branch?: string}} opts auto: start right away (a map dropped on a
 *   fresh install); branch: set up this branch next to the installed ones
 */
async function openSetup(pendingMap = null, { auto = false, branch = null } = {}) {
  if (busy) return;
  const needed = state.setupNeeded;
  setupState.opened = needed;
  setupState.adding = branch;
  // A map dropped on a folder without the server files: setting up is how it gets started.
  setupState.dropped = auto && Boolean(pendingMap);
  setupState.startMap = pendingMap && !pendingMap.procgen ? pendingMap : null;
  $("setup-fix-label").textContent = branch ? t("setup.install", { branch }) : needed ? t("setup.fix") : t("setup.apply");
  $("setup-later").textContent = branch ? t("common.cancel") : needed ? t("setup.later") : t("common.close");
  renderBranches($("setup-branch"), branch ?? state.server?.branch ?? state.branch);
  await loadSetupFolder();

  // A new branch gets the same mods as the one in use.
  setupState.framework = state.framework !== "none" ? state.framework : needed ? "carbon" : "none";
  wanted.rustedit = needed || state.rustedit;
  wanted.playtest = Boolean(state.playtest);
  wanted.profiler = Boolean(state.profiler);
  // On by default the first time; afterwards as it's set.
  wanted.whitelist = needed ? true : Boolean(state.whitelist);
  // On by default the first time; afterwards as it's set.
  $("setup-backup").checked = needed || state.backupClientMaps !== false;
  $("setup-prefab").checked = false;
  // Not over a server that runs (it would have to stop first).
  $("setup-start").checked = !(branch && ctx.server?.isLive());
  syncPrefabOption();
  const startLabel = $("setup-start-label");
  $("setup-prefab-row").hidden = !setupState.startMap;
  if (setupState.startMap) {
    startLabel.replaceChildren(...rich("setup.start.map", { map: setupState.startMap.name }));
  } else {
    startLabel.replaceChildren(el("span", { text: t("setup.start") }), " ", el("em", { text: t("setup.start.note") }));
  }
  renderFrameworkPicker();
  const accounts = renderOwner();

  showSetupView("options");
  $("setup").hidden = false;
  applyBusy();
  $("setup-fix").focus();
  if (auto) {
    // A map was dropped on a fresh install: no questions, the options stay open while it downloads.
    await accounts;
    runSetup();
  }
}

/** Where the branch picked in the setup window is installed, or goes (each has its own folder). */
async function loadSetupFolder() {
  const branch = $("setup-branch").value || state.branch;
  try {
    setupState.folder = { branch, ...(await invoke("branch_folder", { branch })) };
  } catch {
    setupState.folder = null;
  }
  renderSetupFolders();
}

/** What's there and where it goes. The folders can be changed here (a click on one). */
function renderSetupFolders() {
  const needed = state.setupNeeded;
  const adding = setupState.adding;
  const current = state.installs?.find((i) => i.active)?.branch;
  $("setup-title").textContent = adding
    ? t("setup.title.add", { branch: adding })
    : setupState.dropped
      ? t("setup.title.dropped")
      : setupState.opened
        ? t("setup.title.ready")
        : t("setup.title.options");
  $("setup-lead").textContent = adding
    ? current
      ? t("setup.lead.add", { branch: adding, current })
      : t("setup.lead.addFirst", { branch: adding })
    : needed
      ? t("setup.lead.needed")
      : setupState.opened
        ? t("setup.lead.both")
        : t("setup.lead.options");
  const folder = (key) => state.folders.find((f) => f.key === key);
  const steamcmd = folder("steamcmd");
  // The branch picked here: its own folder (the active one's when nothing else was picked).
  const picked = setupState.folder;
  const server = picked ? { key: "server", label: t("folders.server"), path: picked.path, exists: picked.exists, branch: picked.branch } : folder("server");
  const installed = picked ? picked.installed : state.server;
  // What's on the disk (after Reset everything it isn't linked yet, but setup finds it).
  const steamcmdHere = state.steamcmdFound ?? state.steamcmdInstalled;
  setReq("steamcmd", steamcmdHere, [...(steamcmdHere ? [t("setup.req.found")] : rich("setup.req.download", { size: spaceTag("steamcmd") })), folderButton(steamcmd)]);
  const others = (state.installs ?? []).filter((i) => i.branch !== picked?.branch).map((i) => i.branch);
  // Green or red: room on the drive it goes to.
  const size = spaceTag("server", { free: picked ? picked.freeSpace : undefined, path: server.path });
  setReq("server", Boolean(installed), [
    ...(installed
      ? [t("setup.req.foundBuild", { build: installed.buildid, branch: installed.branch })]
      : others.length
        ? rich("setup.req.downloadNext", { branch: picked?.branch ?? t("setup.req.thisBranch"), others: fmtList(others), size })
        : rich("setup.req.download", { size })),
    folderButton(server),
  ]);
}

/** Room for the server download? Asked before anything is downloaded; false: not enough. */
async function roomForServer() {
  const branch = $("setup-branch").value || state.branch;
  if (setupState.folder ? setupState.folder.installed : state.server) return true;
  const short = await invoke("server_space", { branch }).catch(() => null);
  if (!short) return true;
  const choice = await confirmDialog({
    title: t("setup.space.title"),
    text: rich("setup.space.text", {
      size: el("span", { class: "size-tag no-room", text: t("units.serverSize") }),
      needed: fmtBytes(short.needed),
      drive: short.drive,
      free: fmtBytes(short.free),
    }),
    choices: [{ label: t("setup.space.pick"), value: "pick", primary: true }],
  });
  if (choice === "pick") {
    const picked = setupState.folder;
    const server = picked ? { key: "server", path: picked.path, exists: picked.exists, branch: picked.branch } : state.folders.find((f) => f.key === "server");
    if (server) await pickSetupFolder(server);
  }
  return false;
}

/** A folder in the setup window: click to pick another one. */
function folderButton(f) {
  return el(
    "button",
    {
      type: "button",
      class: "path path-btn",
      title: t(f.key === "steamcmd" ? "setup.folder.title.steamcmd" : "setup.folder.title.server", { path: f.path }),
      onclick: () => pickSetupFolder(f),
    },
    // One line: a long path gives way in the middle, the folder's own name stays in view.
    fitPath(el("span", { class: "path-text" }), f.path, { box: (node) => node.closest(".path-btn")?.parentElement }),
    icon("folder-edit"),
  );
}

async function pickSetupFolder(f) {
  if (setupState.running || busy) return;
  try {
    const title = f.key === "steamcmd" ? t("setup.pick.steamcmd") : f.branch ? t("setup.pick.serverBranch", { branch: f.branch }) : t("setup.pick.server");
    const path = await invoke("pick_folder", { title, start: f.exists ? f.path : state.baseDir });
    if (!path) return;
    state = await invoke("set_folder", { key: f.key, path, branch: f.key === "server" ? f.branch ?? null : null });
    render();
    await loadSetupFolder();
    showSetupView("options");
  } catch (err) {
    toast(String(err), "error");
  }
}

/** Also while it runs: it carries on in the background (the Maps page shows how far it is). */
function closeSetup() {
  stopCountdown();
  $("setup").hidden = true;
  applyBusy();
  renderStrip();
}

/** Back to the setup window (from the Maps page, while it runs in the background). */
function showSetup() {
  $("setup").hidden = false;
  applyBusy();
  renderStrip();
}

/** After a good setup the window closes by itself: the Maps page shows the server starting. */
function startCountdown() {
  stopCountdown();
  const button = $("setup-close");
  button.classList.remove("counting");
  void button.offsetWidth; // restart the animation
  button.classList.add("counting");
  countdownTimer = setTimeout(closeSetup, COUNTDOWN_MS);
}

function stopCountdown() {
  clearTimeout(countdownTimer);
  countdownTimer = null;
  $("setup-close").classList.remove("counting");
}

// ---------------------------------------------------------------- server owner (setup window)
/** Steam accounts that signed in on this PC (once Detect my Steam IDs was pressed), signed-in first. */
let steamAccounts = [];
/** Steam64 ID -> picture URL (from Steam's own cache on this PC). */
const avatars = new Map();

/**
 * The owner box. Nothing is filled in: Steam accounts are only looked up when asked (Detect my
 * Steam IDs), and an owner saved earlier is only named (left empty, it stays the owner).
 */
function renderOwner() {
  $("setup-owner").checked = Boolean(state.owner);
  $("setup-owner-id").value = "";
  $("setup-owner-id").classList.remove("bad");
  ownerChanged();
  return Promise.resolve();
}

async function detectAccounts() {
  const button = $("setup-owner-detect");
  button.disabled = true;
  try {
    steamAccounts = await invoke("steam_accounts");
    await Promise.all(
      steamAccounts.map(async (a) => {
        if (avatars.has(a.steamId)) return;
        try {
          avatars.set(a.steamId, URL.createObjectURL(new Blob([await invoke("steam_avatar", { steamId: a.steamId })], { type: "image/png" })));
        } catch {
          avatars.set(a.steamId, null);
        }
      }),
    );
    // Nothing entered yet: the owner saved earlier when it's one of them, else the account
    // signed in to Steam right now.
    if (steamAccounts.length && !$("setup-owner-id").value.trim()) {
      const saved = steamAccounts.find((a) => a.steamId === state.owner?.steamId);
      $("setup-owner-id").value = (saved ?? steamAccounts[0]).steamId;
    }
    ownerEdited();
  } catch (err) {
    toast(String(err), "error");
  } finally {
    button.disabled = false;
  }
}

function ownerName(id) {
  return steamAccounts.find((a) => a.steamId === id)?.name ?? (state.owner?.steamId === id ? state.owner.name : "");
}

/** The accounts found on this PC, as cards to pick from. */
function renderAccounts(id) {
  const box = $("setup-owner-accounts");
  box.hidden = !steamAccounts.length;
  box.replaceChildren(
    ...steamAccounts.map((a) => {
      const picture = avatars.get(a.steamId);
      return el(
        "button",
        {
          type: "button",
          class: `account${a.steamId === id ? " picked" : ""}`,
          "aria-pressed": String(a.steamId === id),
          onclick: () => {
            $("setup-owner-id").value = a.steamId;
            ownerEdited();
          },
        },
        picture ? el("img", { class: "account-pic", src: picture, alt: "" }) : el("span", { class: "account-pic", text: (a.name.trim()[0] ?? "?").toUpperCase() }),
        el("span", { class: "account-text" }, el("b", { text: a.name }), el("code", { text: a.steamId })),
        a.active ? el("span", { class: "account-badge", text: t("owner.signedIn") }) : null,
        el("span", { class: "account-check" }, icon("check")),
      );
    }),
  );
}

function ownerChanged() {
  const on = $("setup-owner").checked;
  $("setup-owner-field").hidden = !on;
  renderWhitelistOption();
  const input = $("setup-owner-id");
  input.classList.remove("bad");
  const id = input.value.trim();
  renderAccounts(id);
  const hint = $("setup-owner-hint");
  hint.classList.remove("bad");
  const local = steamAccounts.find((a) => a.steamId === id);
  let text = [];
  if (local) text = [];
  else if (id && ownerName(id)) text = [t("owner.current", { name: ownerName(id) })];
  else if (id) text = [t("owner.byHand")];
  else if (state.owner) text = rich("owner.saved", { name: state.owner.name || t("owner.savedEarlier") });
  else text = [t("owner.paste")];
  hint.replaceChildren(...text);
  hint.hidden = !text.length;
}

function ownerProblem(text) {
  $("setup-owner-id").classList.add("bad");
  const hint = $("setup-owner-hint");
  hint.textContent = text;
  hint.classList.add("bad");
  $("setup-owner-id").focus();
}

/** While setup downloads, owner changes are saved as they're made (the job reads them later). */
let ownerTimer = null;
function ownerEdited() {
  ownerChanged();
  if (!setupState.running) return;
  clearTimeout(ownerTimer);
  ownerTimer = setTimeout(async () => {
    const on = $("setup-owner").checked;
    const id = $("setup-owner-id").value.trim();
    if (on && !id) return;
    try {
      state = await invoke("set_owner", { steamId: on ? id : null, name: on ? ownerName(id) : null });
    } catch (err) {
      ownerProblem(String(err));
    }
  }, 500);
}

/** Save the owner choice; false (with the reason shown) when the Steam64 ID isn't valid. */
async function saveOwner() {
  const on = $("setup-owner").checked;
  const id = $("setup-owner-id").value.trim();
  if (!on && !state.owner) return true;
  // Left empty: the owner saved earlier stays.
  if (on && !id && state.owner) return true;
  if (on && !id) {
    ownerProblem(t("owner.enter"));
    return false;
  }
  try {
    state = await invoke("set_owner", { steamId: on ? id : null, name: on ? ownerName(id) : null });
    return true;
  } catch (err) {
    ownerProblem(String(err));
    return false;
  }
}

function setReq(key, ok, note) {
  const li = document.querySelector(`#setup-reqs [data-req="${key}"]`);
  li.className = ok ? "ok" : "missing";
  li.querySelector(".req-note").replaceChildren(...note.map((n) => (typeof n === "string" ? document.createTextNode(n) : n)));
}

function renderFrameworkPicker() {
  for (const b of $("setup-framework").querySelectorAll("button")) {
    b.setAttribute("aria-checked", String(b.dataset.value === setupState.framework));
  }
  const none = setupState.framework === "none";
  // Without a framework they can't work: unticked, and ticked again as wanted with Carbon or Oxide.
  for (const key of ["rustedit", "playtest"]) {
    $(`setup-${key}`).disabled = none;
    $(`setup-${key}`).checked = !none && wanted[key];
    $(`setup-${key}-row`).classList.toggle("disabled", none);
  }
  const carbon = setupState.framework === "carbon";
  $("setup-profiler-row").hidden = !carbon;
  $("setup-profiler").checked = carbon && wanted.profiler;
  renderWhitelistOption();
}

/** The whitelist lets the owner in, and the DropLaunch plugin checks it: both are needed. */
function renderWhitelistOption() {
  const framework = (setupState.framework ?? "none") !== "none";
  const owner = $("setup-owner").checked;
  const usable = framework && owner;
  $("setup-whitelist").disabled = !usable;
  $("setup-whitelist").checked = usable && wanted.whitelist !== false;
  $("setup-whitelist-row").classList.toggle("disabled", !usable);
  $("setup-whitelist-note").textContent = !framework ? t("setup.whitelist.needsFramework") : !owner ? t("setup.whitelist.needsOwner") : t("setup.whitelist.note");
}

/** options -> running -> result. While SteamCMD downloads ("live"), the options stay open. */
function showSetupView(view) {
  const running = view === "running";
  const live = running && setupState.live;
  $("setup-reqs").hidden = view !== "options";
  $("setup-options").hidden = !(view === "options" || live);
  $("setup-live-note").hidden = !live;
  $("setup-branch-row").hidden = running;
  $("setup-tasks").hidden = view === "options";
  $("setup-tasks").classList.toggle("compact", live);
  $("setup-log").hidden = !running || live;
  $("setup-result").hidden = view !== "result";
  // While it runs it can be hidden: a server from another branch can be used meanwhile.
  if (running) $("setup-later").textContent = t("setup.hide");
  $("setup-later").title = running ? t("setup.hide.title") : "";
  $("setup-cancel").hidden = !running;
  $("setup-fix").hidden = running;
  $("setup-close").hidden = true;
  $("setup-actions").classList.remove("centered");
}

/** What the setup window says right now. */
function setupOptions() {
  setupState.framework = $("setup-framework").querySelector('[aria-checked="true"]')?.dataset.value ?? setupState.framework;
  return {
    branch: $("setup-branch").value,
    framework: setupState.framework,
    rustedit: setupState.framework !== "none" && $("setup-rustedit").checked,
    startServer: $("setup-start").checked,
    startMap: setupState.startMap?.path ?? null,
    playtest: setupState.framework !== "none" && $("setup-playtest").checked,
    profiler: setupState.framework === "carbon" && $("setup-profiler").checked,
    prefab: Boolean(setupState.startMap) && $("setup-prefab").checked,
    whitelist: !$("setup-whitelist").disabled && $("setup-whitelist").checked,
    backupMaps: $("setup-backup").checked,
  };
}

function setupTaskTitles(options) {
  const frameworkTitle = { none: t("common.modFramework"), carbon: FRAMEWORK_NAMES.carbon, oxide: FRAMEWORK_NAMES.oxide }[options.framework];
  $("task-framework-title").textContent = options.rustedit ? t("setup.task.frameworkRustedit", { framework: frameworkTitle }) : frameworkTitle;
  $("task-launch-title").textContent = setupState.startMap ? t("setup.task.launchMap", { map: setupState.startMap.name }) : t("setup.task.launch");
}

/** Options changed while SteamCMD downloads: the job uses them when it gets there. */
function pushLiveOptions() {
  if (!setupState.running || !setupState.live) return;
  const options = setupOptions();
  setupTaskTitles(options);
  invoke("update_setup_options", { options }).catch(() => {});
}

let activeTask = null;

function taskRow(id) {
  return document.querySelector(`#setup-tasks [data-task="${id}"]`);
}

function setTask(id, taskState, detail) {
  const li = taskRow(id);
  if (!li) return;
  li.className = taskState;
  li.querySelector(".task-detail").textContent = detail || (taskState === "active" ? t("setup.working") : "");
  if (taskState === "active") {
    activeTask = id;
    setupRate.reset();
  } else if (activeTask === id) {
    activeTask = null;
  }
}

function setTaskProgress(text, percent, meta) {
  const li = activeTask && taskRow(activeTask);
  if (!li) return;
  li.querySelector(".task-detail").textContent = [percent != null ? `${text} · ${percent.toFixed(1)}%` : text, meta].filter(Boolean).join("  ·  ");
  li.classList.toggle("measured", percent != null);
  if (percent != null) li.querySelector(".task-bar div").style.width = `${Math.min(100, percent)}%`;
}

function pushSetupLog(text, source) {
  const box = $("setup-log");
  box.append(el("div", { text, style: source === "error" ? "color: var(--danger)" : null }));
  while (box.childElementCount > 5) box.firstElementChild.remove();
}

function onSetupEvent(ev) {
  switch (ev.kind) {
    case "task":
      // Past the download: the options are used now, so they close.
      if (setupState.live && ev.task !== "steamcmd" && ev.task !== "server") {
        setupState.live = false;
        showSetupView("running");
      }
      setTask(ev.task, ev.state, ev.detail);
      break;
    case "status":
      setTaskProgress(ev.text, null, "");
      stripProgress(ev.text, null);
      break;
    case "progress":
      setTaskProgress(ev.text, ev.percent, setupRate.describe(ev.text, ev.done, ev.total));
      stripProgress(ev.text, ev.percent);
      break;
    case "line":
      appendLog(ev.text, ev.source);
      pushSetupLog(ev.text, ev.source);
      break;
  }
}

async function runSetup() {
  if (busy || $("setup").hidden) return;
  // A typo in the Steam64 ID stops here, before anything is downloaded.
  if (!(await saveOwner())) return;
  if (!(await roomForServer())) return;
  // What's highlighted is what runs.
  const options = setupOptions();
  setupTaskTitles(options);
  document.querySelectorAll("#setup-tasks li").forEach((li) => {
    li.className = "";
    li.querySelector(".task-detail").textContent = "";
  });
  $("setup-log").replaceChildren();
  const map = setupState.startMap && options.startServer ? setupState.startMap.name : null;
  // The branch being set up (it has its own folder): is it there already?
  const serverHere = setupState.folder ? Boolean(setupState.folder.installed) : Boolean(state.server);
  $("setup-lead").textContent = serverHere
    ? t("setup.lead.quick")
    : setupState.adding
      ? map
        ? t("setup.lead.addingMap", { branch: options.branch, map })
        : t("setup.lead.adding", { branch: options.branch })
      : map
        ? t("setup.lead.runningMap", { map })
        : t("setup.lead.running");
  setupState.running = true;
  // Something to download: the options stay open meanwhile.
  setupState.live = !(state.steamcmdFound ?? state.steamcmdInstalled) || !serverHere;
  showSetupView("running");
  busy = true;
  // Servers keep starting from the branch in use while another one is set up.
  jobBranch = options.branch;
  jobLabel = setupState.adding ? t("setup.job.installing", { branch: options.branch }) : t("setup.job.setup");
  stripProgress(jobLabel, null);
  applyBusy();
  const framework = FRAMEWORK_NAMES[options.framework];
  appendLog(t("setup.log.started", { branch: options.branch, framework: options.rustedit ? t("framework.withRustedit", { framework }) : framework }), "app");

  const channel = new Channel();
  channel.onmessage = onSetupEvent;
  let summary;
  try {
    summary = await invoke("setup_all", { options, onEvent: channel });
  } catch (err) {
    summary = { ok: false, cancelled: false, title: t("setup.failed"), detail: String(err), seconds: 0 };
  }
  setupState.running = false;
  setupState.live = false;
  clearTimeout(ownerTimer);
  busy = false;
  jobBranch = null;
  // The branch in use only changes once setup worked (the backend does that).
  await refresh();
  ctx.maps.reload();
  // It ran in the background: say how it went on the Maps page.
  const background = $("setup").hidden;
  if (background) {
    const added = setupState.adding && summary.ok ? t("setup.added", { branch: options.branch }) : null;
    showStripResult({
      icon: summary.ok ? "check" : summary.cancelled ? "x" : "alert",
      title: summary.ok ? summary.title : summary.cancelled ? t("setup.cancelled") : t("setup.failed"),
      detail: added ?? summary.detail,
      kind: summary.ok ? "done" : summary.cancelled ? "" : "warn",
    });
    if (added) toast(added, "ok");
  }

  $("setup-lead").textContent = summary.ok ? t("setup.lead.done") : summary.cancelled ? t("setup.lead.stopped") : t("setup.lead.failed");
  const box = $("setup-result");
  const kind = summary.ok ? "" : summary.cancelled ? "neutral" : "failed";
  box.className = `setup-result ${kind}`.trim();
  $("setup-result-svg").replaceChildren(icon(summary.ok ? "check" : summary.cancelled ? "x" : "alert").firstChild);
  $("setup-result-title").textContent = summary.ok ? summary.title : summary.cancelled ? t("setup.cancelled") : t("setup.failed");
  $("setup-result-detail").textContent = summary.ok ? summary.detail : summary.cancelled ? t("setup.cancelled.detail") : summary.detail;

  showSetupView("result");
  $("setup-fix").hidden = summary.ok;
  $("setup-fix-label").textContent = t("setup.tryAgain");
  $("setup-later").hidden = summary.ok;
  $("setup-later").textContent = t("common.close");
  $("setup-close").hidden = !summary.ok;
  (summary.ok ? $("setup-close") : $("setup-fix")).focus();
  if (summary.ok) {
    setupState.startMap = null;
    $("setup-actions").classList.add("centered");
    startCountdown();
  }
}

// ---------------------------------------------------------------- folders
async function changeFolder(folder) {
  try {
    const path = await invoke("pick_folder", { title: t("folders.pick", { folder: folder.label }), start: folder.path });
    if (!path) return;
    state = await invoke("set_folder", { key: folder.key, path, branch: folder.branch ?? null });
    render();
    if (folder.key === "maps") ctx.maps.reload();
  } catch (err) {
    toast(String(err), "error");
  }
}

async function openFolder(path) {
  try {
    await invoke("open_folder", { path });
  } catch (err) {
    toast(String(err), "error");
  }
}

// ---------------------------------------------------------------- wiring
new Dropdown($("branch-select"), { label: t("common.branch"), meta: branchMeta, title: (v) => (latestFor(v)?.description ? latestFor(v).description : t("files.branch.title")) });
new Dropdown($("setup-branch"), { meta: branchMeta });
new Dropdown($("maps-branch"), { label: t("common.branch"), meta: branchMeta, title: () => t("maps.branch.title") });
$("setup-branch").addEventListener("change", (e) => {
  if (!e.isTrusted && !e.bubbles) return;
  if (!$("setup").hidden && !setupState.running) loadSetupFolder();
});

$("btn-primary").addEventListener("click", () => startUpdate(false));
$("btn-verify").addEventListener("click", () => startUpdate(true));
$("btn-check").addEventListener("click", checkForUpdates);
$("btn-cancel").addEventListener("click", cancelJob);
$("strip-close").addEventListener("click", () => {
  clearTimeout(stripResultTimer);
  stripResult = null;
  renderStrip();
});
$("btn-view").addEventListener("click", () => setExpanded(!expanded));
$("branch-select").addEventListener("change", (e) => {
  if (!e.isTrusted && !e.bubbles) return; // our own re-render
  if (!state || e.target.value === state.branch) return;
  pickBranch(e.target.value);
});
$("maps-branch").addEventListener("change", (e) => {
  if (!e.isTrusted && !e.bubbles) return;
  if (!state || e.target.value === state.branch) return;
  pickBranch(e.target.value);
});

/** A branch picked in a dropdown: an installed one is used right away, a new one is set up. */
function pickBranch(branch) {
  if (installedBranch(branch) || !state.installs?.length) useBranch(branch);
  else {
    render(); // the dropdown shows the branch in use until the new one is there
    addBranch(branch);
  }
}
$("btn-open-setup").addEventListener("click", () => openSetup());
$("btn-setup-options").addEventListener("click", () => openSetup());
$("btn-reapply-oxide").addEventListener("click", reapplyOxide);
$("btn-clear-log").addEventListener("click", () => $("log").replaceChildren());
$("btn-copy-log").addEventListener("click", () => copyText([...$("log").querySelectorAll(".ln")].map((r) => r.textContent).join("\n"), t("files.log.what")));
for (const tab of document.querySelectorAll(".tabs [role=tab]")) tab.addEventListener("click", () => setTab(tab.dataset.tab));
$("pill-steamcmd").addEventListener("click", showFiles);
$("pill-server").addEventListener("click", showFiles);

$("setup-framework").addEventListener("click", (e) => {
  const button = e.target.closest("button[data-value]");
  if (!button) return;
  setupState.framework = button.dataset.value;
  renderFrameworkPicker();
  pushLiveOptions();
});
for (const key of ["rustedit", "playtest", "profiler"]) {
  $(`setup-${key}`).addEventListener("change", (e) => {
    wanted[key] = e.target.checked;
    pushLiveOptions();
  });
}
/** Prefab test mode is how the map starts: only when it starts. */
function syncPrefabOption() {
  const off = !$("setup-start").checked;
  $("setup-prefab").disabled = off;
  $("setup-prefab-row").classList.toggle("disabled", off);
}
$("setup-start").addEventListener("change", () => {
  syncPrefabOption();
  pushLiveOptions();
});
$("setup-whitelist").addEventListener("change", (e) => {
  wanted.whitelist = e.target.checked;
  pushLiveOptions();
});
for (const id of ["setup-backup", "setup-prefab"]) $(id).addEventListener("change", pushLiveOptions);
// Links in the UI open in the browser.
document.addEventListener("click", (e) => {
  const link = e.target.closest("a.ext-link");
  if (!link) return;
  e.preventDefault();
  invoke("open_link", { url: link.href }).catch((err) => toast(String(err), "error"));
});
$("setup-fix").addEventListener("click", runSetup);
$("mods-playtest").addEventListener("change", (e) => ctx.setOption({ playtest: e.target.checked }));
$("mods-profiler").addEventListener("change", async (e) => {
  await ctx.setOption({ profiler: e.target.checked });
  if (runningHere()) toast(e.target.checked ? t("mods.profiler.on") : t("mods.profiler.off"));
});

// Cards that fold (the Server files page); remembered. Saved: the cards not as they start
// (folded with data-start-folded, open otherwise).
const FOLDED_KEY = "droplaunch.foldedCards";
function foldedCards() {
  try {
    return new Set(JSON.parse(localStorage.getItem(FOLDED_KEY) ?? "[]"));
  } catch {
    return new Set();
  }
}
function setFolded(card, folded) {
  card.classList.toggle("folded", folded);
  const button = card.querySelector(".fold-btn");
  button.setAttribute("aria-expanded", String(!folded));
  button.title = folded ? t("common.unfold") : t("common.fold");
}
for (const card of document.querySelectorAll(".card.foldable")) {
  const startsFolded = card.hasAttribute("data-start-folded");
  setFolded(card, startsFolded !== foldedCards().has(card.id));
  card.querySelector(".fold-btn").addEventListener("click", () => {
    const folded = foldedCards();
    const now = !card.classList.contains("folded");
    if (now !== startsFolded) folded.add(card.id);
    else folded.delete(card.id);
    try {
      localStorage.setItem(FOLDED_KEY, JSON.stringify([...folded]));
    } catch {}
    setFolded(card, now);
  });
}
$("setup-owner").addEventListener("change", () => {
  ownerEdited();
  if ($("setup-owner").checked && !$("setup-owner-id").value) $("setup-owner-id").focus();
});
$("setup-owner-id").addEventListener("input", ownerEdited);
$("setup-owner-detect").addEventListener("click", detectAccounts);
$("setup-later").addEventListener("click", closeSetup);

// ---------------------------------------------------------------- first start
/** The welcome's downloads: SteamCMD (unless it's here already) and the server files, their size green or red. */
function renderWelcome() {
  const found = state.steamcmdFound ?? state.steamcmdInstalled;
  $("welcome-steamcmd").replaceChildren(icon(found ? "check" : "download"), el("span", { text: found ? t("welcome.steamcmdHere") : t("welcome.steamcmd") }));
  $("welcome-server").replaceChildren(
    ...((state.serverFound ?? state.server) ? [icon("check"), el("span", { text: t("welcome.serverHere") })] : [icon("download"), el("span", {}, rich("welcome.server", { size: spaceTag("server") }))]),
  );
  // The folders it's going to make (or made empty, like Maps on its first start): their names
  // show on hover.
  const making = state.folders.filter((f) => !f.exists || f.empty);
  const folders = el("span", { class: "size-tag room", title: making.map((f) => fileName(f.path)).join("\n"), text: tn("welcome.folders.count", making.length) });
  $("welcome-folders").hidden = !making.length;
  $("welcome-folders").replaceChildren(icon("folder"), el("span", {}, rich("welcome.folders", { folders })));
  // A drive's root or a busy folder (Desktop, Downloads…): better in a folder of its own.
  const places = {
    drive: () => t("welcome.place.drive", { drive: driveOf(state.baseDir) ?? state.baseDir }),
    desktop: () => t("welcome.place.desktop"),
    downloads: () => t("welcome.place.downloads"),
    documents: () => t("welcome.place.documents"),
    home: () => t("welcome.place.home"),
  };
  const place = places[state.busyPlace]?.();
  $("welcome-place").hidden = !place || !making.length;
  $("welcome-place").replaceChildren(icon("alert"), el("span", { text: place ? t("welcome.place", { place }) : "" }));
}

/** Set up on the welcome screen ("Show me around": the guide is on from the start). After a reset
 * with everything installed, the setup window has the options (owner, mods, whitelist…). */
async function welcomeDone() {
  $("welcome").hidden = true;
  if ($("welcome-guide").checked) await ctx.guide.setOn(true);
  await ctx.setOption({ welcomed: true });
  openSetup();
}
$("welcome-host").addEventListener("click", welcomeDone);

/** Clicked next to the welcome (or Esc): skipping is possible, but it says what that means. */
async function askSkipWelcome() {
  // Everything's installed (the settings were reset): nothing to warn about.
  if (!state.setupNeeded) {
    $("welcome").hidden = true;
    await ctx.setOption({ welcomed: true });
    return;
  }
  const choices = [
    { label: t("welcome.skip.ok"), value: "skip" },
    { label: t("welcome.go"), value: "setup", primary: true },
  ];
  const answer = await confirmDialog({ title: t("welcome.skip.title"), text: t("welcome.skip.text"), choices, cancel: false });
  if (answer === "setup") return welcomeDone();
  if (answer !== "skip") return;
  $("welcome").hidden = true;
  await ctx.setOption({ welcomed: true });
  toast(t("welcome.skip.later"));
}
$("welcome").addEventListener("click", (e) => e.target === $("welcome") && askSkipWelcome());
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && !$("welcome").hidden && $("modal").hidden) askSkipWelcome();
});
$("setup-close").addEventListener("click", closeSetup);
$("setup-cancel").addEventListener("click", () => {
  $("setup-cancel").disabled = true;
  invoke("cancel_job").finally(() => ($("setup-cancel").disabled = false));
});

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && !$("setup").hidden && $("modal").hidden) closeSetup();
});
// Things change outside DropLaunch too (update_server.bat, files copied in): look again on focus.
window.addEventListener("focus", () => {
  if (!busy && state) refresh();
});

/** A small screen: the window opens no bigger than the screen's free area (and centred on it). */
async function fitToScreen() {
  const api = window.__TAURI__;
  const win = api?.window?.getCurrentWindow?.();
  if (!win || !api.dpi) return;
  try {
    const monitor = await api.window.currentMonitor();
    if (!monitor) return;
    const area = monitor.workArea ?? { position: monitor.position, size: monitor.size };
    const outer = await win.outerSize();
    const inner = await win.innerSize();
    const width = Math.min(outer.width, area.size.width);
    const height = Math.min(outer.height, area.size.height);
    if (width === outer.width && height === outer.height) return;
    const { PhysicalSize, PhysicalPosition } = api.dpi;
    await win.setSize(new PhysicalSize(width - (outer.width - inner.width), height - (outer.height - inner.height)));
    await win.setPosition(new PhysicalPosition(area.position.x + Math.round((area.size.width - width) / 2), area.position.y + Math.round((area.size.height - height) / 2)));
  } catch (err) {
    console.warn("Could not fit the window to the screen", err);
  }
}

// ---------------------------------------------------------------- start
fitToScreen();
ctx.layout = initLayout(ctx);
ctx.maps = initMaps(ctx);
ctx.server = initServer(ctx);
ctx.game = initGame(ctx);
ctx.updates = initUpdates(ctx);
ctx.about = initAbout(ctx);
ctx.playtest = initPlaytest(ctx);
ctx.mapViewer = initMapViewer(ctx);
ctx.guide = initGuide(ctx);
ctx.settings = initSettings(ctx);
ctx.kits = initKits(ctx);
// Always opens in the simple view on the Maps page (the window starts at its size); setup takes
// over when something's missing.
setExpanded(false, { resize: false });

refresh().then(async () => {
  if (!state) return;
  if (state.configWarning) toast(state.configWarning);
  const files = await invoke("take_pending_files").catch(() => []);
  await Promise.all([ctx.server.load(), ctx.maps.reload(), ctx.game.load(), ctx.playtest.load()]);
  ctx.updates.start();
  render();
  if (state.setupNeeded) {
    // Opened with a map on a fresh install: set everything up and start it, no questions.
    const map = files.length ? await firstMap(files) : null;
    // Until setup is done (the first start, or it was skipped or didn't finish): what DropLaunch
    // does, before the setup window.
    if (!map) {
      renderWelcome();
      $("welcome").hidden = false;
    }
    else openSetup(map, { auto: Boolean(map) });
  } else if (files.length) ctx.maps.openFiles(files);
  else if (!state.welcomed) {
    // Set up, but the settings were reset (Settings → About): the welcome again.
    renderWelcome();
    $("welcome").hidden = false;
  }
  quickBranches();
});

/** Instant branch list (steamcmd.net mirror); "Check for updates" asks Steam itself. */
function quickBranches() {
  invoke("quick_branches")
    .then((snap) => {
      state = snap;
      render();
      if (!$("setup").hidden && !setupState.running) renderBranches($("setup-branch"), $("setup-branch").value || state.branch);
    })
    .catch(() => {});
}

/** The exe was opened with a map before setup: remember it, start it once setup is done. */
async function firstMap(files) {
  try {
    const result = await invoke("import_files", { paths: files });
    const path = result.maps[0];
    return path ? { path, name: fileName(path).replace(/\.map$/i, "") } : null;
  } catch {
    return null;
  }
}
