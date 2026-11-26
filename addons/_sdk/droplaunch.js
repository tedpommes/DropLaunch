/*!
 * DropLaunch add-on SDK, API version 1. MIT licence: copy it, change it, ship it with anything.
 *
 * An add-on's page includes it with <script src="../_sdk/droplaunch.js"></script> (DropLaunch
 * serves it next to the add-ons). It's the only way out of the add-on's frame: every call is a
 * message to DropLaunch's window, which checks it against the permissions in addon.json.
 *
 *   await droplaunch.ready;                       // DropLaunch said hello: info, theme, texts
 *   const reply = await droplaunch.rcon.ask("status");
 *   droplaunch.on("server", (server) => ...);     // the server's state (null: none running)
 *
 * See addons/README.md for every call, event and permission.
 */
(() => {
  "use strict";
  const API = 1;
  /** Calls waiting for DropLaunch's answer, by number. */
  const pending = new Map();
  /** Event name → handlers. */
  const handlers = new Map();
  let seq = 0;
  let resolveReady;
  const ready = new Promise((resolve) => (resolveReady = resolve));
  const info = { addon: null, app: null, language: "en", theme: null, visible: true, calls: [], events: [] };
  let texts = {};
  let english = {};

  const post = (message) => parent.postMessage({ droplaunch: API, ...message }, "*");

  /**
   * One call to DropLaunch: resolves with its answer, rejects with an Error saying what went wrong
   * (its `code`: denied, noServer, noPlugin, timeout, busy, notInUse, invalid, notFound, changed, noCall or
   * failed).
   */
  function call(name, args = {}) {
    const id = ++seq;
    return new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject });
      post({ type: "call", id, name, args });
    });
  }

  /** This DropLaunch has that call or event ("players.list", "chat"); after `ready`. */
  const has = (name) => info.calls.includes(name) || info.events.includes(name);

  // ---------------------------------------------------------------- storage
  // Sets are gathered for a moment and saved in one write (a slider that saves as it moves
  // writes once, not fifty times). get() sees what's waiting to be saved.
  const SAVE_AFTER = 200;
  /** key → { value, waiting: [{ resolve, reject }] } */
  let unsaved = new Map();
  let saveTimer = 0;

  function queueSet(key, value) {
    key = String(key ?? "");
    // (A bad key would take the others in its write down with it.)
    if (!key || new TextEncoder().encode(key).length > 200) return Promise.reject(Object.assign(new Error("A storage key is 1 to 200 bytes"), { code: "invalid" }));
    return new Promise((resolve, reject) => {
      const entry = unsaved.get(key) ?? { value: null, waiting: [] };
      entry.value = value === undefined ? null : value;
      entry.waiting.push({ resolve, reject });
      unsaved.set(key, entry);
      if (!saveTimer) saveTimer = setTimeout(save, SAVE_AFTER);
    });
  }

  function save() {
    clearTimeout(saveTimer);
    saveTimer = 0;
    const batch = unsaved;
    unsaved = new Map();
    if (!batch.size) return Promise.resolve();
    const entries = Object.fromEntries([...batch].map(([key, entry]) => [key, entry.value]));
    const all = [...batch.values()].flatMap((entry) => entry.waiting);
    return call("storage.setMany", { entries }).then(
      () => all.forEach((w) => w.resolve(null)),
      (err) => all.forEach((w) => w.reject(err)),
    );
  }
  // What's still waiting goes before the page does.
  addEventListener("pagehide", save);

  function storageGet(key, fallback) {
    const waiting = unsaved.get(String(key));
    if (waiting) return Promise.resolve(waiting.value ?? fallback);
    return call("storage.get", { key }).then((v) => v ?? fallback);
  }

  /** Listen to an event; returns a function that stops listening. */
  function on(name, handler) {
    if (!handlers.has(name)) handlers.set(name, new Set());
    handlers.get(name).add(handler);
    return () => handlers.get(name)?.delete(handler);
  }

  function emit(name, data) {
    for (const handler of handlers.get(name) ?? []) {
      try {
        handler(data);
      } catch (err) {
        console.error(err);
      }
    }
  }

  function applyTheme(theme) {
    if (!theme) return;
    const root = document.documentElement;
    for (const [name, value] of Object.entries(theme.vars ?? {})) root.style.setProperty(name, value);
    root.dataset.mode = theme.mode ?? "dark";
  }

  // ---------------------------------------------------------------- texts
  // lang/en.json in the add-on's folder ({"key": "text"}), and lang/<code>.json for DropLaunch's
  // language when there is one: what it leaves out stays English.
  async function loadTexts() {
    const read = (code) =>
      fetch(`lang/${code}.json`)
        .then((r) => (r.ok ? r.json() : {}))
        .catch(() => ({}));
    english = await read("en");
    texts = info.language && info.language !== "en" ? await read(info.language) : {};
  }

  /** A text by key, with {name} filled in from `vars`. A missing key comes back as itself. */
  function t(key, vars = {}) {
    const text = texts[key] ?? english[key] ?? key;
    return text.replace(/\{(\w+)\}/g, (all, name) => (name in vars ? String(vars[name]) : all));
  }

  /** Fills in data-t (text), data-t-title and data-t-placeholder under `root`. */
  function translate(root = document) {
    for (const node of root.querySelectorAll("[data-t]")) node.textContent = t(node.dataset.t);
    for (const node of root.querySelectorAll("[data-t-title]")) node.title = t(node.dataset.tTitle);
    for (const node of root.querySelectorAll("[data-t-placeholder]")) node.placeholder = t(node.dataset.tPlaceholder);
  }

  // ---------------------------------------------------------------- messages from DropLaunch
  addEventListener("message", (e) => {
    if (e.source !== parent) return;
    const m = e.data;
    if (!m || m.droplaunch !== API) return;
    if (m.type === "reply") {
      const waiting = pending.get(m.id);
      if (!waiting) return;
      pending.delete(m.id);
      if (m.ok) waiting.resolve(m.value);
      else waiting.reject(Object.assign(new Error(m.error ?? "Failed"), { code: m.code ?? "failed" }));
    } else if (m.type === "init") {
      Object.assign(info, m.info);
      applyTheme(info.theme);
      loadTexts().then(() => {
        translate();
        resolveReady(info);
      });
    } else if (m.type === "event") {
      if (m.name === "theme") {
        info.theme = m.data;
        applyTheme(m.data);
      }
      if (m.name === "visible") info.visible = m.data;
      emit(m.name, m.data);
    }
  });

  // ---------------------------------------------------------------- small UI helpers in the frame
  const make = (tag, props = {}, ...children) => {
    const node = document.createElement(tag);
    for (const [key, value] of Object.entries(props)) {
      if (value == null || value === false) continue;
      if (key === "class") node.className = value;
      else if (key === "text") node.textContent = value;
      else if (key.startsWith("on")) node.addEventListener(key.slice(2), value);
      else node.setAttribute(key, value === true ? "" : value);
    }
    for (const child of children.flat()) if (child != null && child !== false) node.append(child);
    return node;
  };

  let openMenu = null;
  function closeMenu() {
    openMenu?.remove();
    openMenu = null;
  }
  addEventListener("pointerdown", (e) => openMenu && !openMenu.contains(e.target) && closeMenu(), true);
  addEventListener("keydown", (e) => e.key === "Escape" && closeMenu());
  addEventListener("blur", closeMenu);
  addEventListener("resize", closeMenu);

  /**
   * A right-click menu at (x, y). Items: { label, run, danger, disabled, items: [...] } (items: a
   * submenu), or "sep" for a line.
   */
  function menu(x, y, items) {
    closeMenu();
    const build = (list) =>
      make(
        "div",
        { class: "dl-menu", role: "menu" },
        list.filter(Boolean).map((item) => {
          if (item === "sep") return make("div", { class: "dl-menu-sep", role: "separator" });
          const row = make("button", { type: "button", role: "menuitem", class: `dl-menu-item${item.danger ? " danger" : ""}${item.items ? " has-sub" : ""}`, disabled: item.disabled || (item.items && !item.items.length) }, make("span", { text: item.label }));
          if (item.items) {
            const sub = build(item.items);
            sub.classList.add("dl-submenu");
            row.append(sub);
          } else {
            row.addEventListener("click", () => {
              closeMenu();
              item.run?.();
            });
          }
          return row;
        }),
      );
    const node = build(items);
    document.body.append(node);
    const r = node.getBoundingClientRect();
    node.style.left = `${Math.max(4, Math.min(x, innerWidth - r.width - 4))}px`;
    node.style.top = `${Math.max(4, Math.min(y, innerHeight - r.height - 4))}px`;
    // Submenus open to the left when there's no room on the right.
    if (r.left + r.width * 2 > innerWidth) node.classList.add("flip");
    openMenu = node;
  }

  // Arrow keys move through the open menu (→ into a submenu, ← back out).
  addEventListener("keydown", (e) => {
    if (!openMenu || !["ArrowDown", "ArrowUp", "ArrowRight", "ArrowLeft"].includes(e.key)) return;
    e.preventDefault();
    const current = document.activeElement?.closest?.(".dl-menu-item");
    const level = current?.parentElement ?? openMenu;
    const items = [...level.children].filter((n) => n.matches(".dl-menu-item:not([disabled])"));
    if (e.key === "ArrowRight" && current?.classList.contains("has-sub")) {
      current.querySelector(".dl-submenu > .dl-menu-item:not([disabled])")?.focus();
    } else if (e.key === "ArrowLeft" && level !== openMenu) {
      level.closest(".dl-menu-item")?.focus();
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      const at = items.indexOf(current);
      const down = e.key === "ArrowDown";
      const next = at < 0 ? (down ? 0 : items.length - 1) : (at + (down ? 1 : -1) + items.length) % items.length;
      items[next]?.focus();
    }
  });

  /**
   * A dialog in the frame. fields: [{ name, label, type: "text" | "select" | "textarea", value,
   * placeholder, options: [{ value, label }] }]. Resolves with { name: value } or null (cancelled).
   */
  function form({ title, text = "", fields = [], ok = "OK", cancel = "Cancel", danger = false }) {
    closeMenu();
    return new Promise((resolve) => {
      const inputs = fields.map((f) => {
        const input =
          f.type === "select"
            ? make("select", { class: "dl-input", name: f.name }, (f.options ?? []).map((o) => make("option", { value: o.value, text: o.label, selected: String(o.value) === String(f.value ?? "") })))
            : make(f.type === "textarea" ? "textarea" : "input", { class: "dl-input", name: f.name, placeholder: f.placeholder ?? "", type: f.type === "textarea" ? null : "text" });
        if (f.type !== "select" && f.value != null) input.value = f.value;
        return { f, input };
      });
      const done = (value) => {
        removeEventListener("keydown", keys, true);
        back.remove();
        resolve(value);
      };
      const submit = () => done(Object.fromEntries(inputs.map(({ f, input }) => [f.name, input.value])));
      const keys = (e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          done(null);
        } else if (e.key === "Enter" && e.target.tagName !== "TEXTAREA") {
          e.preventDefault();
          submit();
        }
      };
      const back = make(
        "div",
        { class: "dl-modal-back" },
        make(
          "div",
          { class: "dl-modal", role: "dialog", "aria-modal": "true" },
          make("h3", { text: title }),
          text ? make("p", { class: "dl-muted", text }) : null,
          inputs.map(({ f, input }) => make("label", { class: "dl-field" }, f.label ? make("span", { text: f.label }) : null, input)),
          make(
            "div",
            { class: "dl-modal-actions" },
            make("button", { type: "button", class: "dl-btn ghost", text: cancel, onclick: () => done(null) }),
            make("button", { type: "button", class: `dl-btn ${danger ? "danger" : "primary"}`, text: ok, onclick: submit }),
          ),
        ),
      );
      addEventListener("keydown", keys, true);
      document.body.append(back);
      (inputs[0]?.input ?? back.querySelector(".dl-btn:last-child")).focus();
    });
  }

  // ---------------------------------------------------------------- console helpers
  /**
   * JSON.parse for what the server answers: numbers too long for JavaScript (SteamIDs, 17 digits)
   * come back as text instead of being rounded.
   */
  function parseJson(text) {
    const kept = String(text).replace(/("(?:[^"\\]|\\.)*")|(-?\d{16,})(?=\s*[,}\]])/g, (all, str, big) => str ?? `"${big}"`);
    return JSON.parse(kept);
  }

  /** One argument for a console command, in quotes (quotes inside become ', new lines spaces). */
  const quote = (value) => `"${String(value ?? "").replace(/"/g, "'").replace(/[\r\n]+/g, " ").trim()}"`;

  const confirm = (o) => form({ ...o, fields: [] }).then((v) => v !== null);
  const prompt = ({ value = "", placeholder, ...o }) => form({ ...o, fields: [{ name: "value", value, placeholder }] }).then((v) => (v ? v.value : null));

  window.droplaunch = Object.freeze({
    version: API,
    /** Resolves once DropLaunch said hello: { addon, app, language, theme }. */
    ready,
    info,
    call,
    on,
    has,
    t,
    translate,
    parseJson,
    quote,
    app: { info: () => call("app.info") },
    server: {
      status: () => call("server.status"),
      stop: (force = false) => call("server.stop", { force }),
      restart: () => call("server.restart"),
    },
    console: { lines: () => call("console.lines") },
    rcon: {
      /** Sent like typed in DropLaunch's console (it shows there, with its answer). */
      send: (text) => call("rcon.send", { text }),
      /** Sent quietly; resolves with the server's answer (timeout in ms, 1000 to 30000). */
      ask: (text, { timeout } = {}) => call("rcon.ask", { text, timeout }),
    },
    players: { list: () => call("players.list") },
    kits: {
      list: () => call("kits.list"),
      give: (kit, players) => call("kits.give", { kit, players }),
    },
    maps: { list: () => call("maps.list") },
    reports: {
      /** The bug reports of the map that runs (or ran last): { map, reports }. */
      list: () => call("reports.list"),
      setFixed: (id, fixed = true) => call("reports.setFixed", { id, fixed }),
      delete: (id) => call("reports.delete", { id }),
      /** You (the server's owner) to where it was reported, in the game. */
      teleport: (id) => call("reports.teleport", { id }),
    },
    configs: {
      /** The server folders and their config files: { folders: [{ id, name, framework, running, identity, identities, files }] }. */
      list: () => call("configs.list"),
      /** A file's text: { path, text, modified, size }. `folder`: a folder's id (left out: the first). */
      read: (path, folder) => call("configs.read", { path, folder }),
      /**
       * Writes a file read before: `modified` is what read gave (it isn't written over when it
       * changed since: code `changed`). `modified` null makes a server.cfg that isn't there.
       */
      write: (path, text, modified, folder) => call("configs.write", { path, text, modified, folder }),
    },
    storage: {
      get: storageGet,
      set: (key, value) => queueSet(key, value),
      remove: (key) => queueSet(key, null),
      keys: () => save().then(() => call("storage.keys")),
      /** Saves what's waiting now instead of in a moment. */
      flush: save,
    },
    ui: {
      toast: (text, kind = "") => call("ui.toast", { text, kind }),
      copy: (text, what = "") => call("ui.copy", { text, what }),
      openLink: (url) => call("ui.openLink", { url }),
      notify: (title, body = "") => call("ui.notify", { title, body }),
      /** A short text on the add-on's tab (a count); null takes it off. */
      badge: (text) => call("ui.badge", { text: text == null ? null : String(text) }),
      /** Shows the add-on's tab. */
      show: () => call("ui.show"),
      menu,
      closeMenu,
      form,
      confirm,
      prompt,
    },
  });

  // The page's script errors go to DropLaunch, which shows them for a folder being made (it has no
  // developer tools to see them in).
  const reportError = (what) => post({ type: "error", text: String(what ?? "").slice(0, 300) });
  addEventListener("error", (e) => reportError(e.message || e.error));
  addEventListener("unhandledrejection", (e) => reportError(e.reason?.message ?? e.reason));

  post({ type: "hello" });
})();
