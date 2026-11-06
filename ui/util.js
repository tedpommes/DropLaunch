// Small helpers shared by the UI modules.
import { t, tn, fmtNum, language } from "./i18n.js";

export const { invoke, Channel } = window.__TAURI__.core;

export const $ = (id) => document.getElementById(id);

/** el("div", { class: "x", text: "hi" }, child...) */
export function el(tag, props = {}, ...children) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(props)) {
    if (value == null || value === false) continue;
    if (key === "class") node.className = value;
    else if (key === "text") node.textContent = value;
    else if (key.startsWith("on")) node.addEventListener(key.slice(2), value);
    else if (key === "dataset") Object.assign(node.dataset, value);
    else node.setAttribute(key, value === true ? "" : value);
  }
  node.append(...children.flat().filter((c) => c != null && c !== false));
  return node;
}

export const icon = (name, cls) => {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  if (cls) svg.setAttribute("class", cls);
  const use = document.createElementNS("http://www.w3.org/2000/svg", "use");
  use.setAttribute("href", `#i-${name}`);
  svg.append(use);
  return svg;
};

/** A path, coloured so it stands out in a sentence. */
export const pathChip = (path) => el("span", { class: "path", text: path, title: path });

export function fmtBytes(n) {
  const units = t("units.bytes").split(" ");
  if (!n) return `0 ${units[0]}`;
  const i = Math.min(units.length - 1, Math.floor(Math.log(n) / Math.log(1024)));
  const v = n / 1024 ** i;
  return `${fmtNum(v, i === 0 || v >= 100 ? 0 : v >= 10 ? 1 : 2)} ${units[i]}`;
}

export function fmtDate(secs) {
  if (!secs) return "";
  const date = new Date(secs * 1000);
  const sameYear = date.getFullYear() === new Date().getFullYear();
  return date.toLocaleDateString(language.dateLocale, sameYear ? { day: "numeric", month: "short" } : { day: "numeric", month: "short", year: "numeric" });
}

/** "today 14:02", "yesterday 09:10", "3 Oct" */
export function fmtWhen(secs) {
  if (!secs) return "";
  const date = new Date(secs * 1000);
  const time = date.toLocaleTimeString(language.dateLocale, { hour: "2-digit", minute: "2-digit", hour12: false });
  const today = new Date();
  const yesterday = new Date(Date.now() - 86400000);
  if (date.toDateString() === today.toDateString()) return t("time.today", { time });
  if (date.toDateString() === yesterday.toDateString()) return t("time.yesterday", { time });
  return fmtDate(secs);
}

export function fmtAgo(secs) {
  const d = Math.max(0, Date.now() / 1000 - secs);
  if (d < 60) return t("time.justNow");
  if (d < 3600) return t("time.minutesAgo", { n: Math.floor(d / 60) });
  if (d < 86400) return t("time.hoursAgo", { n: Math.floor(d / 3600) });
  return tn("time.daysAgo", Math.floor(d / 86400));
}

export function fmtDuration(s) {
  s = Math.max(0, Math.round(s));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  if (h) return `${h}:${String(m).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}`;
  return m ? `${m}:${String(s % 60).padStart(2, "0")}` : t("time.seconds", { n: s });
}

export const fileName = (path) => (path ? path.split(/[\\/]/).pop() : "");
export const buildNum = (id) => Number.parseInt(id, 10) || 0;

let toastTimer = null;
export function toast(message, kind = "") {
  const box = $("toast");
  box.textContent = message;
  box.className = `toast ${kind}`.trim();
  box.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (box.hidden = true), Math.min(9000, 3500 + message.length * 40));
}

/**
 * Ask something. `choices` are extra buttons ({ label, value, primary }); resolves with the
 * chosen value, or false for Cancel / Escape. `cancel: false`: no Cancel button (Escape still).
 */
export function confirmDialog({ title, text, ok = t("common.continue"), choices, cancel: withCancel = true }) {
  return new Promise((resolve) => {
    $("modal-title").textContent = title;
    // Text, or text and nodes (a coloured download size).
    $("modal-text").replaceChildren(...[text].flat());
    const actions = $("modal-actions");
    const cancel = el("button", { class: "btn btn-ghost", text: t("common.cancel") });
    const buttons = (choices ?? [{ label: ok, value: true, primary: true }]).map((c) =>
      el("button", { class: `btn ${c.danger ? "btn-danger" : c.primary ? "btn-primary" : "btn-ghost"}`, text: c.label, onclick: () => done(c.value) }),
    );
    actions.replaceChildren(...(withCancel ? [cancel] : []), ...buttons);
    const backdrop = $("modal");
    backdrop.hidden = false;
    const onKey = (e) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        done(false);
      }
    };
    document.addEventListener("keydown", onKey, true);
    function done(value) {
      backdrop.hidden = true;
      document.removeEventListener("keydown", onKey, true);
      resolve(value);
    }
    cancel.onclick = () => done(false);
    (buttons.find((b) => b.classList.contains("btn-primary")) ?? buttons[0]).focus();
  });
}

// ---------------------------------------------------------------- windows over the page
// The setup, welcome, Settings, update, map viewer, About and question windows (role="dialog"
// aria-modal="true", shown by taking `hidden` off them or their overlay): one that comes up
// without taking the focus itself gets it on its first control, Tab stays in the top one (and in
// a guide tip over it), and when one closes, the focus goes back to where it was.
const MODAL = '[aria-modal="true"]';
const FOCUSABLE = 'a[href], button, input, select, textarea, [tabindex]:not([tabindex="-1"])';
/** dialog → what had the focus before it came into the dialog. */
const focusBefore = new Map();
let shownBefore = new Set();

const shownDialogs = () => [...document.querySelectorAll(MODAL)].filter((d) => !d.closest("[hidden]"));
const focusables = (root) => [...root.querySelectorAll(FOCUSABLE)].filter((n) => !n.disabled && n.tabIndex >= 0 && n.getClientRects().length > 0);
const visibleTips = () => [...document.querySelectorAll(".guide-tip")].filter((tip) => tip.getClientRects().length > 0);

/**
 * A window that just came up: once its own code had its turn (most focus their main button
 * themselves), the focus is in it or in a guide tip, or its first control gets it. So a keyboard
 * user doesn't need a Tab first, and a screen reader starts in it.
 */
function focusShown(dialog) {
  setTimeout(() => {
    if (shownDialogs().at(-1) !== dialog) return; // closed again, or another came over it
    const at = document.activeElement;
    if ([dialog, ...visibleTips()].some((area) => area.contains(at))) return;
    focusables(dialog)[0]?.focus({ preventScroll: true });
  });
}

document.addEventListener("focusin", (e) => {
  const dialog = e.target.closest?.(MODAL);
  const from = e.relatedTarget;
  if (dialog && from && !dialog.contains(from) && !focusBefore.has(dialog)) focusBefore.set(dialog, from);
});

new MutationObserver(() => {
  const shown = new Set(shownDialogs());
  for (const dialog of shownBefore) {
    if (shown.has(dialog)) continue;
    const back = focusBefore.get(dialog);
    focusBefore.delete(dialog);
    const at = document.activeElement;
    // Unless something else took the focus already (Settings gives it to its button itself).
    if (back?.isConnected && (!at || at === document.body || dialog.contains(at))) back.focus({ preventScroll: true });
  }
  for (const dialog of shown) if (!shownBefore.has(dialog)) focusShown(dialog);
  shownBefore = shown;
}).observe(document.body, { subtree: true, attributes: true, attributeFilter: ["hidden"] });

document.addEventListener("keydown", (e) => {
  if (e.key !== "Tab" || e.defaultPrevented) return;
  const top = shownDialogs().at(-1); // the later one in the page is the one in front
  if (!top) return;
  const tips = visibleTips();
  const items = [top, ...tips].flatMap(focusables);
  const at = document.activeElement;
  const inside = [top, ...tips].some((area) => area.contains(at));
  if (!items.length) return e.preventDefault();
  const [first, last] = [items[0], items.at(-1)];
  if (!inside || (!e.shiftKey && at === last) || (e.shiftKey && at === first)) {
    e.preventDefault();
    (e.shiftKey ? last : first).focus();
  }
});

/** "C:" for a Windows path (null when it can't tell). */
export function driveOf(path) {
  const match = /^([A-Za-z]:)/.exec(path ?? "");
  return match ? match[1].toUpperCase() : null;
}

/**
 * The ways to write `path` shorter, longest first: the drive and the first folder stay, then "…",
 * then as many of the last folders as there's room for (C:\Users\…\Maps\map.map).
 */
function pathShortenings(path) {
  const sep = path.includes("\\") ? "\\" : "/";
  const parts = path.split(/[\\/]/);
  const out = [path];
  if (parts.length < 4) return out;
  for (let keep = parts.length - 3; keep >= 1; keep--) out.push([...parts.slice(0, 2), "…", ...parts.slice(-keep)].join(sep));
  out.push([parts[0], "…", parts.at(-1)].join(sep));
  return out;
}

const fitted = new WeakMap();
const fitObserver =
  typeof ResizeObserver === "function"
    ? new ResizeObserver((entries) => {
        for (const entry of entries) for (const node of fitted.get(entry.target)?.nodes ?? [entry.target]) fitNow(node);
      })
    : null;
/**
 * What the observer watches. An observer keeps its elements alive, and one taken off the page
 * gets no more callbacks, so `sweep` lets go of the ones a re-render replaced.
 */
const watching = new Set();
/** A label still being built is off the page for a moment at most. */
const BUILDING_MS = 1000;

function watch(node) {
  if (!fitObserver || watching.has(node)) return;
  watching.add(node);
  fitObserver.observe(node);
}

function unwatch(node) {
  fitObserver?.unobserve(node);
  watching.delete(node);
}

/**
 * Off the page for good: it was laid out once (so it was on the page), or it was made more than a
 * second ago and never put on it (replaced before its first layout). Not one still being built.
 */
function gone(node, now) {
  if (node.isConnected) return false;
  const job = fitted.get(node);
  return Boolean(job?.seen) || now - (job?.at ?? 0) > BUILDING_MS;
}

/** Lets go of what was on the page and isn't any more. */
function sweep() {
  const now = performance.now();
  for (const node of watching) {
    const job = fitted.get(node);
    if (gone(node, now)) unwatch(node);
    else if (job?.nodes) job.nodes = job.nodes.filter((n) => !gone(n, now));
  }
}

function fitNow(node) {
  const job = fitted.get(node);
  if (!job?.path) return;
  if (!node.isConnected) {
    unwatch(node);
    return;
  }
  job.seen = true;
  if (!node.clientWidth) return;
  // The box that limits the width, when the node itself is only as wide as its text.
  const box = job.box?.(node);
  if (box) {
    const watched = fitted.get(box);
    if (!watched) {
      fitted.set(box, { nodes: [node], seen: true });
      watch(box);
    } else if (!watched.nodes.includes(node)) watched.nodes.push(node);
  }
  for (const text of pathShortenings(job.path)) {
    node.textContent = job.before + text;
    if (node.scrollWidth <= node.clientWidth) return;
  }
}

/**
 * Shows `path` (after `before`) on one line in `node`: when it doesn't fit, it gives way in the
 * middle instead of at an end, and again whenever the room changes. `node` needs overflow hidden
 * and nowrap (its text-overflow ellipsis is the last resort). `box(node)`: the element that limits it.
 */
export function fitPath(node, path, { before = "", box } = {}) {
  sweep();
  fitted.set(node, { path, before, box, at: performance.now() });
  node.textContent = before + path;
  watch(node);
  return node;
}

/**
 * A download's size ("about 6 GB") in green when its drive has room for it (`need` bytes), in red
 * when it doesn't, then with how much is free. `free` unknown: plain text.
 */
export function sizeTag(label, { free, need, path } = {}) {
  if (free == null) return el("span", { text: label });
  const ok = free >= need;
  const drive = driveOf(path) ?? t("units.thatDrive");
  return el("span", {
    class: `size-tag ${ok ? "room" : "no-room"}`,
    title: ok ? t("units.freeOn", { free: fmtBytes(free), drive }) : t("units.notEnough", { drive, free: fmtBytes(free), need: fmtBytes(need) }),
    text: ok ? label : t("units.onlyFree", { size: label, free: fmtBytes(free) }),
  });
}

/** The Rust game a server of this branch needs (as client.rs decides): its name for buttons. */
export function rustGameFor(branch) {
  const rust = !branch || ["public", "release", "last-month", "last-last-month"].includes(branch);
  return rust ? { name: "Rust", button: t("join.button.rust") } : { name: "Rust - Staging Branch", button: t("join.button.staging") };
}

/**
 * What Join found (the backend's JoinPlan, for the server on this PC): the right Rust game is
 * starting, or it's open already, the other one is open (Rust vs Rust - Staging
 * Branch: a client only joins a server of its own build), or it isn't installed. The F1 command
 * is on the clipboard either way.
 */
export async function afterJoin(plan) {
  await navigator.clipboard.writeText(plan.connect).catch(() => {});
  if (plan.note) toast(plan.note, "warn");
  const vars = { game: plan.game, running: plan.runningGame, branch: plan.branch, connect: plan.connect };
  if (plan.state === "started") {
    if (!plan.note) toast(t("join.started", vars));
  } else if (plan.state === "running") {
    await rustIsOpen(plan);
  } else if (plan.state === "wrong") {
    const ok = await confirmDialog({ title: t("join.wrong.title", vars), text: t("join.wrong.text", vars), ok: t("join.wrong.ok", vars) });
    if (ok) restartInto(plan);
  } else if (plan.state === "missing") {
    const ok = await confirmDialog({
      title: t("join.missing.title", vars),
      text: t(plan.appId === 700580 ? "join.missing.textStaging" : "join.missing.text", vars),
      ok: t("join.missing.ok"),
    });
    if (ok) invoke("install_rust_game", { branch: plan.branch }).catch((err) => toast(String(err), "error"));
  }
}

/** The right Rust is open: bring it up for F1 + paste, or restart it straight into the server. */
async function rustIsOpen(plan) {
  const choice = await confirmDialog({
    title: t("join.open.title", { game: plan.game }),
    text: t("join.open.text", { connect: plan.connect }),
    choices: [
      { label: t("join.open.switch"), value: "show" },
      { label: t("join.open.restart"), value: "restart", primary: true },
    ],
  });
  if (choice === "show") {
    const shown = await invoke("show_rust", { branch: plan.branch }).catch(() => false);
    toast(t(shown ? "join.open.shown" : "join.open.notShown"));
  } else if (choice === "restart") {
    restartInto(plan);
  }
}

async function restartInto(plan) {
  toast(t("join.closing", { game: plan.game }));
  try {
    await invoke("restart_rust_and_join", { port: plan.port, branch: plan.branch });
    toast(t("join.restarted", { game: plan.game }), "ok");
  } catch (err) {
    toast(String(err), "error");
  }
}

/** `what`: what was copied, as the start of the sentence ("Console", "Map path"). */
export async function copyText(text, what) {
  try {
    await navigator.clipboard.writeText(text);
    toast(t("common.copied", { what }));
  } catch {
    toast(t("common.copyFailed"));
  }
}

/**
 * Keeps a log box scrolled to the newest line unless the reader scrolled up. Works while the
 * box is hidden (another tab): call reveal() when it's shown again.
 */
export function followScroll(box) {
  let follow = true;
  box.addEventListener("scroll", () => {
    if (box.offsetParent) follow = box.scrollHeight - box.scrollTop - box.clientHeight < 48;
  });
  const stick = () => {
    if (follow) box.scrollTop = box.scrollHeight;
  };
  // Moved or resized: follow the newest line again.
  const bottom = () => {
    follow = true;
    requestAnimationFrame(() => (box.scrollTop = box.scrollHeight));
  };
  return { after: stick, reveal: stick, bottom };
}

/** Speed + time-left estimate for one progress stream. */
export class Rate {
  reset() {
    this.key = null;
  }
  describe(text, done, total) {
    const now = performance.now() / 1000;
    const key = `${text}|${total}`;
    if (this.key !== key || done < this.done) {
      Object.assign(this, { key, done, t: now, speed: null });
    } else if (now - this.t > 0.5) {
      const instant = (done - this.done) / (now - this.t);
      this.speed = this.speed == null ? instant : this.speed * 0.5 + instant * 0.5;
      this.done = done;
      this.t = now;
    }
    if (!total) return "";
    const parts = [t("units.progress", { done: fmtBytes(done), total: fmtBytes(total) })];
    if (this.speed > 0) parts.push(t("units.speed", { speed: fmtBytes(this.speed) }), t("units.timeLeft", { time: fmtDuration((total - done) / this.speed) }));
    return parts.join("  ·  ");
  }
}

// ---------------------------------------------------------------- Windows notifications
const clickHandlers = new Map();
window.__TAURI__.event.listen("notification-clicked", (e) => clickHandlers.get(e.payload.tag)?.(e.payload.action));

/**
 * A Windows notification, only while DropLaunch is in the background (in front, the window
 * shows it). `action` is what a click does, `buttons` are [label, action] pairs; the click comes
 * back to the handler registered for `tag` with onNotificationClick.
 */
export function desktopNotify({ tag, title, body, action, buttons = [] }) {
  if (document.hasFocus()) return;
  invoke("notify", { toast: { tag, title, body, action, buttons } }).catch(() => {});
}

export function onNotificationClick(tag, handler) {
  clickHandlers.set(tag, handler);
}
