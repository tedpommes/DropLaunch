// The server panel (selected map / running server with boot progress, players, Join, Stop,
// Restart) and the console (the server's log + commands over RCON).
import { $, el, icon, invoke, fmtBytes, fmtWhen, fmtDuration, fileName, toast, copyText, followScroll, desktopNotify, onNotificationClick, afterJoin, rustGameFor } from "./util.js";
import { initConsoleInput } from "./console.js";
import { LiveGraph, fmtRate, niceBytes, niceCeil } from "./graph.js";
import { t, rich, fmtList } from "./i18n.js";

const MAX_CONSOLE_LINES = 3000;
// Boot stages read from the server's log, in order (the backend's boot::Stage).
const STAGES = ["starting", "gameFiles", "mods", "map", "entities", "finishing", "ready"];
// The steps shown while it boots: the stages, plus DropLaunch's console connecting (the server
// opens WebRCON after loading its mods, before the map). `src` is the console colour they share.
const STEPS = [
  { key: "starting", name: t("boot.step.starting"), src: "server" },
  { key: "gameFiles", name: t("boot.step.gameFiles"), src: "server" },
  { key: "mods", name: t("boot.step.mods"), src: "mods" },
  { key: "rcon", name: t("boot.step.rcon"), src: "rcon" },
  { key: "map", name: t("boot.step.map"), src: "map" },
  { key: "entities", name: t("boot.step.entities"), src: "map" },
  { key: "finishing", name: t("boot.step.finishing"), src: "server" },
  { key: "ready", name: t("boot.step.ready"), src: "ready" },
];
// Who wrote a console line (the backend's ConsoleLine source), as its tag says it.
const SOURCE_TAGS = { server: "rust", map: "map", carbon: "carbon", oxide: "oxide", rustedit: "rustedit", rcon: "rcon", droplaunch: "droplaunch" };
// A world step that takes this long (no new step finished) gets a note; the ocean paths are the
// step some maps hang on for many minutes (normally a few seconds).
const SLOW_AFTER = { oceanPaths: 30_000 };
const SLOW_DEFAULT = 180_000;
// Trouble prefab test mode avoids, spotted in the log (the backend's boot::Issue).
const ISSUES = {
  cargoShip: [t("hint.cargoShip.title"), t("hint.cargoShip.text")],
  events: [t("hint.events.title"), t("hint.events.text")],
  terrain: [t("hint.terrain.title"), t("hint.terrain.text")],
};
const PREFAB_NOTE = t("server.prefabNote");
const PHASE_NAMES = {
  booting: t("server.phase.booting"),
  ready: t("server.phase.ready"),
  stopping: t("server.phase.stopping"),
  stopped: t("server.phase.stopped"),
  crashed: t("server.phase.crashed"),
};
const FRAMEWORK_NAMES = { none: t("framework.none"), carbon: t("framework.carbon"), oxide: t("framework.oxide") };
// What the performance graph can show (serverinfo every 2 seconds, the last 5 minutes).
const PERF_LINES = {
  fps: { key: "fps", label: t("perf.fps"), color: "#4cc38a", format: (v) => t("perf.fpsValue", { n: Math.round(v) }), floor: 30 },
  netIn: { key: "netIn", label: t("perf.netIn"), color: "#7a95e3", format: fmtRate, nice: niceBytes, floor: 4096 },
  netOut: { key: "netOut", label: t("perf.netOut"), color: "#e46fb0", format: fmtRate, nice: niceBytes, floor: 4096 },
  memory: { key: "memory", label: t("perf.memory"), color: "#e6b450", format: (mb) => fmtBytes(mb * 1048576), nice: (mb) => niceCeil(mb / 1024) * 1024, floor: 1024 },
  players: { key: "players", label: t("perf.players"), color: "#a98be8", format: (v) => String(Math.round(v)), nice: niceCeil, floor: 4 },
};
/** One line on its own (its own format and scale). */
const perfView = (tab, line) => ({ tab, series: [line], format: line.format, nice: line.nice, floor: line.floor });
const PERF_VIEWS = {
  all: { tab: t("perf.tab.all"), separate: true, series: Object.values(PERF_LINES), format: String },
  fps: perfView(t("perf.tab.fps"), PERF_LINES.fps),
  network: { tab: t("perf.tab.network"), series: [PERF_LINES.netIn, PERF_LINES.netOut], format: fmtRate, nice: niceBytes, floor: 4096 },
  memory: perfView(t("perf.tab.memory"), PERF_LINES.memory),
  players: perfView(t("perf.tab.players"), PERF_LINES.players),
};
const PERF_VIEW_KEY = "droplaunch.perfView";

function storedPerfView() {
  try {
    const v = localStorage.getItem(PERF_VIEW_KEY);
    return PERF_VIEWS[v] ? v : "all";
  } catch {
    return "all";
  }
}

export function initServer(ctx) {
  let view = null;
  let lastPhase = null;
  /** The running server's numbers over time (kept while the panel re-renders). */
  const perf = new LiveGraph({ views: PERF_VIEWS, view: storedPerfView(), span: 300000, every: 2000, height: 92, label: t("perf.label") });
  let perfServer = null;
  let perfUptime = null;
  const perfTabs = el("div", { class: "seg perf-tabs", role: "tablist", "aria-label": t("perf.tabs") });
  const perfBox = el("div", { class: "perf" }, perfTabs, perf.element);

  function renderPerfTabs() {
    perfTabs.replaceChildren(
      ...Object.entries(PERF_VIEWS).map(([name, v]) =>
        el(
          "button",
          {
            role: "tab",
            "aria-selected": String(perf.viewName === name),
            onclick: () => {
              perf.setView(name);
              try {
                localStorage.setItem(PERF_VIEW_KEY, name);
              } catch {}
              renderPerfTabs();
            },
          },
          el("span", { text: v.tab }),
        ),
      ),
    );
  }
  renderPerfTabs();

  /** A new serverinfo: one more point on the graph (a new server starts an empty one). */
  function trackPerf(v) {
    if (!v || v.id !== perfServer) {
      perf.clear();
      perfServer = v?.id ?? null;
      perfUptime = null;
    }
    const info = v?.info;
    if (!info || v.phase !== "ready" || info.uptime === perfUptime) return;
    perfUptime = info.uptime;
    perf.push({ fps: info.framerate, netIn: info.networkIn, netOut: info.networkOut, memory: info.memorySystem || info.memory, players: info.players });
  }
  /** A failed start the user closed (simple view). */
  let dismissed = null;
  /** Hints the user closed: "<server id>:<hint>". */
  const closedHints = new Set();
  /** Servers whose prefab test mode hint got "Keep waiting": the restart button stays, by Restart. */
  const waited = new Set();
  let lastSlow = false;
  /** Servers already announced as stuck (toast, taskbar flash): once each. */
  const announcedStuck = new Set();
  const consoleBox = $("console");
  const consoleScroll = followScroll(consoleBox);
  // The command line goes wherever the console goes (kept by reference: it moves around).
  const consoleForm = $("console-form");
  const consoleInput = $("console-input");
  const consoleSend = $("console-send");
  const commandLine = initConsoleInput({
    form: consoleForm,
    input: consoleInput,
    send: (text) => invoke("server_command", { text }).catch((err) => toast(String(err), "error")),
  });
  // Simple view: the card's own part, and the server's output under it (shown once the card is
  // dragged taller). They stay in place so the output keeps its scroll position.
  const cardMain = el("div", { class: "cp-main" });
  const cardLog = el("div", { class: "cp-log" }, el("button", { class: "icon-btn max-btn", dataset: { max: "console-card" }, title: t("common.fullScreen") }, icon("expand")));

  const isLive = () => Boolean(view && ["booting", "ready", "stopping"].includes(view.phase));

  // ---------------------------------------------------------------- panel
  function render() {
    const panel = $("server-panel");
    // Its place: the side column with more options, under the maps in the simple view.
    const home = ctx.expanded ? document.querySelector("#view-maps .col-side") : document.querySelector("#view-maps .col-main");
    // (Above the Kits card in the side column.)
    if (panel.parentElement !== home) home.insertBefore(panel, home.querySelector("#kits-card"));
    const sized = panel.classList.contains("sized");
    if (ctx.expanded) {
      panel.hidden = false;
      panel.replaceChildren(...(isLive() ? livePanel() : idlePanel()).filter(Boolean));
      panel.className = `card server-panel ${isLive() ? `is-live phase-${view.phase}` : "is-idle"}`;
    } else {
      // Simple view: only while a server runs (or its start failed), under the maps.
      const nodes = compactPanel();
      panel.hidden = !nodes;
      if (nodes) {
        cardMain.replaceChildren(...nodes);
        if (cardMain.parentElement !== panel || cardLog.parentElement !== panel) panel.replaceChildren(cardMain, cardLog);
        panel.className = `card server-panel compact ${isLive() ? `is-live phase-${view.phase}` : "is-failed"}${sized ? " sized" : ""}`;
      }
    }
    placeConsole(!ctx.expanded && !panel.hidden);
    renderConsoleState();
    ctx.kits?.render();
    ctx.layout?.apply();
  }

  /** The console lives in its card, or in the simple view's server card while that shows
   * (unless the console is full screen). */
  function placeConsole(cardShows) {
    const inCard = cardShows && ctx.layout?.maximized() !== "console-card";
    const parent = inCard ? cardLog : $("console-card");
    if (consoleBox.parentElement === parent) return;
    commandLine.close();
    parent.append(consoleBox, consoleForm);
    consoleScroll.bottom();
  }

  function statusRow(dotClass, label, extra) {
    return el("div", { class: "status-row" }, el("span", { class: `status-dot ${dotClass}` }), el("strong", { text: label }), extra, ctx.expanded ? foldButton() : null);
  }

  // ---------------------------------------------------------------- folding (more options)
  const SERVER_FOLDED = "droplaunch.serverFolded";

  /** Folded: just how it's going, the name with Restart and Stop (or Start). Kept between starts. */
  function serverFolded() {
    try {
      return localStorage.getItem(SERVER_FOLDED) === "1";
    } catch {
      return false;
    }
  }

  function foldButton() {
    const folded = serverFolded();
    return el(
      "button",
      {
        class: "icon-btn srv-fold",
        title: folded ? t("common.unfold") : t("common.fold"),
        "aria-expanded": String(!folded),
        onclick: () => {
          try {
            localStorage.setItem(SERVER_FOLDED, folded ? "0" : "1");
          } catch {
            /* not kept */
          }
          render();
        },
      },
      icon("chevron"),
    );
  }

  function modsText() {
    const s = ctx.state;
    if (!s || s.framework === "none") return t("framework.vanilla");
    const name = s.rustedit ? t("framework.withRustedit", { framework: FRAMEWORK_NAMES[s.framework] }) : FRAMEWORK_NAMES[s.framework];
    return s.oxideInactive ? t("framework.inactive", { name }) : name;
  }

  function idlePanel() {
    const s = ctx.state;
    const m = ctx.maps?.selected();
    const more = ctx.expanded;
    const nodes = [];
    if (view && !isLive()) {
      const crashed = view.phase === "crashed";
      nodes.push(statusRow(crashed ? "bad" : "off", crashed ? t("server.lastFailed") : t("server.none"), el("span", { class: "status-time", text: view.title })));
      if (view.message && (crashed || view.restarting === false)) {
        nodes.push(el("div", { class: `srv-message ${crashed ? "bad" : ""}`, text: view.message }));
      }
    } else {
      nodes.push(statusRow("off", t("server.none")));
    }
    if (!m) return nodes;
    if (more && serverFolded()) return [...nodes, startButton(m)];

    const meta = m.procgen
      ? t("maps.procgen.meta", { seed: 1184645116, size: 3000 })
      : [m.outside ? m.folder : m.folder ? `Maps\\${m.folder}` : t("maps.folderName"), fmtBytes(m.size), fmtWhen(m.modified)].filter(Boolean).join(" · ");
    nodes.push(
      el(
        "div",
        { class: "sel-card" },
        ctx.maps?.icon ? ctx.maps.icon(m, true) : el("span", { class: `map-icon big${m.procgen ? " procgen" : ""}` }, icon(m.procgen ? "terrain" : "map")),
        el("div", { class: "sel-text" }, el("div", { class: "sel-name", text: m.name, title: m.path }), el("div", { class: "sel-meta", text: meta, title: m.outside ? m.path : "" })),
      ),
    );
    if (more) {
      nodes.push(
        el(
          "dl",
          { class: "kv" },
          el("div", {}, el("dt", { text: t("server.kv.mods") }), el("dd", { class: s?.framework === "none" ? "missing" : s?.oxideInactive ? "warn" : "", text: modsText() })),
          el("div", {}, el("dt", { text: t("server.kv.connect") }), el("dd", { class: "mono", text: "127.0.0.1:28015" })),
        ),
      );
    }
    const setupNeeded = s?.setupNeeded;
    nodes.push(startButton(m));
    if (!m.procgen && !setupNeeded) {
      nodes.push(
        el(
          "div",
          { class: "prefab-start" },
          el(
            "button",
            {
              class: "btn btn-ghost btn-block",
              disabled: ctx.startBlocked() ? true : null,
              onclick: () => ctx.maps.start(m, { prefab: true }),
            },
            icon("flask"),
            el("span", { text: t("server.startPrefab") }),
          ),
          el("div", { class: "prefab-note", text: PREFAB_NOTE }),
        ),
      );
    }
    if (more && !m.procgen) {
      nodes.push(el("div", { class: "sel-links" }, el("button", { class: "btn btn-small btn-ghost", onclick: () => reveal(m.path) }, icon("map"), el("span", { text: t("server.showMapFile") }))));
    }
    return nodes;
  }

  /** "Starting", or "Stuck?" with a warning dot while a step hangs. */
  /** Start the selected map (or set up first). */
  function startButton(m) {
    const setupNeeded = ctx.state?.setupNeeded;
    return el(
      "button",
      {
        class: "btn btn-primary btn-lg btn-block",
        id: "btn-start",
        disabled: ctx.startBlocked() ? true : null,
        onclick: () => ctx.maps.start(m),
      },
      icon(setupNeeded ? "wrench" : "play"),
      el("span", { text: setupNeeded ? t("server.setupFirst") : t("server.start") }),
    );
  }

  function phaseLabel(v, hints) {
    if (hints.stuck) return ["warn", t("server.stuck")];
    return [{ booting: "busy", ready: "ok", stopping: "busy" }[v.phase], v.restarting ? t("server.restarting") : PHASE_NAMES[v.phase]];
  }

  function livePanel() {
    const v = view;
    const nodes = [];
    const hints = hintState(v);
    const [dot, label] = phaseLabel(v, hints);
    nodes.push(statusRow(dot, label, el("span", { class: "status-time", id: "srv-clock", text: clockText() })));
    nodes.push(el("div", { class: "srv-title-row" }, el("h3", { class: "srv-title", text: v.title, title: v.map ?? v.title }), el("div", { class: "srv-mini" }, ...controls(v, true))));
    if (serverFolded()) return nodes;
    nodes.push(el("div", { class: "srv-sub", text: subLine(v) }));
    if (v.attached) nodes.push(el("div", { class: "srv-note", text: t("server.attached") }));

    nodes.push(hintBox(v, hints, true));
    if (v.phase === "booting" || (v.phase === "stopping" && !v.readyAt)) {
      nodes.push(bootBox(v, hints.stuck));
    } else if (ctx.expanded && (v.phase === "ready" || v.readyAt)) {
      nodes.push(statsBox(v), perfBox, profilerBox(v));
    }

    if (v.phase === "ready") {
      nodes.push(
        el(
          "div",
          { class: "join-row" },
          joinButton(v),
          el("button", { class: "btn btn-ghost", onclick: () => copyText(v.connect, t("server.connectCommand")), title: v.connect }, icon("copy"), el("span", { text: t("server.copyConnect") })),
        ),
      );
      nodes.push(connectLine(v));
    }
    if (v.message && v.phase !== "stopping") nodes.push(el("div", { class: "srv-message", text: v.message }));
    return nodes.filter(Boolean);
  }

  // ---------------------------------------------------------------- Carbon's profiler
  /** A recording: { server, until, saving }. */
  let profiling = null;
  let profilingTimer = null;
  /** The last profile saved (its file), shown with Show until the next one. */
  let lastProfile = null;
  const PROFILER_PANEL = "https://carbonmod.gg/tools/profiler-panel/";
  const PROFILER_FOLDED = "droplaunch.profilerFolded";

  /** Carbon's profiler is on: record a while, then open the profile in Carbon's Profiler Panel. */
  function profilerBox(v) {
    if (!ctx.state?.profiler || v.boot.framework !== "Carbon" || v.phase !== "ready") return null;
    const recording = profiling?.server === v.id;
    const record = (secs) =>
      el(
        "button",
        { class: "btn btn-small btn-ghost", title: t("profiler.record.title"), onclick: () => startProfile(v, secs) },
        icon("play"),
        el("span", { text: secs < 60 ? t("profiler.recordSeconds", { n: secs }) : t("profiler.recordMinutes", { n: secs / 60 }) }),
      );
    const saved = lastProfile && !recording;
    const folded = profilerFolded();
    return el(
      "div",
      { class: `profiler-box${folded ? " folded" : ""}` },
      el(
        "div",
        { class: "profiler-head" },
        el("span", { class: `status-dot ${recording ? "busy" : saved ? "ok" : "off"}` }),
        el("strong", { text: t("profiler.title") }),
        el(
          "button",
          { class: "icon-btn profiler-fold", title: folded ? t("profiler.unfold") : t("profiler.fold"), "aria-expanded": String(!folded), onclick: () => (setProfilerFolded(!folded), render()) },
          icon("chevron"),
        ),
        el("span", { id: "profiler-note", text: recording ? profileNote() : saved ? t("profiler.saved", { file: fileName(lastProfile) }) : t("profiler.about") }),
      ),
      folded
        ? null
        : el(
        "div",
        { class: "profiler-actions" },
        recording
          ? el("button", { class: "btn btn-small btn-danger", disabled: profiling.saving ? true : null, onclick: stopProfile }, icon("stop"), el("span", { text: t("profiler.stop") }))
          : [record(30), record(120)],
        saved ? el("button", { class: "btn btn-small btn-ghost", title: lastProfile, onclick: () => reveal(lastProfile) }, icon("file"), el("span", { text: t("profiler.show") })) : null,
        el("button", { class: "btn btn-small btn-ghost", title: ctx.state.profilesDir, onclick: () => invoke("open_folder", { path: ctx.state.profilesDir }).catch((err) => toast(String(err), "error")) }, icon("folder"), el("span", { text: t("profiler.folder") })),
        el(
          "button",
          { class: "btn btn-small btn-ghost", title: t("profiler.panel.title"), onclick: () => invoke("open_link", { url: PROFILER_PANEL }).catch((err) => toast(String(err), "error")) },
          icon("external"),
          el("span", { text: t("profiler.panel") }),
        ),
      ),
    );
  }

  /** Folded away (only its title shows; how it starts), kept between starts. */
  function profilerFolded() {
    try {
      return localStorage.getItem(PROFILER_FOLDED) !== "0";
    } catch {
      return true;
    }
  }

  function setProfilerFolded(on) {
    try {
      localStorage.setItem(PROFILER_FOLDED, on ? "1" : "0");
    } catch {
      /* not kept */
    }
  }

  function profileNote() {
    if (profiling.saving) return t("profiler.saving");
    return t("profiler.recording", { n: Math.max(0, Math.ceil((profiling.until - Date.now()) / 1000)) });
  }

  async function startProfile(v, secs) {
    try {
      // Calls with their timings: what takes the time (a little slower while it records).
      await invoke("server_command", { text: `c.profile ${secs} -c -t` });
    } catch (err) {
      toast(String(err), "error");
      return;
    }
    profiling = { server: v.id, until: Date.now() + secs * 1000, saving: false };
    render();
    clearInterval(profilingTimer);
    profilingTimer = setInterval(() => {
      if (!profiling) return clearInterval(profilingTimer);
      const note = $("profiler-note");
      if (note) note.textContent = profileNote();
      // Carbon stops by itself after the duration (and takes a few seconds more to finish).
      if (Date.now() >= profiling.until && !profiling.saving) saveProfile();
    }, 1000);
  }

  async function stopProfile() {
    if (!profiling || profiling.saving) return;
    // c.profile again ends the recording.
    await invoke("server_command", { text: "c.profile" }).catch(() => {});
    saveProfile();
  }

  /**
   * Once Carbon has finished the recording (c.profilestatus says "False"), write it to
   * carbon/profiles as a .cprf (c.profiler.print; it answers "Exported profile output at '<file>'").
   */
  async function saveProfile() {
    if (!profiling || profiling.saving) return;
    profiling.saving = true;
    render();
    try {
      for (let i = 0; i < 60; i++) {
        const status = await invoke("server_ask", { text: "c.profilestatus" }).catch(() => "");
        if (/false/i.test(status)) break;
        await new Promise((r) => setTimeout(r, 1000));
      }
      const reply = await invoke("server_ask", { text: "c.profiler.print" });
      const file = /'([^']+\.cprf)'/i.exec(reply)?.[1];
      if (file) {
        lastProfile = file;
        appendLines([{ text: t("profiler.savedLine", { file }), kind: "ok", source: "droplaunch", time: Date.now() }]);
        toast(t("profiler.savedToast", { file: fileName(file) }), "ok");
      } else {
        toast(t("profiler.notSaved", { reply: reply.trim() || t("profiler.noAnswer") }), "warn");
      }
    } catch (err) {
      toast(String(err), "error");
    } finally {
      profiling = null;
      clearInterval(profilingTimer);
      render();
    }
  }

  /** Join with the Rust game the server's branch needs (Rust, or Rust - Staging Branch). */
  function joinButton(v) {
    const game = rustGameFor(v.branch);
    return el("button", { class: "btn btn-primary", onclick: join, title: t("join.button.title", { game: game.name }) }, icon("join"), el("span", { text: game.button }));
  }

  /** "F1 → client.connect 127.0.0.1:28015" (only once the server is ready). */
  function connectLine(v, cls = "connect-line") {
    return el(
      "div",
      { class: cls, title: t("server.connectLine.title") },
      el("kbd", { text: "F1" }),
      el("span", { class: "connect-arrow", "aria-hidden": "true" }, icon("arrow-right")),
      el("code", { text: v.connect }),
    );
  }

  /** Restart and Stop (`small`: next to the server's name), after "Keep waiting" on the prefab
   * test mode hint with that restart left of them. */
  function controls(v, small = false) {
    const stopping = v.phase === "stopping";
    const size = small ? " btn-small" : "";
    return [
      waited.has(v.id) && v.phase === "booting" && prefabHelps(v) ? prefabButton(v, `btn${size} btn-ghost`, false) : null,
      el(
        "button",
        { class: `btn btn-ghost${size}`, disabled: stopping || !v.startFile ? true : null, onclick: restart, title: t("server.restart.title") },
        icon("refresh"),
        el("span", { text: t("server.restart") }),
      ),
      el(
        "button",
        { class: `btn btn-danger${size}`, onclick: () => stop(stopping), title: stopping ? t("server.stop.force.title") : v.rcon && v.phase === "ready" ? t("server.stop.save.title") : t("server.stop.title") },
        icon("stop"),
        el("span", { text: stopping ? t("server.forceStop") : t("server.stop") }),
      ),
    ];
  }

  /** Simple view: one card under the maps while a server runs, or when its start failed. */
  function compactPanel() {
    const v = view;
    if (!v) return null;
    if (!isLive()) {
      if (v.phase !== "crashed" || dismissed === v.id) return null;
      return [
        el(
          "div",
          { class: "cp-head" },
          el("span", { class: "status-dot bad" }),
          el("strong", { text: t("server.phase.crashed") }),
          el("span", { class: "cp-title", text: v.title, title: v.map ?? v.title }),
          el("span", { class: "cp-sub" }),
          el("button", { class: "icon-btn", title: t("common.close"), onclick: () => ((dismissed = v.id), render()) }, icon("x")),
        ),
        v.message ? el("div", { class: "srv-message bad", text: v.message }) : null,
      ].filter(Boolean);
    }
    const hints = hintState(v);
    const [dot, label] = phaseLabel(v, hints);
    const sub = subLine(v);
    const ready = v.phase === "ready";
    const nodes = [
      el(
        "div",
        { class: "cp-head" },
        el("span", { class: `status-dot ${dot}` }),
        el("strong", { text: label }),
        el("span", { class: "cp-title", text: v.title, title: v.map ?? v.title }),
        el("span", { class: "cp-sub", text: sub }),
        ready ? connectLine(v, "cp-connect") : null,
        // Until it's ready, Restart and Stop sit up here (the server's output gets the room), with
        // the time (they wrap to the next line together).
        ready
          ? el("span", { class: "status-time", id: "srv-clock", text: clockText() })
          : el("span", { class: "cp-head-buttons" }, ...controls(v, true), el("span", { class: "status-time", id: "srv-clock", text: clockText() })),
      ),
    ];
    nodes.push(hintBox(v, hints, true));
    if (v.phase === "booting" || (v.phase === "stopping" && !v.readyAt)) nodes.push(bootBox(v, hints.stuck));
    if (v.message && v.phase !== "stopping") nodes.push(el("div", { class: "srv-message", text: v.message }));

    // Ready: Join on the left, Restart and Stop on the right.
    if (ready) {
      nodes.push(
        el(
          "div",
          { class: "cp-actions" },
          el(
            "div",
            { class: "cp-join" },
            joinButton(v),
            el("button", { class: "btn btn-ghost", onclick: () => copyText(v.connect, t("server.connectCommand")), title: v.connect }, icon("copy"), el("span", { text: t("server.copyConnect") })),
          ),
          el("div", { class: "cp-control" }, el("div", { class: "cp-buttons" }, ...controls(v))),
        ),
      );
    }
    return nodes.filter(Boolean);
  }

  /** "Prefab test mode · staging branch · port 28015 · Carbon". */
  function subLine(v) {
    const branch = (ctx.state?.installs?.length ?? 0) > 1 && v.branch ? t("server.branchName", { branch: v.branch }) : null;
    return [v.prefab ? t("server.prefabMode") : null, branch, t("server.port", { port: v.port }), v.boot.framework].filter(Boolean).join(" · ");
  }

  /** `stuck`: the bar turns amber and says so. */
  function bootBox(v, stuck = false) {
    const pct = Math.round(v.boot.percent);
    return el(
      "div",
      { class: `boot${stuck ? " stalled" : ""}` },
      el("div", { class: "progress-row" }, el("span", { class: "boot-stage", text: stageLabel(v) }), el("span", { class: "boot-pct", text: `${pct}%` })),
      el("div", { class: "bar" }, el("div", { class: "bar-fill", style: `width:${pct}%` })),
      el("div", { class: "boot-detail", text: v.boot.detail || t("boot.waiting"), title: v.boot.detail }),
      el(
        "ol",
        { class: "boot-path", "aria-label": t("boot.steps") },
        bootSteps(v).map((step, i, all) =>
          el(
            "li",
            { class: `${step.state || "todo"} src-${step.src}`, title: { done: t("boot.state.done"), active: t("boot.state.active"), "": t("boot.state.todo") }[step.state] },
            el("i", { class: "dot" }, step.state === "done" ? icon("check") : null),
            el("span", { class: "label", text: step.name }),
            i < all.length - 1 ? el("span", { class: "link", "aria-hidden": "true" }) : null,
          ),
        ),
      ),
    );
  }

  /** Every step with its state: done, active or "" (still to come). */
  function bootSteps(v) {
    const index = STAGES.indexOf(v.boot.stage);
    const mods = v.boot.framework ? v.boot.framework.toLowerCase() : "server";
    return STEPS.filter((step) => step.key !== "mods" || v.boot.framework || index === STAGES.indexOf("mods")).map((step) => {
      const src = step.src === "mods" ? mods : step.src;
      if (step.key === "rcon") {
        return { name: step.name, src, state: v.rcon ? "done" : v.boot.rconStarted ? "active" : "" };
      }
      const i = STAGES.indexOf(step.key);
      const name = step.key === "mods" && v.boot.framework ? v.boot.framework : step.name;
      return { name, src, state: i < index ? "done" : i === index ? "active" : "" };
    });
  }

  /** A world step that's taking much longer than it should: { work, label, since }. */
  function slowStep(v) {
    const b = v?.boot;
    if (!b || v.phase !== "booting" || !b.work || !b.workSince) return null;
    const limit = SLOW_AFTER[b.work] ?? SLOW_DEFAULT;
    return Date.now() - b.workSince > limit ? { work: b.work, since: b.workSince } : null;
  }

  const WORK_LABELS = {
    readingMap: t("boot.work.readingMap"),
    generating: t("boot.work.generating"),
    spawningWorld: t("boot.work.spawningWorld"),
    terrainMesh: t("boot.work.terrainMesh"),
    processingWorld: t("boot.work.processingWorld"),
    oceanPaths: t("boot.work.oceanPaths"),
  };

  function slowFor(since) {
    const minutes = Math.floor((Date.now() - since) / 60000);
    return minutes >= 1 ? t("hint.forMinutes", { n: minutes }) : t("hint.forSeconds", { n: Math.round((Date.now() - since) / 1000) });
  }

  /**
   * Notes about trouble prefab test mode avoids (a step that hangs, errors in the log), and
   * whether to offer starting the map that way (custom maps that don't already run in it).
   */
  /** A custom map that doesn't run in prefab test mode yet (that mode may get it going). */
  const prefabHelps = (v) => Boolean(v.map) && !v.procgen && !v.prefab;

  function hintState(v) {
    const slow = slowStep(v);
    const items = [];
    if (slow) {
      const label = WORK_LABELS[slow.work] ?? t("boot.work.other");
      const ocean = slow.work === "oceanPaths";
      const time = el("span", { class: "hint-time", dataset: { since: slow.since }, text: `(${slowFor(slow.since)})` });
      items.push(
        ocean && prefabHelps(v)
          ? {
              key: `slow-${slow.work}`,
              stuck: true,
              title: rich("hint.stuck.title", { time }),
              text: t("hint.stuck.text"),
              prefab: true,
            }
          : {
              key: `slow-${slow.work}`,
              title: rich("hint.slow.title", { step: label, time }),
              text: ocean ? t("hint.slow.ocean") : t("hint.slow.other"),
            },
      );
    }
    if (prefabHelps(v)) {
      for (const issue of v.boot.issues ?? []) {
        const [title, text] = ISSUES[issue] ?? [];
        if (title) items.push({ key: issue, title: [el("span", { text: title })], text, prefab: true });
      }
    }
    const shown = items.filter((item) => !closedHints.has(`${v.id}:${item.key}`));
    return { shown, slow, stuck: shown.some((item) => item.stuck), offerPrefab: shown.some((item) => item.prefab) };
  }

  /**
   * The notes. Stuck: a loud amber banner with "Keep waiting" to quiet it, the prefab test mode
   * restart next to it (`withButton`). Once it's closed that restart stays, left of Restart.
   */
  function hintBox(v, { shown, slow, stuck, offerPrefab }, withButton) {
    if (!shown.length) return null;
    const close = () => {
      for (const item of shown) closedHints.add(`${v.id}:${item.key}`);
      if (offerPrefab) waited.add(v.id);
      render();
    };
    const actions = [
      stuck ? el("button", { class: "btn btn-small btn-ghost", title: t("hint.keepWaiting.title"), onclick: close }, el("span", { text: t("hint.keepWaiting") })) : null,
      offerPrefab && withButton ? prefabButton(v, `btn btn-small${stuck ? " attention" : ""}`) : null,
    ].filter(Boolean);
    return el(
      "div",
      { class: `hint${offerPrefab ? " prefab" : ""}${stuck ? " stuck" : ""}`, role: stuck ? "alert" : "status" },
      el(
        "div",
        { class: "hint-top" },
        el("span", { class: "hint-icon" }, icon(slow ? "hourglass" : "alert")),
        el("div", { class: "hint-body" }, ...shown.map((item) => el("div", { class: "hint-item" }, el("strong", {}, ...item.title), el("span", { text: item.text })))),
        stuck ? null : el("button", { class: "icon-btn", title: t("common.close"), onclick: close }, icon("x")),
      ),
      actions.length ? el("div", { class: "hint-actions" }, ...actions) : null,
    );
  }

  /** Once per server: a step hangs, so make sure the user sees it (also in the background). */
  function announceStuck(v) {
    if (announcedStuck.has(v.id)) return;
    announcedStuck.add(v.id);
    toast(t("hint.stuck.toast", { title: v.title }), "warn");
    if (!document.hasFocus()) {
      // Flashes the taskbar button until DropLaunch is looked at.
      window.__TAURI__.window?.getCurrentWindow?.().requestUserAttention?.(1).catch(() => {});
    }
  }

  function prefabButton(v, cls, primary = true) {
    return el(
      "button",
      { class: `${cls}${primary ? " btn-primary" : ""}`, title: t("server.prefabRestart.title", { title: v.title }), onclick: () => ctx.maps.start({ path: v.map, name: v.title }, { prefab: true, replace: true }) },
      icon("flask"),
      el("span", { text: t("server.prefabRestart") }),
    );
  }

  function stageLabel(v) {
    return (
      {
        starting: t("boot.stage.starting"),
        gameFiles: t("boot.stage.gameFiles"),
        mods: v.boot.framework ? t("boot.stage.mods", { framework: v.boot.framework }) : t("boot.stage.modsPlain"),
        map: v.procgen ? t("boot.stage.building") : t("boot.stage.map"),
        entities: t("boot.stage.entities"),
        finishing: t("boot.stage.finishing"),
        ready: t("server.phase.ready"),
      }[v.boot.stage] ?? t("server.phase.booting")
    );
  }

  function statsBox(v) {
    const info = v.info;
    const stat = (value, label) => el("div", { class: "stat" }, el("b", { text: value }), el("span", { text: label }));
    if (!info) {
      return el("div", { class: "stats" }, stat("…", t("server.stat.players")), stat("…", t("server.stat.fps")), stat("…", t("server.stat.entities")), stat(v.rcon ? "…" : "–", t("server.stat.uptime")));
    }
    const entities = info.entityCount >= 10000 ? `${Math.round(info.entityCount / 1000)}k` : String(info.entityCount);
    return el(
      "div",
      { class: "stats" },
      stat(`${info.players}/${info.maxPlayers}`, info.joining ? t("server.stat.joining", { n: info.joining }) : t("server.stat.players")),
      stat(String(Math.round(info.framerate)), t("server.stat.fps")),
      stat(entities, t("server.stat.entities")),
      stat(fmtDuration(info.uptime), t("server.stat.uptime")),
    );
  }

  /** Time since the start while it boots (once ready, the uptime tile takes over). */
  function clockText() {
    if (!view || view.phase !== "booting") return "";
    return fmtDuration((Date.now() - view.startedAt) / 1000);
  }

  setInterval(() => {
    const clock = $("srv-clock");
    if (clock && isLive()) clock.textContent = clockText();
    for (const time of document.querySelectorAll(".hint-time")) time.textContent = `(${slowFor(Number(time.dataset.since))})`;
    // A step just went past its expected time: show the note (and shout when it's stuck).
    const slow = Boolean(slowStep(view));
    if (slow !== lastSlow) {
      lastSlow = slow;
      render();
      ctx.maps?.render();
      if (slow && hintState(view).stuck) announceStuck(view);
    }
  }, 1000);

  // ---------------------------------------------------------------- actions
  async function stop(force) {
    try {
      await invoke("stop_server", { force });
    } catch (err) {
      toast(String(err), "error");
    }
  }

  async function restart() {
    try {
      await invoke("restart_server");
    } catch (err) {
      toast(String(err), "error");
    }
  }

  /** The right Rust game for the server's branch (Rust, or Rust - Staging Branch). */
  async function join() {
    try {
      await afterJoin(await invoke("join_server"));
    } catch (err) {
      toast(t("join.failed", { error: err }), "error");
    }
  }

  async function makeStartFile(m) {
    try {
      const paths = await invoke("create_start_file", { target: m.procgen ? { kind: "procgen" } : { kind: "map", path: m.path } });
      toast(
        paths.length > 1 ? t("server.startFiles.ready", { files: fmtList(paths.map(fileName)) }) : t("server.startFile.ready", { file: fileName(paths[0]) }),
      );
      await ctx.refresh();
      await ctx.maps.reload();
    } catch (err) {
      toast(String(err), "error");
    }
  }

  async function reveal(path) {
    try {
      await invoke("reveal", { path });
    } catch (err) {
      toast(String(err), "error");
    }
  }

  // ---------------------------------------------------------------- console
  function appendLines(lines) {
    if (!lines.length) return;
    const box = consoleBox;
    box.querySelector("#console-empty")?.remove();
    const frag = document.createDocumentFragment();
    for (const line of lines) {
      const time = new Date(line.time).toLocaleTimeString([], { hour12: false });
      const source = SOURCE_TAGS[line.source] ? line.source : line.kind === "cmd" || line.kind === "reply" ? "rcon" : line.kind === "log" ? "server" : "droplaunch";
      frag.append(
        el(
          "div",
          { class: `ln ${line.kind === "log" ? "plain" : line.kind} src-${source}` },
          el("span", { class: "t", text: time }),
          el("span", { class: "src", text: SOURCE_TAGS[source] }),
          el("span", { class: "m", text: line.text }),
        ),
      );
    }
    box.append(frag);
    while (box.childElementCount > MAX_CONSOLE_LINES) box.firstElementChild.remove();
    consoleScroll.after();
  }

  function renderConsoleState() {
    const connected = isLive() && view.rcon;
    $("console-live").hidden = !connected;
    consoleInput.disabled = !connected;
    consoleSend.disabled = !connected;
    consoleInput.placeholder = connected ? t("console.placeholder.connected") : isLive() ? t("console.placeholder.booting") : t("console.placeholder.idle");
    $("btn-open-log").disabled = !view?.logFile;
    // The commands of the server that runs (its branch's build, its mods).
    if (isLive()) commandLine.load(`${view.id}:${view.branch ?? ""}:${view.boot.framework ?? ""}`);
  }
  $("btn-clear-console").addEventListener("click", () => consoleBox.replaceChildren());
  $("btn-copy-console").addEventListener("click", () => copyText([...consoleBox.querySelectorAll(".ln")].map((r) => r.textContent).join("\n"), t("console.what")));
  $("btn-open-log").addEventListener("click", () => view?.logFile && reveal(view.logFile));

  // ---------------------------------------------------------------- state
  /** The same server in the same state, only its numbers (players, fps, uptime) are new. */
  const onlyNumbers = (a, b) => a && b && a.id === b.id && JSON.stringify({ ...a, info: null }) === JSON.stringify({ ...b, info: null });

  function apply(next) {
    const before = view;
    view = next;
    trackPerf(view);
    // Every few seconds while it runs: just the tiles that show them (with more options).
    if (onlyNumbers(before, next)) {
      $("server-panel").querySelector(".stats")?.replaceWith(statsBox(view));
      return;
    }
    if (before?.id !== view?.id) dismissed = null;
    const phase = view?.phase ?? null;
    if (before && view && before.id === view.id && lastPhase !== "ready" && phase === "ready") {
      toast(t("server.readyToast", { title: view.title, connect: view.connect }), "ok");
      if (!ctx.state?.autoJoin) desktopNotify({ tag: "ready", title: t("server.ready.title", { title: view.title }), body: t("server.ready.body"), action: "join" });
    }
    lastPhase = phase;
    render();
    ctx.onServerChange();
  }

  // A click on the "server is ready" notification.
  onNotificationClick("ready", () => {
    if (view?.phase === "ready") join();
  });

  window.__TAURI__.event.listen("server", (e) => {
    const ev = e.payload;
    if (ev.kind === "state") apply(ev.server);
    else if (ev.kind === "lines") appendLines(ev.lines);
  });

  async function load() {
    try {
      const status = await invoke("server_status");
      appendLines(status.lines);
      apply(status.server);
    } catch (err) {
      toast(t("server.readFailed", { error: err }), "error");
    }
  }

  return {
    load,
    render,
    apply,
    view: () => view,
    isLive,
    /** The boot hangs on a step prefab test mode skips. */
    stuck: () => Boolean(view && isLive() && hintState(view).stuck),
    reveal: consoleScroll.reveal,
    bottom: consoleScroll.bottom,
    makeStartFile,
    note: (text) => appendLines([{ text, kind: "error", source: "droplaunch", time: Date.now() }]),
  };
}
