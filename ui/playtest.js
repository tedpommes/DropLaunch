// The playtest page: bug reports testers send with /bug in Rust's chat, as pins on a picture of
// the map where they stood, live, with who's where right now. The backend (playtest/) keeps the
// reports per map (across its versions) and puts a small plugin in the server that sends them.
import { $, el, icon, invoke, toast, confirmDialog, desktopNotify, onNotificationClick, fmtAgo, fmtWhen } from "./util.js";
import { MapCanvas, readPicture, reportPin } from "./mapcanvas.js";
import { contextMenu } from "./menu.js";
import { t, tn, language } from "./i18n.js";

const TRAIL_MAX = 20000;
const COMPASS = [
  t("playtest.dir.n"),
  t("playtest.dir.ne"),
  t("playtest.dir.e"),
  t("playtest.dir.se"),
  t("playtest.dir.s"),
  t("playtest.dir.sw"),
  t("playtest.dir.w"),
  t("playtest.dir.nw"),
];
const PREFS = "droplaunch.playtest";

function loadPrefs() {
  try {
    return { grid: true, names: true, trails: false, filter: "all", ...JSON.parse(localStorage.getItem(PREFS) ?? "{}") };
  } catch {
    return { grid: true, names: true, trails: false, filter: "all" };
  }
}

const compass = (yaw) => COMPASS[Math.round((((yaw % 360) + 360) % 360) / 45) % 8];

const escapeHtml = (text) => String(text ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);

export function initPlaytest(ctx) {
  let view = null;
  let players = [];
  /** Where testers went this server run: [x, z] points (for "Where testers went"). */
  let trail = [];
  let trailServer = null;
  let pictureKey = null;
  let pictureError = null;
  let loadingPicture = false;
  let selected = null;
  let known = null;
  /** A spot testers were just teleported to: { x, z, until } (a ring for a moment). */
  let landed = null;
  const prefs = loadPrefs();

  const page = $("playtest");
  const map = new MapCanvas($("pt-map-wrap"), {
    onDraw: drawExtras,
    onBlankClick: () => {
      if (!selected) return;
      selected = null;
      renderPins();
      renderList();
      map.draw();
    },
    onContextMenu: teleportMenu,
  });
  map.showGrid = prefs.grid;
  map.showNames = prefs.names;
  const testers = map.addLayer();
  const pins = map.addLayer();

  function savePrefs() {
    try {
      localStorage.setItem(PREFS, JSON.stringify(prefs));
    } catch {}
  }

  const isOpen = () => !page.hidden;
  const live = () => view?.plugin === "active" && view?.server;

  function open() {
    page.hidden = false;
    document.body.classList.add("playtest-open");
    render();
    loadPicture();
    requestAnimationFrame(() => map.fit());
  }

  function close() {
    page.hidden = true;
    document.body.classList.remove("playtest-open");
  }

  // ---------------------------------------------------------------- the map's picture

  async function loadPicture(force = false) {
    if (!view?.map || loadingPicture) return;
    // The procedural map's file only exists once the server built it.
    const key = `${view.map.key}|${view.map.version}|${view.map.saved ?? ""}`;
    if (!force && pictureKey === key && (map.picture || pictureError)) return;
    if (view.map.procgen && view.server?.phase === "booting") {
      pictureError = t("playtest.picture.wait");
      renderEmpty();
      return;
    }
    loadingPicture = true;
    pictureKey = key;
    pictureError = null;
    renderEmpty();
    try {
      map.setPicture(readPicture(await invoke("playtest_picture")));
      ctx.maps?.pictureMade?.();
    } catch (err) {
      map.setPicture(null);
      pictureError = String(err?.message ?? err);
    } finally {
      loadingPicture = false;
    }
    renderEmpty();
  }

  function renderEmpty() {
    map.setMessage(loadingPicture ? t("map.drawing") : map.picture ? "" : pictureError || (view?.map ? "" : t("playtest.picture.start")));
  }

  /** Over the map: where testers went, the selected report's view, a spot just teleported to. */
  function drawExtras(g) {
    if (prefs.trails && trail.length) {
      // Overlapping spots add up: busy places glow brighter.
      g.fillStyle = "rgba(255, 196, 92, 0.3)";
      const r = Math.max(3, 2 * Math.sqrt(map.zoom) + 1);
      for (const [x, z] of trail) {
        const [sx, sy] = map.toScreen(x, z);
        if (sx < -4 || sy < -4 || sx > map.side + 4 || sy > map.side + 4) continue;
        g.beginPath();
        g.arc(sx, sy, r, 0, Math.PI * 2);
        g.fill();
      }
    }
    // Which way the selected report's tester looked, and what at.
    const r = view?.reports.find((x) => x.id === selected);
    if (r) {
      const [sx, sy] = map.toScreen(r.x, r.z);
      // Yaw 0 is north (up), clockwise.
      const rad = (r.yaw * Math.PI) / 180 - Math.PI / 2;
      const len = 64;
      const cone = g.createRadialGradient(sx, sy, 4, sx, sy, len);
      cone.addColorStop(0, "rgba(122, 149, 227, 0.85)");
      cone.addColorStop(1, "rgba(122, 149, 227, 0)");
      g.fillStyle = cone;
      g.beginPath();
      g.moveTo(sx, sy);
      g.arc(sx, sy, len, rad - 0.45, rad + 0.45);
      g.closePath();
      g.fill();
      if (r.lookAt) {
        const [lx, ly] = map.toScreen(r.lookAt[0], r.lookAt[2]);
        g.setLineDash([4, 4]);
        g.strokeStyle = "rgba(255, 255, 255, 0.8)";
        g.lineWidth = 1.5;
        g.beginPath();
        g.moveTo(sx, sy);
        g.lineTo(lx, ly);
        g.stroke();
        g.setLineDash([]);
        g.beginPath();
        g.arc(lx, ly, 5, 0, Math.PI * 2);
        g.stroke();
      }
    }
    if (landed && landed.until > Date.now()) {
      const [sx, sy] = map.toScreen(landed.x, landed.z);
      g.strokeStyle = "#7a95e3";
      g.lineWidth = 2.5;
      for (const radius of [7, 14]) {
        g.beginPath();
        g.arc(sx, sy, radius, 0, Math.PI * 2);
        g.stroke();
      }
    }
  }

  // ---------------------------------------------------------------- pins and testers

  function renderPins() {
    pins.replaceChildren(...visibleReports().map((r) => reportPin(r, { selected: r.id === selected, onClick: () => select(r.id, { scroll: true }) })));
    map.placeAll();
  }

  function renderTesters() {
    const have = new Map([...testers.children].map((n) => [n.dataset.id, n]));
    const keep = new Set();
    for (const p of players) {
      keep.add(p.id);
      let node = have.get(p.id);
      if (!node) {
        node = el(
          "button",
          { class: "pt-player", onclick: (e) => (e.stopPropagation(), teleport({ player: p.id }, p.name)) },
          el("i"),
          el("span", { class: "pt-player-name" }),
        );
        node.dataset.id = p.id;
        testers.append(node);
      }
      node.dataset.x = p.x;
      node.dataset.z = p.z;
      node.classList.toggle("sleeping", p.sleeping || p.dead);
      node.title = t("playtest.tester.title", { name: testerName(p) });
      node.querySelector(".pt-player-name").textContent = p.name;
      map.place(node);
    }
    for (const [id, node] of have) if (!keep.has(id)) node.remove();
  }

  /** "Ann", "Ann (sleeping)", "Ann (dead)". */
  function testerName(p) {
    return p.dead ? t("playtest.tester.dead", { name: p.name }) : p.sleeping ? t("playtest.tester.sleeping", { name: p.name }) : p.name;
  }

  // ---------------------------------------------------------------- right click: teleport there

  /** Who goes to the spot that was right-clicked: one tester, or everyone on the server. */
  function teleportMenu(e, x, z) {
    const square = map.gridName(x, z);
    const items = [{ label: t("playtest.tp.head", { square }), icon: "map", kind: "menu-head", disabled: true, run() {} }];
    const alive = players.filter((p) => !p.dead);
    if (!live()) items.push({ label: view?.server ? t("playtest.tp.noPlugin") : t("playtest.tp.noServer"), disabled: true, run() {} });
    else if (!alive.length) items.push({ label: t("playtest.tp.nobody"), disabled: true, run() {} });
    else {
      if (alive.length > 1) items.push({ label: t("playtest.tp.everyone", { n: alive.length }), icon: "users", run: () => teleportTo(null, x, z, square, null) }, "sep");
      for (const p of alive) items.push({ label: testerName(p), icon: "arrow-right", run: () => teleportTo(p.id, x, z, square, p.name) });
      for (const p of players.filter((q) => q.dead)) items.push({ label: testerName(p), disabled: true, run() {} });
    }
    contextMenu(e.clientX, e.clientY, items);
  }

  async function teleportTo(player, x, z, square, who) {
    try {
      const n = await invoke("playtest_teleport_to", { x, z, player });
      landed = { x, z, until: Date.now() + 2500 };
      map.draw();
      setTimeout(() => map.draw(), 2600);
      toast(player ? t("playtest.tp.done", { who, square }) : tn("playtest.tp.doneTesters", n, { square }), "ok");
    } catch (err) {
      toast(String(err), "error");
    }
  }

  // ---------------------------------------------------------------- the list

  function visibleReports() {
    const reports = view?.reports ?? [];
    const shown = reports.filter((r) => (prefs.filter === "open" ? !r.fixed : prefs.filter === "fixed" ? r.fixed : true));
    // Open ones first, newest first.
    return shown.sort((a, b) => Boolean(a.fixed) - Boolean(b.fixed) || b.time - a.time);
  }

  function select(id, { scroll = false, center = false } = {}) {
    selected = id;
    renderPins();
    renderList();
    map.draw();
    const r = view?.reports.find((x) => x.id === id);
    if (r && center) map.centerOn(r.x, r.z);
    if (scroll) $("pt-list").querySelector(`[data-id="${CSS.escape(id)}"]`)?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  }

  function reportMeta(r) {
    const dir = compass(r.yaw);
    const look = !r.look
      ? t("playtest.meta.look", { dir })
      : r.lookDistance != null
        ? t("playtest.meta.lookAtDistance", { dir, what: r.look, m: Math.round(r.lookDistance) })
        : t("playtest.meta.lookAt", { dir, what: r.look });
    const parts = [r.name, r.grid, fmtAgo(r.time / 1000), look];
    return parts.filter(Boolean).join(" · ");
  }

  function versionNote(r) {
    const current = view?.map?.version;
    const notes = [];
    if (r.version && r.version !== current) notes.push(t("playtest.note.reportedOn", { version: r.version }));
    if (r.fixed) {
      const ago = fmtAgo(r.fixed.time / 1000);
      notes.push(r.fixed.version && r.fixed.version !== r.version ? t("playtest.note.fixedIn", { version: r.fixed.version, ago }) : t("playtest.note.fixed", { ago }));
    }
    return notes.join(" · ");
  }

  function renderList() {
    const reports = view?.reports ?? [];
    const open = reports.filter((r) => !r.fixed).length;
    const fixed = reports.length - open;
    $("pt-count").textContent = reports.length
      ? t("playtest.count", { open, fixed, map: mapTitle() ?? t("playtest.thisMap") })
      : view?.map
        ? t("playtest.countNone", { map: mapTitle() })
        : "";
    for (const b of $("pt-filter").children) {
      b.setAttribute("aria-selected", String(b.dataset.filter === prefs.filter));
      const n = b.dataset.filter === "open" ? open : b.dataset.filter === "fixed" ? fixed : reports.length;
      b.querySelector("b").textContent = n ? ` ${n}` : "";
    }
    const canTeleport = Boolean(live());
    const items = visibleReports().map((r) => {
      const note = versionNote(r);
      return el(
        "div",
        {
          class: `pt-bug${r.fixed ? " fixed" : ""}${r.id === selected ? " sel" : ""}`,
          dataset: { id: r.id },
          onclick: (e) => {
            if (!e.target.closest("button")) select(r.id, { center: true });
          },
        },
        el("span", { class: "pt-num", text: String(r.n) }),
        el("div", { class: "pt-bug-text" }, el("strong", { text: r.text }), el("span", { class: "pt-meta", text: reportMeta(r) }), note ? el("span", { class: "pt-meta", text: note }) : null),
        el(
          "div",
          { class: "pt-acts" },
          r.fixed
            ? null
            : el(
                "button",
                {
                  class: "btn btn-small btn-ghost",
                  disabled: !canTeleport,
                  title: canTeleport ? t("playtest.teleport.title") : t("playtest.teleport.off"),
                  onclick: () => teleport({ report: r.id }, `#${r.n}`),
                },
                el("span", { text: t("playtest.teleport") }),
              ),
          el("button", { class: "btn btn-small btn-ghost", onclick: () => setFixed(r, !r.fixed) }, el("span", { text: r.fixed ? t("playtest.reopen") : t("playtest.markFixed") })),
          el("button", { class: "icon-btn pt-delete", title: t("playtest.delete.title"), onclick: () => remove(r) }, icon("trash")),
        ),
      );
    });
    // How it works, until the first reports are in (the status line says it from then on).
    $("pt-howto").hidden = reports.length > 0;
    $("pt-list").replaceChildren(
      ...(items.length
        ? items
        : [el("div", { class: "pt-none", text: reports.length ? t("playtest.none.filter") : t("playtest.none.yet") })]),
    );
  }

  /** What the kit is doing, with a way forward when it can't work. */
  function renderStatus() {
    const box = $("pt-status");
    const p = view?.plugin;
    const actions = [];
    let [kind, text] = ["", ""];
    if (p === "noServer") [kind, text] = ["", view?.reports?.length ? t("playtest.status.lastTest") : t("playtest.status.noServer")];
    else if (p === "off") {
      [kind, text] = ["warn", t("playtest.status.off")];
      actions.push(el("button", { class: "btn btn-small btn-primary", onclick: () => setEnabled(true) }, el("span", { text: t("playtest.turnOn") })));
    } else if (p === "noFramework") {
      [kind, text] = ["warn", t("playtest.status.noFramework")];
      actions.push(
        el(
          "button",
          {
            class: "btn btn-small btn-ghost",
            onclick: () => {
              close();
              ctx.showFiles?.();
            },
          },
          el("span", { text: t("topbar.tab.files") }),
        ),
      );
    } else if (p === "starting") [kind, text] = ["busy", t("playtest.status.starting")];
    else if (p === "loading") [kind, text] = ["busy", t("playtest.status.loading")];
    else if (p === "notLoaded") [kind, text] = ["warn", t("playtest.status.notLoaded")];
    else if (p === "active") [kind, text] = ["ok", t("playtest.status.active")];
    box.className = `pt-status ${kind}`.trim();
    box.replaceChildren(el("span", { class: `status-dot ${kind || "off"}` }), el("span", { class: "pt-status-text", text }), ...actions);
    box.hidden = !text;
  }

  /** The map's name (the procedural map in the language in use). */
  function mapTitle() {
    if (!view?.map) return null;
    return view.map.procgen ? t("maps.procgen.name") : view.map.title;
  }

  function renderHead() {
    $("pt-title").textContent = (view?.server?.procgen ? t("maps.procgen.name") : view?.server?.title) ?? mapTitle() ?? t("playtest.title");
    const chips = [];
    if (view?.map) {
      const text = view.map.procgen
        ? t("maps.procgen.name")
        : view.map.saved
          ? t("playtest.chip.saved", { version: view.map.version, when: fmtWhen(view.map.saved / 1000) })
          : view.map.version;
      chips.push(el("span", { class: "pt-chip", title: view.map.version, text }));
    }
    if (view?.plugin === "active") {
      const n = players.length;
      chips.push(el("span", { class: "pt-chip is-live", text: n ? tn("playtest.chip.testers", n) : t("playtest.chip.live") }));
    }
    $("pt-chips").replaceChildren(...chips);
    $("pt-export").disabled = !view?.reports?.length;
    $("pt-enabled").checked = Boolean(view?.enabled);
  }

  function render() {
    if (!isOpen()) return;
    renderHead();
    renderStatus();
    renderList();
    renderPins();
    renderTesters();
    renderEmpty();
    $("pt-show-grid").checked = prefs.grid;
    $("pt-show-names").checked = prefs.names;
    $("pt-show-trails").checked = prefs.trails;
    map.draw();
  }

  // ---------------------------------------------------------------- actions

  // ---------------------------------------------------------------- the card (side column)
  /** How many of the newest open reports the card shows. */
  const CARD_REPORTS = 3;

  /** The card under Give items: how bug reports stand, the newest open ones, the page for all. */
  function renderCard() {
    const body = $("bugs-body");
    const p = view?.plugin ?? "noServer";
    const reports = view?.reports ?? [];
    const unfixed = reports.filter((r) => !r.fixed).sort((a, b) => b.time - a.time);
    const kind = p === "active" ? "ok" : p === "off" || p === "noFramework" || p === "notLoaded" ? "warn" : p === "noServer" ? "off" : "busy";
    const text = {
      noServer: reports.length ? t("playtest.status.lastTest") : t("bugs.noServer"),
      off: t("bugs.off"),
      noFramework: t("bugs.noFramework"),
      starting: t("playtest.status.starting"),
      loading: t("playtest.status.loading"),
      notLoaded: t("playtest.status.notLoaded"),
      active: t("bugs.active"),
    }[p];
    const count = $("bugs-count");
    count.hidden = !unfixed.length;
    count.textContent = String(unfixed.length);
    count.title = tn("bugs.openCount", unfixed.length);
    body.replaceChildren(...[
      el("div", { class: "bugs-status" }, el("span", { class: `status-dot ${kind}` }), el("span", { text })),
      p === "off" ? el("button", { class: "btn btn-small btn-primary bugs-on", onclick: () => setEnabled(true) }, el("span", { text: t("playtest.turnOn") })) : null,
      unfixed.length
        ? el(
            "div",
            { class: "bugs-list" },
            unfixed.slice(0, CARD_REPORTS).map((r) =>
              el(
                "button",
                { type: "button", class: "bugs-item", title: r.text, onclick: () => (open(), select(r.id, { scroll: true, center: true })) },
                el("span", { class: "bugs-n", text: `#${r.n}` }),
                el("span", { class: "bugs-text" }, el("strong", { text: r.text }), el("span", { text: [r.name, r.grid, fmtAgo(r.time / 1000)].filter(Boolean).join(" · ") })),
              ),
            ),
          )
        : null,
      el(
        "button",
        { class: "btn btn-small btn-ghost btn-block", onclick: () => open() },
        icon("bug"),
        el("span", { text: unfixed.length > CARD_REPORTS ? t("bugs.openAll", { n: unfixed.length }) : t("bugs.openPage") }),
      ),
    ].filter(Boolean));
  }

  async function apply(next) {
    view = next;
    renderCard();
    map.fallbackSize = view?.size || 4000;
    map.gridInfo = view?.grid ?? null;
    map.monuments = view?.monuments ?? [];
    $("pt-names-row").hidden = !map.monuments.length;
    ctx.server?.render?.();
    if (isOpen()) {
      render();
      loadPicture();
    }
  }

  async function call(cmd, args) {
    try {
      apply(await invoke(cmd, args));
    } catch (err) {
      toast(String(err), "error");
    }
  }

  const setFixed = (r, fixed) => call("playtest_set_fixed", { id: r.id, fixed });
  const setEnabled = (on) => call("playtest_set_enabled", { on });

  async function remove(r) {
    const ok = await confirmDialog({ title: t("playtest.delete.confirm.title", { n: r.n }), text: t("playtest.delete.confirm.text", { text: r.text, name: r.name }), ok: t("playtest.delete") });
    if (ok) call("playtest_delete", { id: r.id });
  }

  async function teleport(target, what) {
    try {
      await invoke("playtest_teleport", { report: target.report ?? null, player: target.player ?? null });
      toast(t("playtest.tp.you", { what }), "ok");
    } catch (err) {
      toast(String(err), "error");
    }
  }

  // ---------------------------------------------------------------- export

  /** One HTML file with everything in it: the map with pins, the list, and when. */
  async function exportReport() {
    if (!view?.reports?.length) return;
    const size = 1200;
    const shot = document.createElement("canvas");
    shot.width = shot.height = size;
    const g = shot.getContext("2d");
    g.fillStyle = "#0e2a33";
    g.fillRect(0, 0, size, size);
    if (map.picture) g.drawImage(map.picture.canvas, 0, 0, size, size);
    const world = map.worldSize();
    const at = (x, z) => [(x / world + 0.5) * size, (0.5 - z / world) * size];
    if (trail.length && prefs.trails) {
      g.fillStyle = "rgba(255, 196, 92, 0.3)";
      for (const [x, z] of trail) {
        const [sx, sy] = at(x, z);
        g.beginPath();
        g.arc(sx, sy, 3, 0, Math.PI * 2);
        g.fill();
      }
    }
    const reports = visibleReports();
    for (const r of reports) {
      const [sx, sy] = at(r.x, r.z);
      g.beginPath();
      g.arc(sx, sy, 11, 0, Math.PI * 2);
      g.fillStyle = r.fixed ? "#4cc38a" : "#ef6a62";
      g.fill();
      g.lineWidth = 2;
      g.strokeStyle = "#fff";
      g.stroke();
      g.fillStyle = "#fff";
      g.font = "700 11px Segoe UI, sans-serif";
      g.textAlign = "center";
      g.textBaseline = "middle";
      g.fillText(String(r.n), sx, sy + 0.5);
    }
    const image = shot.toDataURL("image/jpeg", 0.86);
    const title = mapTitle() ?? view.server?.title ?? t("playtest.title");
    const open = view.reports.filter((r) => !r.fixed).length;
    const when = new Date().toLocaleString(language.dateLocale);
    const rows = reports
      .map(
        (r) => `<tr class="${r.fixed ? "fixed" : ""}"><td class="n"><span>${r.n}</span></td><td><b>${escapeHtml(r.text)}</b><div class="meta">${escapeHtml(reportMeta(r))}</div>${versionNote(r) ? `<div class="meta">${escapeHtml(versionNote(r))}</div>` : ""}<div class="meta pos">${escapeHtml(`teleportpos ${r.x.toFixed(1)},${r.y.toFixed(1)},${r.z.toFixed(1)}`)}</div></td></tr>`,
      )
      .join("");
    const html = `<!doctype html><html lang="${escapeHtml(language.code)}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>${escapeHtml(t("playtest.export.pageTitle", { map: title }))}</title><style>
:root{color-scheme:dark}body{margin:0;background:#1e2028;color:#b2b7bf;font:14px/1.5 "Segoe UI",system-ui,sans-serif}main{max-width:1180px;margin:0 auto;padding:24px 16px}
h1{margin:0;color:#eef0f4;font-size:22px}.sub{margin:4px 0 20px;color:#80868f}.wrap{display:grid;grid-template-columns:minmax(0,1fr) minmax(0,1fr);gap:20px;align-items:start}
img{width:100%;border-radius:14px;display:block}table{width:100%;border-collapse:separate;border-spacing:0 8px}td{background:#2a2c35;padding:10px 12px;vertical-align:top}
td:first-child{border-radius:12px 0 0 12px;width:30px}td:last-child{border-radius:0 12px 12px 0}b{color:#eef0f4}.n span{display:inline-grid;place-items:center;width:24px;height:24px;border-radius:50%;background:#ef6a62;color:#fff;font-weight:700;font-size:12px}
.fixed .n span{background:#4cc38a}.fixed b{text-decoration:line-through;color:#80868f}.meta{color:#80868f;font-size:12.5px}.pos{font-family:Consolas,monospace;font-size:12px}
footer{margin-top:24px;color:#80868f;font-size:12px}@media(max-width:800px){.wrap{grid-template-columns:1fr}}</style></head><body><main>
<h1>${escapeHtml(t("playtest.export.heading", { map: title }))}</h1><p class="sub">${escapeHtml(t("playtest.export.summary", { version: view.map?.version ?? "", open, fixed: view.reports.length - open, when }))}</p>
<div class="wrap"><img src="${image}" alt="${escapeHtml(t("playtest.export.alt", { map: title }))}"><table>${rows}</table></div>
<footer>${escapeHtml(t("playtest.export.footer"))}</footer></main></body></html>`;
    try {
      const path = await invoke("playtest_save_report", { html, name: t("playtest.export.fileName", { map: title, date: new Date().toISOString().slice(0, 10) }) });
      if (path) toast(t("playtest.export.saved", { path }), "ok");
    } catch (err) {
      toast(String(err), "error");
    }
  }

  // ---------------------------------------------------------------- events

  window.__TAURI__.event.listen("playtest", (e) => {
    const next = e.payload;
    const ids = new Set(next.reports.map((r) => r.id));
    // A new report while DropLaunch runs: say so (the first look only learns what's there).
    if (known && next.map?.key === view?.map?.key) {
      for (const r of next.reports) {
        if (known.has(r.id)) continue;
        toast(t("playtest.newBug", { n: r.n, name: r.name, grid: r.grid, text: r.text }), "warn");
        desktopNotify({ tag: "bug", title: t("playtest.newBug.title", { n: r.n, name: r.name }), body: t("playtest.newBug.body", { grid: r.grid, text: r.text }), action: "open" });
        if (isOpen() && !selected) selected = r.id;
      }
    }
    known = ids;
    apply(next);
  });

  window.__TAURI__.event.listen("playtest-players", (e) => {
    const { serverId, players: next } = e.payload;
    if (trailServer !== serverId) {
      trailServer = serverId;
      trail = [];
    }
    players = next;
    for (const p of next) if (!p.dead && !p.sleeping) trail.push([p.x, p.z]);
    if (trail.length > TRAIL_MAX) trail = trail.slice(-TRAIL_MAX);
    if (!isOpen()) return;
    renderTesters();
    renderHead();
    if (prefs.trails) map.draw();
  });

  onNotificationClick("bug", () => open());

  $("pt-back").addEventListener("click", close);
  $("pt-export").addEventListener("click", exportReport);
  $("pt-enabled").addEventListener("change", (e) => setEnabled(e.target.checked));
  $("pt-show-grid").addEventListener("change", (e) => {
    prefs.grid = e.target.checked;
    map.showGrid = prefs.grid;
    savePrefs();
    map.draw();
  });
  $("pt-show-names").addEventListener("change", (e) => {
    prefs.names = e.target.checked;
    map.showNames = prefs.names;
    savePrefs();
    map.draw();
  });
  $("pt-show-trails").addEventListener("change", (e) => {
    prefs.trails = e.target.checked;
    savePrefs();
    map.draw();
  });
  for (const b of $("pt-filter").children) {
    b.addEventListener("click", () => {
      prefs.filter = b.dataset.filter;
      savePrefs();
      renderList();
      renderPins();
    });
  }
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && isOpen() && $("modal").hidden) close();
  });

  $("bugs-open").addEventListener("click", () => open());
  renderCard();

  async function load() {
    try {
      const next = await invoke("playtest_status");
      known = new Set(next.reports.map((r) => r.id));
      apply(next);
    } catch (err) {
      console.warn("playtest status", err);
    }
  }

  return {
    load,
    open,
    close,
    isOpen,
    /** Open reports of the running server's map (for its card). */
    openCount: () => (view?.server ? (view.reports ?? []).filter((r) => !r.fixed).length : 0),
    /** The kit can take reports from the running server. */
    state: () => view?.plugin ?? "noServer",
  };
}
