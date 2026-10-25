// Splitters: between the maps and what's under them (the console with more options, the server
// card in the simple view), and one for the width of the side column (shared by both pages). Plus full screen consoles, and room for cards that grow (they take it
// from the map list or SteamCMD's output instead of scrolling).
import { $, icon } from "./util.js";
import { t } from "./i18n.js";

const SPLIT_KEY = "droplaunch.split.";
const COLUMN_KEY = "droplaunch.split.column";
const SIDE_KEY = "droplaunch.sideWidth";
const MIN_SIDE = 260;
const MIN_MAIN = 420;

function load(key) {
  try {
    const value = Number.parseFloat(localStorage.getItem(key));
    return Number.isFinite(value) ? value : null;
  } catch {
    return null;
  }
}

function save(key, value) {
  try {
    if (value == null) localStorage.removeItem(key);
    else localStorage.setItem(key, String(value));
  } catch {
    /* per viewer only */
  }
}

function loadJson(key) {
  try {
    const value = JSON.parse(localStorage.getItem(key));
    return value && typeof value === "object" ? value : null;
  } catch {
    return null;
  }
}

/**
 * A splitter that shares a column's height between the card above it and the one below.
 * @param {object} o
 * @param {HTMLElement} o.splitter
 * @param {() => HTMLElement} o.top
 * @param {() => HTMLElement} o.bottom
 * @param {() => string} o.key where the share is remembered
 * @param {number} o.minTop
 * @param {number} o.minBottom
 * @param {(share: number|null) => void} [o.shared] after the share changed
 */
function heightSplit(ctx, { splitter, top, bottom, key, minTop, minBottom, shared, active = () => true }) {
  /** Share of the height the top card gets (null: the default layout). */
  function setShare(share) {
    const [above, below] = [top(), bottom()];
    above.style.flex = share == null ? "" : `${share} 1 0`;
    below.style.flex = share == null ? "" : `${1 - share} 1 0`;
    shared?.(share);
  }

  function apply() {
    if (!active()) return;
    const below = bottom();
    const visible = !below.hidden && getComputedStyle(below).display !== "none";
    splitter.hidden = !visible;
    setShare(visible ? load(SPLIT_KEY + key()) : null);
  }

  let drag = null;
  splitter.addEventListener("pointerdown", (e) => {
    if (e.button !== 0 || !active()) return;
    const above = top().getBoundingClientRect().height;
    const below = bottom().getBoundingClientRect().height;
    drag = { y: e.clientY, above, total: above + below, share: null };
    splitter.setPointerCapture(e.pointerId);
    splitter.classList.add("dragging");
    document.body.classList.add("resizing");
    e.preventDefault();
  });
  splitter.addEventListener("pointermove", (e) => {
    if (!drag) return;
    const above = Math.max(minTop, Math.min(drag.total - minBottom, drag.above + e.clientY - drag.y));
    drag.share = above / drag.total;
    setShare(drag.share);
  });
  const end = () => {
    if (!drag) return;
    if (drag.share != null) save(SPLIT_KEY + key(), drag.share);
    drag = null;
    splitter.classList.remove("dragging");
    document.body.classList.remove("resizing");
    ctx.onResize?.();
  };
  splitter.addEventListener("pointerup", end);
  splitter.addEventListener("pointercancel", end);
  splitter.addEventListener("dblclick", () => {
    if (!active()) return;
    save(SPLIT_KEY + key(), null);
    apply();
    ctx.onResize?.();
  });
  splitter.addEventListener("keydown", (e) => {
    if ((e.key !== "ArrowUp" && e.key !== "ArrowDown") || !active()) return;
    e.preventDefault();
    const above = top().getBoundingClientRect().height;
    const total = above + bottom().getBoundingClientRect().height;
    const share = Math.max(minTop / total, Math.min(1 - minBottom / total, above / total + (e.key === "ArrowUp" ? -0.05 : 0.05)));
    save(SPLIT_KEY + key(), share);
    setShare(share);
    ctx.onResize?.();
  });
  return { apply };
}

/**
 * Simple view: the maps and the server card share the window's height; the splitter under the
 * maps moves the line between them. What was dragged is remembered as weights (the cards'
 * heights then); none: the server card its own height and the maps the rest. The server card
 * shows the server's output once it was made taller than its own content.
 */
function columnSplit(ctx) {
  // The gap and the command line: the output shows as soon as there's room for that, and grows.
  const SERVER_LOG_ROOM = 58;
  const cards = {
    maps: () => document.querySelector(".maps-card"),
    server: () => $("server-panel"),
  };
  const MIN = { maps: 150, server: 90 };
  let weights = loadJson(COLUMN_KEY);
  // Only the simple view.
  const active = () => !ctx.expanded;

  /** On screen. */
  function shown(name) {
    const card = cards[name]();
    return !card.hidden && card.offsetParent !== null;
  }

  function apply() {
    const on = active();
    for (const [name, card] of Object.entries(cards)) {
      const weight = on && weights && shown(name) ? weights[name] : null;
      card().style.flex = weight == null ? "" : `${weight} 1 0`;
    }
    $("server-panel").classList.toggle("sized", on && Boolean(weights?.sized) && shown("server"));
    if (on) $("splitter").hidden = !shown("server");
  }

  /** The server card without its output: it doesn't get smaller than that. */
  function serverOwn() {
    const card = cards.server();
    const main = card.querySelector(".cp-main");
    if (!main) return MIN.server;
    const style = getComputedStyle(card);
    const px = (name) => Number.parseFloat(style[name]) || 0;
    return Math.ceil(main.offsetHeight + px("paddingTop") + px("paddingBottom") + px("borderTopWidth") + px("borderBottomWidth"));
  }

  /** Where the line between two cards can go. */
  function limits(upper, lower) {
    const min = (name) => (name === "server" ? Math.max(MIN.server, serverOwn()) : MIN[name]);
    return { minUpper: min(upper), minLower: min(lower), own: serverOwn() };
  }

  function measure() {
    const next = { ...(weights ?? {}) };
    for (const name of Object.keys(cards)) if (shown(name)) next[name] = cards[name]().getBoundingClientRect().height;
    return next;
  }

  function attach(splitter, pair) {
    let drag = null;
    splitter.addEventListener("pointerdown", (e) => {
      if (e.button !== 0 || ctx.expanded) return;
      const [upper, lower] = pair();
      if (!upper || !lower) return;
      weights = { ...measure(), sized: Boolean(weights?.sized) };
      drag = { y: e.clientY, upper, lower, a: weights[upper], total: weights[upper] + weights[lower], ...limits(upper, lower), moved: false };
      splitter.setPointerCapture(e.pointerId);
      splitter.classList.add("dragging");
      document.body.classList.add("resizing");
      e.preventDefault();
    });
    splitter.addEventListener("pointermove", (e) => {
      if (!drag) return;
      move(drag, drag.a + e.clientY - drag.y);
      drag.moved = true;
    });
    const end = () => {
      if (!drag) return;
      if (drag.moved) save(COLUMN_KEY, JSON.stringify(weights));
      else weights = loadJson(COLUMN_KEY); // just a click
      drag = null;
      apply();
      splitter.classList.remove("dragging");
      document.body.classList.remove("resizing");
      ctx.onResize?.();
    };
    splitter.addEventListener("pointerup", end);
    splitter.addEventListener("pointercancel", end);
    splitter.addEventListener("dblclick", () => {
      if (ctx.expanded) return;
      weights = null;
      save(COLUMN_KEY, null);
      apply();
      ctx.onResize?.();
    });
    splitter.addEventListener("keydown", (e) => {
      if ((e.key !== "ArrowUp" && e.key !== "ArrowDown") || ctx.expanded) return;
      const [upper, lower] = pair();
      if (!upper || !lower) return;
      e.preventDefault();
      weights = { ...measure(), sized: Boolean(weights?.sized) };
      move({ upper, lower, total: weights[upper] + weights[lower], ...limits(upper, lower) }, weights[upper] + (e.key === "ArrowUp" ? -24 : 24));
      save(COLUMN_KEY, JSON.stringify(weights));
      ctx.onResize?.();
    });
  }

  /** The line between `upper` and `lower` to `height` (of `upper`). */
  function move(d, height) {
    const a = Math.max(d.minUpper, Math.min(d.total - d.minLower, height));
    weights[d.upper] = a;
    weights[d.lower] = d.total - a;
    // The server's output shows once there's room for it.
    if (weights.server != null) weights.sized = weights.server >= d.own + SERVER_LOG_ROOM;
    apply();
  }

  // Under the maps: the maps and the server card.
  attach($("splitter"), () => ["maps", shown("server") ? "server" : null]);
  return { apply };
}

export function initLayout(ctx) {
  const maps = document.querySelector(".maps-card");
  const column = columnSplit(ctx);
  const splits = [
    // Simple view.
    column,
    // Maps page with more options: the console under the maps.
    heightSplit(ctx, {
      splitter: $("splitter"),
      top: () => maps,
      bottom: () => $("console-card"),
      key: () => "expanded",
      minTop: 150,
      minBottom: 120,
      active: () => ctx.expanded,
    }),
  ];
  let maximized = null;

  /** Lay out again (view switched, the server card came or went). */
  function apply() {
    for (const split of splits) split.apply();
    scheduleFit();
  }

  // ---------------------------------------------------------------- room for what's shown
  // A card that grows (an update's progress, a hint about the server) gets the
  // room it needs from the card that can give it (the map list, SteamCMD's output) instead of
  // getting a scroll bar. Nothing is remembered: when it gets smaller, it gives the room back.
  const fits = [
    { card: () => $("strip"), giver: () => maps, when: () => !ctx.expanded },
    { card: () => $("server-panel"), giver: () => maps, when: () => !ctx.expanded },
    { card: () => $("server-card"), giver: () => $("log-card"), when: () => ctx.expanded },
  ];

  function fit() {
    for (const f of fits) f.card().style.minHeight = "";
    for (const f of fits) {
      const card = f.card();
      const giver = f.giver();
      if (!f.when() || card.hidden || !card.offsetParent || !giver.offsetParent) continue;
      const have = card.offsetHeight;
      const need = card.scrollHeight + (card.offsetHeight - card.clientHeight);
      if (need <= have) continue;
      const room = giver.offsetHeight - (Number.parseFloat(getComputedStyle(giver).minHeight) || 120);
      if (room >= 1) card.style.minHeight = `${Math.min(need, have + room)}px`;
    }
  }

  let fitQueued = false;
  function scheduleFit() {
    if (fitQueued) return;
    fitQueued = true;
    requestAnimationFrame(() => {
      fitQueued = false;
      fit();
    });
  }
  // Not for the server's output (it scrolls inside its box, and lines come many times a second
  // while a server starts) or the clocks.
  const quiet = (node) => Boolean((node.nodeType === Node.ELEMENT_NODE ? node : node.parentElement)?.closest("#console, .status-time, .hint-time"));
  const changes = new MutationObserver((records) => {
    if (records.some((r) => !quiet(r.target))) scheduleFit();
  });
  for (const f of fits) {
    changes.observe(f.card(), { childList: true, subtree: true, characterData: true, attributes: true, attributeFilter: ["hidden", "class"] });
  }
  window.addEventListener("resize", scheduleFit);

  // ---------------------------------------------------------------- side column width
  const root = document.documentElement;
  function setSide(width) {
    if (width == null) root.style.removeProperty("--side-w");
    else root.style.setProperty("--side-w", `${Math.round(width)}px`);
  }
  setSide(load(SIDE_KEY));

  /** The widest the side may be in this layout (the main column keeps MIN_MAIN). */
  function maxSide(layout, handle) {
    const style = getComputedStyle(layout);
    const inner = layout.clientWidth - Number.parseFloat(style.paddingLeft) - Number.parseFloat(style.paddingRight);
    return Math.max(MIN_SIDE, inner - handle.getBoundingClientRect().width - MIN_MAIN);
  }

  for (const handle of document.querySelectorAll(".vsplitter")) {
    const layout = handle.parentElement;
    const side = () => layout.querySelector(".col-side");
    let drag = null;
    handle.addEventListener("pointerdown", (e) => {
      if (e.button !== 0) return;
      drag = { x: e.clientX, width: side().getBoundingClientRect().width, max: maxSide(layout, handle), last: null };
      handle.setPointerCapture(e.pointerId);
      handle.classList.add("dragging");
      document.body.classList.add("resizing-x");
      e.preventDefault();
    });
    handle.addEventListener("pointermove", (e) => {
      if (!drag) return;
      // The side column is on the right: dragging left makes it wider.
      drag.last = Math.max(MIN_SIDE, Math.min(drag.max, drag.width - (e.clientX - drag.x)));
      setSide(drag.last);
    });
    const end = () => {
      if (!drag) return;
      if (drag.last != null) save(SIDE_KEY, Math.round(drag.last));
      drag = null;
      handle.classList.remove("dragging");
      document.body.classList.remove("resizing-x");
      ctx.onResize?.();
    };
    handle.addEventListener("pointerup", end);
    handle.addEventListener("pointercancel", end);
    handle.addEventListener("dblclick", () => {
      save(SIDE_KEY, null);
      setSide(null);
      ctx.onResize?.();
    });
    handle.addEventListener("keydown", (e) => {
      if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
      e.preventDefault();
      const width = Math.max(MIN_SIDE, Math.min(maxSide(layout, handle), side().getBoundingClientRect().width + (e.key === "ArrowLeft" ? 24 : -24)));
      save(SIDE_KEY, Math.round(width));
      setSide(width);
      ctx.onResize?.();
    });
  }

  // ---------------------------------------------------------------- full screen
  function setMaximized(id) {
    maximized = id;
    for (const card of document.querySelectorAll(".card.maximized")) card.classList.remove("maximized");
    if (id) $(id).classList.add("maximized");
    document.body.classList.toggle("has-max", Boolean(id));
    scheduleFit();
    for (const button of document.querySelectorAll(".max-btn")) {
      const on = button.dataset.max === id;
      button.replaceChildren(icon(on ? "collapse" : "expand"));
      button.title = on ? t("common.normalSize") : t("common.fullScreen");
    }
    ctx.onResize?.();
  }

  document.addEventListener("click", (e) => {
    const button = e.target.closest(".max-btn");
    if (button) setMaximized(maximized === button.dataset.max ? null : button.dataset.max);
  });
  // Not while a window on top takes the Escape (it closes that one). Looked at first (capture),
  // before that window's own handler closes it.
  document.addEventListener(
    "keydown",
    (e) => {
      const onTop = ["modal", "setup", "app-update", "about", "map-viewer", "settings"].some((id) => !$(id).hidden);
      if (e.key === "Escape" && maximized && !onTop && !ctx.kits?.takesEscape()) setMaximized(null);
    },
    true,
  );

  return { apply, setMaximized, maximized: () => maximized, fit: scheduleFit };
}
