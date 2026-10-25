// A map's picture, big, with zoom and the in-game grid: opened from its icon in the map list.
// Bug reports testers sent for the map (playtest page) show as pins.
import { $, invoke, fmtBytes, fmtWhen } from "./util.js";
import { MapCanvas, readPicture, reportPin } from "./mapcanvas.js";
import { t, tn } from "./i18n.js";

const PREFS = "droplaunch.mapViewer";

function loadPrefs() {
  try {
    return { grid: true, names: true, reports: true, ...JSON.parse(localStorage.getItem(PREFS) ?? "{}") };
  } catch {
    return { grid: true, names: true, reports: true };
  }
}

export function initMapViewer(ctx) {
  const box = $("map-viewer");
  const prefs = loadPrefs();
  const map = new MapCanvas($("mv-map-wrap"));
  const pins = map.addLayer();
  map.showGrid = prefs.grid;
  map.showNames = prefs.names;
  let current = null;
  let reports = [];
  /** Which open() a reply belongs to (another map may have been opened meanwhile). */
  let token = 0;

  function savePrefs() {
    try {
      localStorage.setItem(PREFS, JSON.stringify(prefs));
    } catch {}
  }

  function renderSub() {
    const m = current;
    if (!m) return;
    const open = reports.filter((r) => !r.fixed).length;
    const size = map.picture ? t("mapview.size", { n: map.picture.size }) : null;
    const file = m.procgen ? t("maps.procgen.name") : [fmtBytes(m.size), m.modified ? t("mapview.saved", { when: fmtWhen(m.modified) }) : null].filter(Boolean).join(" · ");
    const openText = tn("mapview.open", open);
    const bugs = reports.length ? (reports.length > open ? t("mapview.openFixed", { open: openText, fixed: reports.length - open }) : openText) : null;
    $("mv-sub").textContent = [size, file, bugs].filter(Boolean).join(" · ");
    $("mv-reports-row").hidden = !reports.length;
    $("mv-reports-label").textContent = t("mapview.reports", { n: reports.length });
    $("mv-key-open").hidden = $("mv-key-fixed").hidden = !(reports.length && prefs.reports);
  }

  function renderPins() {
    pins.replaceChildren(...(prefs.reports ? reports : []).map((r) => reportPin(r, { onClick: () => map.centerOn(r.x, r.z) })));
    map.placeAll();
  }

  async function open(m) {
    const mine = ++token;
    current = m;
    reports = [];
    box.hidden = false;
    $("mv-title").textContent = m.name;
    $("mv-grid").checked = prefs.grid;
    $("mv-names").checked = prefs.names;
    $("mv-reports").checked = prefs.reports;
    $("mv-names-row").hidden = true;
    map.monuments = [];
    map.setPicture(null);
    map.reset();
    map.setMessage(t("map.drawing"));
    pins.replaceChildren();
    renderSub();
    requestAnimationFrame(() => map.fit());
    $("mv-close").focus();
    const path = m.procgen ? null : m.path;
    try {
      const [buffer, notes] = await Promise.all([invoke("map_picture", { path }), invoke("map_notes", { path }).catch(() => ({ reports: [], monuments: [] }))]);
      if (mine !== token) return;
      // Names are known once the map ran on a server with bug reports on (its plugin tells them).
      map.monuments = notes.monuments;
      $("mv-names-row").hidden = !notes.monuments.length;
      map.setPicture(readPicture(buffer));
      map.setMessage("");
      reports = notes.reports;
      renderPins();
      renderSub();
    } catch (err) {
      if (mine === token) map.setMessage(String(err?.message ?? err));
    }
  }

  function close() {
    token++;
    box.hidden = true;
    current = null;
    // The big picture takes a few MB: let it go.
    map.setPicture(null);
  }

  $("mv-close").addEventListener("click", close);
  box.addEventListener("click", (e) => e.target === box && close());
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && !box.hidden && $("modal").hidden) close();
  });
  $("mv-grid").addEventListener("change", (e) => {
    prefs.grid = e.target.checked;
    map.showGrid = prefs.grid;
    savePrefs();
    map.draw();
  });
  $("mv-names").addEventListener("change", (e) => {
    prefs.names = e.target.checked;
    map.showNames = prefs.names;
    savePrefs();
    map.draw();
  });
  $("mv-reports").addEventListener("change", (e) => {
    prefs.reports = e.target.checked;
    savePrefs();
    renderPins();
    renderSub();
  });

  return { open, close, isOpen: () => !box.hidden };
}
