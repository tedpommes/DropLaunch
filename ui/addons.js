// Add-ons (addons.rs; Settings → Add-ons): pages someone made for DropLaunch, each in a sandboxed
// frame: a tab next to Maps and Server files, or a card in More options' side column.
//
// A frame can't reach this page or Tauri: it only sends messages (addons/_sdk/droplaunch.js), and
// this passes them on as `addon_call` with the add-on they came from (the backend checks them
// against its addon.json), except the ones about this window: toasts, the clipboard, links (asked
// first unless addon.json lists the site), its tab's badge. Events go the other way: the server's
// state, console lines, players, chat and bug reports to add-ons that may see them, the theme when
// it changes, whether a page can be seen, and a setting another of its pages saved.
import { $, el, icon, invoke, toast, confirmDialog, copyText, pointAt } from "./util.js";
import { t, tn, language } from "./i18n.js";
import { themes, cssVars } from "./theme.js";

const API = 1;
const FOLDED_KEY = "droplaunch.addonCardsFolded";
/** Where "How to make one" goes. */
export const ADDON_GUIDE = "https://github.com/tedpommes/DropLaunch/blob/main/addons/README.md";
const TOAST_KINDS = ["ok", "warn", "error"];
/** Each permission (addons.rs PERMISSIONS) in words: [what it lets an add-on do, a short name]. */
const PERMISSION_TEXTS = {
  "server:read": ["addons.perm.server.read", "addons.permShort.server.read"],
  "server:control": ["addons.perm.server.control", "addons.permShort.server.control"],
  "console:read": ["addons.perm.console.read", "addons.permShort.console.read"],
  rcon: ["addons.perm.rcon", "addons.permShort.rcon"],
  kits: ["addons.perm.kits", "addons.permShort.kits"],
  "maps:read": ["addons.perm.maps.read", "addons.permShort.maps.read"],
  "reports:read": ["addons.perm.reports.read", "addons.permShort.reports.read"],
  "reports:write": ["addons.perm.reports.write", "addons.permShort.reports.write"],
  "configs:read": ["addons.perm.configs.read", "addons.permShort.configs.read"],
  "configs:write": ["addons.perm.configs.write", "addons.permShort.configs.write"],
  network: ["addons.perm.network", "addons.permShort.network"],
};
/** Ones that can change things on the server or reach out: shown first and marked. */
const STRONG = ["rcon", "server:control", "configs:write", "network"];
/** The calls this window answers itself (the rest are addons_list's `calls`). */
const WINDOW_CALLS = ["ui.toast", "ui.copy", "ui.openLink", "ui.notify", "ui.badge", "ui.show"];
/** Every event a page can hear. */
const EVENTS = ["server", "console", "players", "chat", "report", "reports", "theme", "visible", "storage"];
/** Calls one add-on may make in a second, all its pages together. */
const CALLS_PER_SECOND = 40;

export function initAddons(ctx) {
  /** id → { addon, parts: [{ ui, frame, tab, view, card, name }] } */
  const running = new Map();
  /** What addons_list said: { addons, folder, api }. */
  let state = { addons: [], folder: null, api: API };

  const has = (addon, permission) => !permission || addon.permissions.includes(permission);
  const send = (frame, message) => frame.contentWindow?.postMessage({ droplaunch: API, ...message }, "*");
  const themeInfo = () => ({ mode: themes.current?.mode ?? "dark", vars: cssVars(themes.current) });

  // ---------------------------------------------------------------- limits
  // What needs no permission (messages, the clipboard, links, its tab) still mustn't let one
  // add-on take over the window: a few at a time, and the ones that act for the user (copy, open
  // a listed site, show its tab) only while the user is in the add-on's page.
  /** id → what it did lately: { asking, refused, toasts, notes, opened, copied, calls } (times in ms). */
  const usage = new Map();
  function used(addon) {
    if (!usage.has(addon.id)) usage.set(addon.id, { asking: false, refused: [], toasts: [], notes: [], opened: [], copied: [], calls: [] });
    return usage.get(addon.id);
  }
  /** One more in `times`, unless there were `max` in the last `ms` (then false). */
  function within(times, max, ms) {
    const now = Date.now();
    while (times.length && now - times[0] > ms) times.shift();
    if (times.length >= max) return false;
    times.push(now);
    return true;
  }
  /** The user is in one of the add-on's pages: the window has the focus, and it's in its frame. */
  const inUse = (addon) => document.hasFocus() && (running.get(addon.id)?.parts ?? []).some((p) => p.frame === document.activeElement);
  /** An Error with a code the add-on can check (addon_call's codes, and busy, notInUse, invalid). */
  const coded = (code, message) => Object.assign(new Error(message), { code });
  const tooOften = (addon) => coded("busy", t("addons.error.busy", { name: addon.name }));

  function folded() {
    try {
      return new Set(JSON.parse(localStorage.getItem(FOLDED_KEY)) ?? []);
    } catch {
      return new Set();
    }
  }

  // ---------------------------------------------------------------- frames
  /**
   * A second lock on where a frame can go. The window's frame-src (tauri.conf.json) already stops a
   * frame from sending itself to a website, before anything is sent. This policy (CSP Embedded
   * Enforcement) must be taken by every page in the frame too: add-on pages do (Allow-CSP-From,
   * addons.rs), others are blocked. Each page's own policy (from its permissions) is the strict one.
   */
  function framePolicy(addon) {
    const origin = addon.base.match(/^[a-z][a-z0-9+.-]*:\/\/[^/]+/i)?.[0] ?? location.origin;
    return [
      "default-src 'none'",
      `script-src ${origin}`,
      `style-src 'unsafe-inline' ${origin}`,
      `img-src ${origin} data: blob: https:`,
      `font-src ${origin} data:`,
      `media-src ${origin} data: blob:`,
      `connect-src ${origin} https: wss:`,
      "frame-src 'none'",
      "worker-src 'none'",
      "form-action 'none'",
      "base-uri 'none'",
    ].join("; ");
  }

  function frameFor(addon, ui) {
    return el("iframe", {
      class: "addon-frame",
      // Scripts, and forms that a script handles; no same origin (so no way into this page),
      // no popups, no top navigation, no alert/confirm (droplaunch.ui has its own).
      sandbox: "allow-scripts allow-forms",
      csp: framePolicy(addon),
      referrerpolicy: "no-referrer",
      title: `${addon.name}: ${ui.title}`,
      src: addon.base + ui.page,
    });
  }

  function addTab(addon, ui, n) {
    const name = `addon:${addon.id}:${n}`;
    const frame = frameFor(addon, ui);
    const tab = el(
      "button",
      { role: "tab", class: "addon-tab", "data-tab": name, "aria-selected": "false", title: `${ui.title} (${addon.name})` },
      icon(ui.icon ?? "puzzle"),
      el("span", { text: ui.title }),
      el("i", { class: "tab-badge", hidden: true }),
    );
    tab.addEventListener("click", () => ctx.setTab(name));
    document.querySelector(".tabs").append(tab);
    const view = el("main", { class: "view addon-view", "data-tab": name, hidden: true }, el("section", { class: "card addon-page" }, frame));
    ([...document.querySelectorAll(".addon-view")].pop() ?? $("view-files")).after(view);
    return { ui, frame, tab, view, name };
  }

  function addCard(addon, ui, n) {
    const id = `addon-card-${addon.id}-${n}`;
    const frame = frameFor(addon, ui);
    const fold = el("button", { class: "icon-btn fold-btn", title: t("common.fold"), "aria-expanded": "true" }, icon("chevron"));
    const card = el(
      "section",
      { class: "card foldable addon-card", id },
      el("div", { class: "card-head" }, el("span", { class: "addon-card-icon" }, icon(ui.icon ?? "puzzle")), el("h2", { text: ui.title }), el("span", { class: "spacer" }), fold),
      el("div", { class: "addon-card-body", style: `height: ${ui.height ?? 260}px` }, frame),
    );
    const setFolded = (on) => {
      card.classList.toggle("folded", on);
      fold.setAttribute("aria-expanded", String(!on));
      fold.title = on ? t("common.unfold") : t("common.fold");
      updateVisible();
    };
    setFolded(folded().has(id));
    fold.addEventListener("click", () => {
      const now = !card.classList.contains("folded");
      const all = folded();
      if (now) all.add(id);
      else all.delete(id);
      try {
        localStorage.setItem(FOLDED_KEY, JSON.stringify([...all]));
      } catch {}
      setFolded(now);
    });
    $("bugs-card").parentElement.append(card);
    seen.observe(card);
    return { ui, frame, card, name: null };
  }

  // ---------------------------------------------------------------- can it be seen?
  // A page hears `visible` (true or false) when that changes, so a card can stop asking for
  // things nobody looks at: a tab while it's the one in front, a card while it's unfolded and on
  // screen (not scrolled away, nor in the simple view); neither while the window is minimised or
  // Settings covers it.
  const onScreen = new WeakMap();
  const seen = new IntersectionObserver((entries) => {
    for (const e of entries) onScreen.set(e.target, e.isIntersecting);
    updateVisible();
  });

  function isVisible(part) {
    if (document.hidden || !$("settings").hidden) return false;
    if (part.view) return !part.view.hidden;
    return !part.card.classList.contains("folded") && onScreen.get(part.card) === true;
  }

  function updateVisible() {
    for (const r of running.values()) {
      for (const p of r.parts) {
        const now = isVisible(p);
        if (p.visible === now) continue;
        p.visible = now;
        if (p.started) send(p.frame, { type: "event", name: "visible", data: now });
      }
    }
  }
  document.addEventListener("visibilitychange", updateVisible);
  new MutationObserver(updateVisible).observe($("settings"), { attributes: true, attributeFilter: ["hidden"] });

  function start(addon) {
    const parts = addon.ui.map((ui, n) => (ui.kind === "tab" ? addTab(addon, ui, n) : addCard(addon, ui, n)));
    running.set(addon.id, { addon, parts });
    for (const part of parts) watchStart(addon, part);
  }

  // ---------------------------------------------------------------- a page that didn't start
  /** How long a loaded page has to say hello (the SDK does it at once). */
  const START_GRACE = 5000;

  /**
   * A page that loaded but hasn't said hello by then didn't start (its file is missing, or its
   * script broke first): a note in its place, with Reload, instead of an empty tab. (The SDK says
   * hello while the page loads, so usually before its load event: `started` is only reset when
   * DropLaunch loads the page again.)
   */
  function watchStart(addon, part) {
    part.frame.addEventListener("load", () => {
      clearTimeout(part.startTimer);
      if (part.started) return;
      part.startTimer = setTimeout(() => {
        if (!part.started && running.get(addon.id)?.parts.includes(part)) showStuck(addon, part);
      }, START_GRACE);
    });
  }

  function showStuck(addon, part) {
    part.stuck?.remove();
    part.stuck = el(
      "div",
      { class: "addon-stuck" },
      el("p", { text: t("addons.stuck", { name: addon.name }) }),
      el("p", { class: "addon-note", text: addon.linked ? t("addons.stuckLinked") : t("addons.stuckHint") }),
      el("button", { class: "btn btn-small btn-ghost", onclick: () => reload(addon.id) }, icon("refresh"), el("span", { text: t("addons.reloadPage") })),
    );
    part.frame.after(part.stuck);
  }

  function started(part) {
    part.started = true;
    clearTimeout(part.startTimer);
    part.stuck?.remove();
    part.stuck = null;
  }

  function stop(id) {
    const r = running.get(id);
    if (!r) return;
    running.delete(id);
    const shown = r.parts.some((p) => p.view && !p.view.hidden);
    for (const p of r.parts) {
      p.tab?.remove();
      p.view?.remove();
      if (p.card) seen.unobserve(p.card);
      p.card?.remove();
    }
    if (shown) ctx.setTab("maps");
  }

  /** The list again: frames for add-ons turned on or changed, gone for ones turned off or removed. */
  async function refresh({ restart } = {}) {
    try {
      state = await invoke("addons_list");
    } catch (err) {
      toast(String(err), "error");
      return state;
    }
    const usable = state.addons.filter((a) => a.on && !a.problem);
    for (const [id, r] of running) {
      const now = usable.find((a) => a.id === id);
      if (!now || id === restart || JSON.stringify(now) !== JSON.stringify(r.addon)) stop(id);
    }
    for (const addon of usable) if (!running.has(addon.id)) start(addon);
    return state;
  }

  /** An add-on's frames load again (after its files changed). */
  function reload(id) {
    for (const p of running.get(id)?.parts ?? []) {
      p.stuck?.remove();
      p.stuck = null;
      p.started = false;
      p.frame.src = p.frame.src;
    }
  }

  /** Its tab (the first one), in More options. */
  function show(id) {
    const part = running.get(id)?.parts.find((p) => p.name);
    if (!part) return false;
    ctx.setExpanded(true);
    ctx.setTab(part.name);
    return true;
  }

  /** One of its pages: its tab, or its card in more options (pointed at). */
  function showPart(part) {
    if (!part?.frame?.isConnected) return;
    ctx.settings?.close();
    if (part.name) {
      ctx.setExpanded(true);
      ctx.setTab(part.name);
    } else if (part.card) {
      ctx.setExpanded(true);
      ctx.setTab("maps");
      pointAt(part.card);
    }
  }

  /** An add-on's first page (its tab, else its card); one without pages: its row in Settings. */
  function showAddon(id) {
    const part = running.get(id)?.parts.find((p) => p.name) ?? running.get(id)?.parts[0];
    if (part) showPart(part);
    else inSettings(id);
  }

  /** Its row in Settings → Add-ons. */
  const inSettings = (id) => ctx.settings.open("addons", { point: `.addon-row[data-addon="${CSS.escape(id)}"]` });

  /** setTab says which tab shows now: add-on pages hear whether they're visible. */
  function tabChanged() {
    updateVisible();
  }

  // ---------------------------------------------------------------- messages from frames
  function partOf(source) {
    if (!source) return null;
    for (const r of running.values()) for (const p of r.parts) if (p.frame.contentWindow === source) return { addon: r.addon, part: p };
    return null;
  }

  window.addEventListener("message", async (e) => {
    const m = e.data;
    if (!m || typeof m !== "object" || m.droplaunch !== API) return;
    const found = partOf(e.source);
    if (!found) return;
    const { addon, part } = found;
    if (m.type === "hello") {
      started(part);
      part.visible = isVisible(part);
      send(part.frame, {
        type: "init",
        info: {
          addon: { id: addon.id, name: addon.name, version: addon.version, permissions: addon.permissions },
          app: { version: state.version ?? null, api: state.api },
          language: language.code,
          theme: themeInfo(),
          visible: part.visible,
          // What this DropLaunch has (droplaunch.has): calls and events.
          calls: [...WINDOW_CALLS, ...(state.calls ?? [])],
          events: EVENTS,
        },
      });
      return;
    }
    if (m.type === "error") {
      // A folder being made: its maker sees the page's script errors (a few at a time).
      if (addon.linked && within(used(addon).toasts, 5, 10000)) toast(t("addons.scriptError", { name: addon.name, error: String(m.text ?? "").slice(0, 300) }), "error", () => inSettings(addon.id));
      return;
    }
    if (m.type !== "call" || typeof m.name !== "string") return;
    let reply;
    try {
      reply = { ok: true, value: (await handle(addon, part, m.name, m.args && typeof m.args === "object" ? m.args : {})) ?? null };
    } catch (err) {
      // addon_call's errors are { code, message }; the window's own are Errors with a code.
      reply = { ok: false, error: String(err?.message ?? err), code: typeof err?.code === "string" ? err.code : "failed" };
    }
    send(part.frame, { type: "reply", id: m.id, ...reply });
  });

  async function handle(addon, part, name, args) {
    const text = (key, max = 2000) => String(args[key] ?? "").slice(0, max);
    const u = used(addon);
    // A page that calls without end doesn't keep DropLaunch busy.
    if (!within(u.calls, CALLS_PER_SECOND, 1000)) throw tooOften(addon);
    switch (name) {
      case "ui.toast":
        if (!within(u.toasts, 5, 10000)) throw tooOften(addon);
        // A click on it shows the page that said it.
        toast(`${addon.name}: ${text("text", 300)}`, TOAST_KINDS.includes(args.kind) ? args.kind : "", () => showPart(part));
        return null;
      case "ui.copy":
        // The clipboard is the user's: only when they're in the add-on (they pressed its Copy).
        if (!inUse(addon)) throw coded("notInUse", t("addons.error.notInUse", { name: addon.name }));
        if (!within(u.copied, 10, 10000)) throw tooOften(addon);
        await copyText(text("text", 100000), `${addon.name}: ${text("what", 60) || t("addons.copied")}`);
        return null;
      case "ui.openLink":
        return openLink(addon, text("url"));
      case "ui.notify":
        if (!within(u.notes, 3, 60000)) throw tooOften(addon);
        return invoke("notify", { toast: { tag: `addon-${addon.id}`, title: `${addon.name}: ${text("title", 100)}`, body: text("body", 300), action: "", buttons: [] } });
      case "ui.badge":
        for (const p of running.get(addon.id)?.parts ?? []) {
          const badge = p.tab?.querySelector(".tab-badge");
          if (!badge) continue;
          badge.textContent = args.text == null ? "" : text("text", 6);
          badge.hidden = args.text == null || args.text === "";
        }
        return null;
      case "ui.show":
        // Its tab comes to the front when the user asked for it there (a click in its card).
        return inUse(addon) ? show(addon.id) : false;
      default: {
        const value = await invoke("addon_call", { id: addon.id, call: name, args });
        // The add-on's other pages (a tab and its card) hear what one of them saved.
        if (name === "storage.set" || name === "storage.remove") changedStorage(addon, part, { [text("key", 200)]: name === "storage.set" ? (args.value ?? null) : null });
        else if (name === "storage.setMany" && args.entries && typeof args.entries === "object") changedStorage(addon, part, args.entries);
        return value;
      }
    }
  }

  /** Each saved key, to the add-on's pages other than the one that saved it. */
  function changedStorage(addon, from, entries) {
    for (const p of running.get(addon.id)?.parts ?? []) {
      if (p === from) continue;
      for (const [key, value] of Object.entries(entries)) send(p.frame, { type: "event", name: "storage", data: { key, value: value ?? null } });
    }
  }

  /**
   * A site from addon.json's `links` (the add dialog showed them) opens without a question while
   * the user is in the add-on's page, a few a minute. Anything else is asked first: one question
   * at a time per add-on, with a way to turn the add-on off right there, and after two "no" in a
   * minute it isn't asked again for a while. (An add-on that kept asking had the window locked.)
   */
  async function openLink(addon, url) {
    let address;
    try {
      address = new URL(url);
    } catch {
      throw coded("invalid", t("addons.link.bad"));
    }
    if (address.protocol !== "https:") throw coded("invalid", t("addons.link.bad"));
    const u = used(addon);
    const host = address.hostname;
    const listed = addon.links.some((h) => (h.startsWith("*.") ? host.endsWith(h.slice(1)) : host === h));
    if (!(listed && inUse(addon) && within(u.opened, 5, 60000))) {
      const now = Date.now();
      u.refused = u.refused.filter((at) => now - at < 60000);
      if (u.asking || u.refused.length >= 2) throw tooOften(addon);
      u.asking = true;
      let answer;
      try {
        answer = await confirmDialog({
          title: t("addons.link.title", { name: addon.name }),
          text: el("span", { class: "path", text: address.href }),
          choices: [
            { label: t("addons.link.off"), value: "off", danger: true },
            { label: t("addons.link.open"), value: true, primary: true },
          ],
        });
      } finally {
        u.asking = false;
      }
      if (answer === "off") {
        await setOn(addon.id, false);
        toast(t("addons.turnedOff", { name: addon.name }), "ok", () => inSettings(addon.id));
        return false;
      }
      if (!answer) {
        u.refused.push(Date.now());
        return false;
      }
    }
    await invoke("open_link", { url: address.href });
    return true;
  }

  // ---------------------------------------------------------------- events to frames
  function broadcast(name, data, permission) {
    for (const r of running.values()) {
      if (!has(r.addon, permission)) continue;
      for (const p of r.parts) send(p.frame, { type: "event", name, data });
    }
  }

  window.__TAURI__.event.listen("server", (e) => {
    const payload = e.payload;
    if (payload?.kind === "state") broadcast("server", payload.server ?? null, "server:read");
    else if (payload?.kind === "lines") broadcast("console", payload.lines, "console:read");
    // Who's on, who joined, who left (DropLaunch's one list, without IP addresses).
    else if (payload?.kind === "players") broadcast("players", { players: payload.players ?? [], joined: payload.joined ?? [], left: payload.left ?? [] }, "server:read");
    // Chat, one message at a time, as the server sent it (not read from console lines).
    else if (payload?.kind === "chat") for (const message of payload.messages ?? []) broadcast("chat", message, "console:read");
  });
  window.addEventListener("themechange", () => broadcast("theme", themeInfo(), null));

  // Bug reports: each new one, and the list whenever it changed (fixed, deleted, another map).
  window.__TAURI__.event.listen("bug-reports", (e) => {
    for (const report of e.payload ?? []) broadcast("report", report, "reports:read");
  });
  let reportsSent = null;
  window.__TAURI__.event.listen("playtest", (e) => {
    const view = e.payload;
    if (![...running.values()].some((r) => has(r.addon, "reports:read"))) return;
    const now = JSON.stringify([view?.map?.key ?? null, view?.reports ?? []]);
    if (now === reportsSent) return;
    reportsSent = now;
    broadcast("reports", { map: view?.map ?? null, reports: view?.reports ?? [] }, "reports:read");
  });

  // ---------------------------------------------------------------- adding one
  /** What a permission lets an add-on do, in words. */
  function permissionText(addon, permission) {
    const texts = PERMISSION_TEXTS[permission];
    const base = texts ? t(texts[0]) : permission;
    return permission === "network" && addon.network?.length ? `${base}: ${addon.network.join(", ")}` : base;
  }

  /** A permission's short name (Settings → Add-ons). */
  const permissionName = (permission) => (PERMISSION_TEXTS[permission] ? t(PERMISSION_TEXTS[permission][1]) : permission);

  /**
   * The add-on dialog's text: what it is, who made it, and what it may do. An update (`before`:
   * the one there now) marks what it may do that the old one couldn't, and says so when its name
   * or maker changed: it gets what the old one saved.
   */
  function describe(addon) {
    const perms = [...addon.permissions].sort((a, b) => STRONG.includes(b) - STRONG.includes(a));
    const before = addon.before;
    const added = (list, old) => Boolean(before) && (list ?? []).some((x) => !(old ?? []).includes(x));
    const isNew = (p) => added([p], before?.permissions) || (p === "network" && added(addon.network, before?.network));
    const otherMaker = before && (before.name !== addon.name || before.author !== addon.author);
    const permItem = (p) =>
      el("li", { class: [STRONG.includes(p) && "strong", isNew(p) && "new"].filter(Boolean).join(" ") || null, text: isNew(p) ? t("addons.newPermission", { what: permissionText(addon, p) }) : permissionText(addon, p) });
    const opens = addon.links?.length ? t("addons.opens", { sites: addon.links.join(", ") }) : null;
    return [
      otherMaker ? el("p", { class: "addon-warn", text: t("addons.replacesOther", { name: before.name, author: before.author || t("addons.noAuthor") }) }) : null,
      addon.description ? el("p", { text: addon.description }) : null,
      el("p", { class: "addon-by", text: [addon.author && t("addons.by", { author: addon.author }), t("addons.version", { version: addon.version })].filter(Boolean).join(" · ") }),
      el("p", { class: "addon-may", text: perms.length ? t("addons.may") : t("addons.mayNothing") }),
      perms.length ? el("ul", { class: "addon-perms" }, perms.map(permItem)) : null,
      // The sites it opens in the browser without asking (addon.json's links).
      opens ? el("p", { class: `addon-may${added(addon.links, before?.links) ? " new" : ""}`, text: added(addon.links, before?.links) ? t("addons.newPermission", { what: opens }) : opens }) : null,
      addon.linked ? el("p", { class: "addon-note", text: t("addons.linkedNote") }) : null,
      el("p", { class: "addon-note", text: t("addons.trust") }),
    ].filter(Boolean);
  }

  /** Ask, then add it; its tab shows once it's there. */
  async function offer(found) {
    const yes = await confirmDialog({
      title: found.replaces ? t("addons.add.update", { name: found.name, from: found.replaces, to: found.version }) : t("addons.add.title", { name: found.name }),
      text: describe(found),
      ok: found.replaces ? t("addons.add.updateButton") : t("addons.add.button"),
    });
    if (!yes) return null;
    try {
      const addon = await invoke("addon_install", { path: found.path });
      await refresh({ restart: addon.id });
      toast(t("addons.added", { name: addon.name }), "ok", () => showAddon(addon.id));
      show(addon.id);
      return addon;
    } catch (err) {
      toast(String(err), "error");
      return null;
    }
  }

  /** Dropped paths: add-ons among them are offered (one by one); the rest comes back. */
  async function fromFiles(paths) {
    let checked;
    try {
      checked = await invoke("addons_check", { paths });
    } catch (err) {
      toast(String(err), "error");
      return paths;
    }
    if (checked.problems.length) toast(checked.problems.join(" · "), "error");
    for (const found of checked.addons) await offer(found);
    return checked.rest;
  }

  /** Settings: pick a .dladdon, or a folder with an add-on being made. */
  async function pick(folderToo) {
    const path = folderToo
      ? await invoke("pick_folder", { title: t("addons.pickFolder"), start: null }).catch(() => null)
      : await invoke("pick_addon", { title: t("addons.pickFile") }).catch(() => null);
    if (!path) return null;
    const rest = await fromFiles([path]);
    if (rest.length) toast(t("addons.notAnAddon"), "error");
    return null;
  }

  async function setOn(id, on) {
    await invoke("addon_set_on", { id, on });
    return refresh();
  }

  async function remove(addon) {
    const yes = await confirmDialog({
      title: t("addons.remove.title", { name: addon.name }),
      text: addon.linked ? t("addons.remove.linked") : t("addons.remove.text"),
      choices: [{ label: t("addons.remove.button"), value: true, danger: true }],
    });
    if (!yes) return state;
    await invoke("addon_remove", { id: addon.id });
    return refresh();
  }

  // At the start: add-on files lying next to DropLaunch.exe or in Add-ons, not added yet.
  refresh().then((s) => {
    const found = s?.found ?? [];
    if (found.length) toast(tn("addons.foundAtStart", found.length, { names: found.map((f) => f.name).join(", ") }), "", () => ctx.settings.open("addons", { point: ".addon-found" }));
  });

  return {
    get state() {
      return state;
    },
    refresh,
    reload,
    show,
    tabChanged,
    fromFiles,
    offer,
    pick,
    setOn,
    remove,
    describe,
    permissionText,
    permissionName,
    /** Is this tab name an add-on's page that's there? */
    isTab: (name) => [...running.values()].some((r) => r.parts.some((p) => p.name === name)),
  };
}
