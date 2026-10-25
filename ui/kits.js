// Kits: sets of items given to players on the running server, over RCON. A card under the server
// card (more options), "Give items", with just a kit to pick and Give to (a list of who's online,
// each with Give, and Give to all); full screen (its expand button) it has everything: the kits
// (each with Edit and Give), what the chosen one holds (what to wear, the belt, the inventory) and
// who gets it, laid out for the window's size. New kit / Edit open the editor there: Rust's items
// by category on the left (a search looks in the chosen category; a click or a drag onto a slot
// asks how many), the kit on the right (drag items between slots, click one to change its
// amount). Each kit is a file in DropLaunch's data folder (Kits/).
//
// Dragging is done with pointer events: the window takes the system's drag and drop for files
// (dropping a .map on DropLaunch), so the page's own HTML drag and drop never starts.
import { $, el, icon, invoke, toast, confirmDialog } from "./util.js";
import { Dropdown } from "./dropdown.js";
import { t, tn } from "./i18n.js";

/** Rust's containers and their slots. */
const SLOTS = { wear: 7, belt: 6, main: 24 };
const CONTAINERS = ["wear", "belt", "main"];
/** The categories in the picker, in this order (Rust's own names; All: every item). */
const ALL = "All";
const CATEGORIES = [ALL, "Weapon", "Ammunition", "Attire", "Medical", "Food", "Tool", "Construction", "Items", "Resources", "Component", "Traps", "Electrical", "Fun", "Misc"];
/** The categories' and containers' names (written out: the language check looks for each key). */
const CATEGORY_NAMES = {
  All: t("kits.cat.All"),
  Weapon: t("kits.cat.Weapon"),
  Ammunition: t("kits.cat.Ammunition"),
  Attire: t("kits.cat.Attire"),
  Medical: t("kits.cat.Medical"),
  Food: t("kits.cat.Food"),
  Tool: t("kits.cat.Tool"),
  Construction: t("kits.cat.Construction"),
  Items: t("kits.cat.Items"),
  Resources: t("kits.cat.Resources"),
  Component: t("kits.cat.Component"),
  Traps: t("kits.cat.Traps"),
  Electrical: t("kits.cat.Electrical"),
  Fun: t("kits.cat.Fun"),
  Misc: t("kits.cat.Misc"),
};
const CONTAINER_NAMES = { wear: t("kits.wear"), belt: t("kits.belt"), main: t("kits.main") };
/** Where a new item goes first (else the inventory, else the belt). */
const HOME = { Attire: "wear", Weapon: "belt", Tool: "belt", Medical: "belt" };
const MAX_AMOUNT = 100000;
/** The big stack offered for anything that stacks (given as one stack, above Rust's stack size). */
const BIG_STACK = 99999;
/** Pictures loading at once. */
const ICON_JOBS = 6;
/** The player list follows the server this often while the card shows. */
const PLAYERS_EVERY = 5000;
/** The picker's picture size (the slider), kept between starts. */
const TILE_KEY = "droplaunch.kitTile";
const TILE = { min: 72, max: 176, step: 8, start: 104 };
/** A press moves this far before it's a drag. */
const DRAG_START = 5;

export function initKits(ctx) {
  const card = $("kits-card");
  const main = $("kits-main");
  let kits = [];
  let selected = null;
  /** Rust's items by short name: { s, n, c, st }. */
  let catalog = null;
  let players = [];
  /** Who's being given a kit right now (player ids, or "*" for everyone). */
  const giving = new Set();
  /** Who was just given one: { kitId:playerId } for a moment. */
  const given = new Set();
  let playersError = "";
  /** The kit being edited: { kit, isNew, saved (as JSON, to see changes), untitled }. */
  let editing = null;
  let category = CATEGORIES[1];
  let query = "";
  /** The amount popup that's open: { node, close }. */
  let pop = null;
  /** The Give to list that's open: { node, kitId, anchor, close }. */
  let menu = null;
  let renaming = false;
  /** A drag just ended: the click that may follow isn't one. */
  let dragged = false;
  /** What's drawn now (nothing is drawn again while it stays the same). */
  let drawnKey = "";
  /** The kits that come with DropLaunch (a "Built in" tag), by id. */
  let builtIns = new Set();
  const builtInTag = (kit) => (builtIns.has(kit.id) ? el("span", { class: "builtin-tag", text: t("common.builtIn") }) : null);

  const live = () => Boolean(ctx.server?.isLive()) && ctx.server?.view()?.phase === "ready";
  const full = () => card.classList.contains("maximized");
  const itemOf = (s) => catalog?.get(s) ?? { s, n: s, c: "Misc", st: 1 };
  const count = (kit) => CONTAINERS.reduce((n, c) => n + kit[c].length, 0);
  const current = () => kits.find((k) => k.id === selected) ?? kits[0] ?? null;

  // ---------------------------------------------------------------- pictures
  const icons = new Map();
  const waiting = [];
  let running = 0;

  /** An item's picture as a URL (downloaded once by DropLaunch, then kept), or null. */
  function iconUrl(item) {
    if (!icons.has(item)) {
      icons.set(
        item,
        new Promise((resolve) => {
          waiting.push({ item, resolve });
          pump();
        }),
      );
    }
    return icons.get(item);
  }

  function pump() {
    while (running < ICON_JOBS && waiting.length) {
      const { item, resolve } = waiting.shift();
      running++;
      invoke("item_icon", { item })
        .then((bytes) => resolve(URL.createObjectURL(new Blob([bytes], { type: "image/png" }))))
        .catch(() => resolve(null))
        .finally(() => {
          running--;
          pump();
        });
    }
  }

  /** The picture of an item, or its initials until (or if) there's none. */
  function itemPicture(item, lazy = false) {
    const def = itemOf(item);
    const node = el("span", { class: "kit-pic", "data-cat": def.c }, el("span", { class: "kit-pic-alt", text: initials(def.n) }));
    const load = () =>
      iconUrl(item).then((url) => {
        if (!url) return;
        node.replaceChildren(el("img", { src: url, alt: "", draggable: "false" }));
        node.classList.add("has-img");
      });
    if (lazy) observer.observe(node), (node._load = load);
    else load();
    return node;
  }

  // Pictures in the picker load once they scroll into view.
  const observer = new IntersectionObserver(
    (entries) => {
      for (const e of entries) {
        if (!e.isIntersecting) continue;
        observer.unobserve(e.target);
        e.target._load?.();
      }
    },
    { rootMargin: "200px" },
  );

  const initials = (name) =>
    name
      .split(/[\s.-]+/)
      .filter(Boolean)
      .slice(0, 2)
      .map((w) => w[0].toUpperCase())
      .join("");

  // ---------------------------------------------------------------- data
  async function loadCatalog() {
    if (catalog) return;
    let list = await invoke("kit_items").catch(() => null);
    if (!list) {
      try {
        list = await (await fetch("kits/items.json")).json();
      } catch {
        list = [];
      }
    }
    catalog = new Map(list.map((i) => [i.s, i]));
  }

  async function saveKits(next) {
    try {
      kits = await invoke("kits_save", { kits: next });
      return true;
    } catch (err) {
      toast(String(err), "error");
      return false;
    }
  }

  async function loadPlayers() {
    if (!live()) {
      players = [];
      playersError = "";
      return;
    }
    try {
      players = await invoke("kit_players");
      playersError = "";
    } catch (err) {
      playersError = String(err);
    }
  }

  // ---------------------------------------------------------------- the card (not full screen)
  /** Built once (the dropdown stays); render() fills it in. */
  const compact = (() => {
    const select = el("select", { "data-size": "sm", "aria-label": t("kits.title") });
    const root = el("div", { class: "kits-compact" });
    main.append(root);
    root.append(select);
    const dropdown = new Dropdown(select, { meta: (id) => (builtIns.has(id) ? t("common.builtIn") : "") });
    select.addEventListener("change", (e) => {
      if (!e.isTrusted && !e.bubbles) return;
      selected = select.value;
      render();
    });
    const give = el("button", { class: "btn btn-small btn-primary kit-give-to", "aria-haspopup": "dialog", onclick: (e) => current() && toggleMenu(current(), e.currentTarget) }, icon("gift"), el("span", { text: t("kits.give.title") }), icon("chevron"));
    const note = el("p", { class: "kit-note small" });
    const make = el("button", { class: "btn btn-small btn-ghost", onclick: () => openFull(() => edit(null)) }, icon("plus"), el("span", { text: t("kits.make") }));
    const row = el("div", { class: "kits-compact-row" }, dropdown.root, give);
    root.replaceChildren(select, row, note, make);
    return { root, select, dropdown, give, note, make, row };
  })();

  function renderCompact() {
    const kit = current();
    const players0 = live() ? players.length : 0;
    const key = JSON.stringify(["compact", kits.map((k) => [k.id, k.name]), kit?.id, live(), players0]);
    if (key === drawnKey) return;
    drawnKey = key;
    for (const node of [...main.children]) if (node !== compact.root) node.remove();
    compact.root.hidden = false;
    compact.row.hidden = !kit;
    compact.make.hidden = Boolean(kit);
    if (kit) {
      compact.select.replaceChildren(...kits.map((k) => el("option", { value: k.id, text: k.name })));
      compact.select.value = kit.id;
      compact.select.dispatchEvent(new Event("change"));
      compact.give.disabled = !live() || !count(kit);
      compact.give.dataset.giveFor = kit.id;
    }
    compact.note.textContent = !kit ? t("kits.empty") : !live() ? t("kits.give.noServer") : players0 ? "" : t("kits.give.none");
    compact.note.hidden = !compact.note.textContent;
  }

  /** Full screen, then (optionally) something to do there. */
  function openFull(then) {
    if (!full()) ctx.layout?.setMaximized("kits-card");
    then?.();
  }

  // ---------------------------------------------------------------- full screen: kits, contents, who
  /** A kit's containers as slots (the inventory up to its last used row). */
  function preview(kit) {
    return el(
      "div",
      { class: "kit-preview" },
      CONTAINERS.map((c) =>
        el(
          "div",
          { class: `kit-container c-${c}` },
          el("div", { class: "kit-container-name", text: CONTAINER_NAMES[c] }),
          el(
            "div",
            { class: "kit-slots" },
            Array.from({ length: c === "main" ? Math.max(1, Math.ceil((Math.max(-1, ...kit.main.map((i) => i.slot)) + 1) / 6)) * 6 : SLOTS[c] }, (_, slot) => {
              const it = kit[c].find((i) => i.slot === slot);
              return el("div", { class: `kit-slot${it ? " filled" : ""}`, title: it ? `${itemOf(it.item).n} ×${it.amount}` : null }, it ? [itemPicture(it.item), el("span", { class: "kit-amount", text: `×${it.amount}` })] : null);
            }),
          ),
        ),
      ),
    );
  }

  function kitRow(kit) {
    const items = CONTAINERS.flatMap((c) => kit[c]).slice(0, 5);
    const pick = () => ((selected = kit.id), render(true));
    const chosen = kit.id === current()?.id;
    // A div that acts as a button: its name turns into a text box (which a button can't hold).
    return el(
      "div",
      {
        class: "kit-card",
        role: "button",
        tabindex: "0",
        "aria-pressed": String(chosen),
        onclick: pick,
        onkeydown: (e) => (e.key === "Enter" || e.key === " ") && e.target === e.currentTarget && (e.preventDefault(), pick()),
      },
      el(
        "span",
        { class: "kit-card-text" },
        // Only the name itself, of the chosen kit, turns into a text box (a click elsewhere picks it).
        el("span", { class: "kit-card-name", text: kit.name, title: chosen ? t("kits.rename.title") : null, onclick: (e) => chosen && rename(e, kit) }),
        el("span", { class: "kit-card-count" }, el("span", { text: tn("kits.count", count(kit)) }), builtInTag(kit)),
      ),
      el(
        "span",
        { class: "kit-card-btns" },
        el("button", { type: "button", class: "icon-btn", title: t("kits.edit"), onclick: (e) => (e.stopPropagation(), (selected = kit.id), edit(kit)) }, icon("pencil")),
        el(
          "button",
          { type: "button", class: "icon-btn kit-card-give", "data-give-for": kit.id, title: live() ? t("kits.give.title") : t("kits.give.noServer"), disabled: !live() || !count(kit) ? true : null, "aria-haspopup": "dialog", onclick: (e) => (e.stopPropagation(), toggleMenu(kit, e.currentTarget)) },
          icon("gift"),
        ),
      ),
      el("span", { class: "kit-card-pics" }, items.map((i) => itemPicture(i.item))),
    );
  }

  /** The name becomes a text box: Enter or clicking elsewhere keeps the new name, Esc doesn't. */
  function rename(e, kit) {
    e.stopPropagation();
    const name = e.currentTarget;
    const input = el("input", { type: "text", class: "kit-rename", maxlength: "60", value: kit.name, "aria-label": t("kits.rename.title") });
    let done = false;
    renaming = true;
    const finish = async (keep) => {
      if (done) return;
      done = true;
      const value = input.value.trim();
      if (keep && value && value !== kit.name && (await saveKits(kits.map((k) => (k.id === kit.id ? { ...k, name: value } : k))))) selected = kit.id;
      renaming = false;
      render(true);
    };
    input.addEventListener("click", (ev) => ev.stopPropagation());
    input.addEventListener("keydown", (ev) => {
      ev.stopPropagation();
      if (ev.key === "Enter") finish(true);
      else if (ev.key === "Escape") finish(false);
    });
    input.addEventListener("blur", () => finish(true));
    name.replaceWith(input);
    input.select();
    input.focus();
  }

  /** Who's online, each with Give, then Give to all (the full screen column and the Give to list). */
  function playerList(kit) {
    if (!live()) return [el("p", { class: "kit-note", text: t("kits.give.noServer") })];
    const busy = giving.size > 0;
    const vanilla = ctx.state?.framework === "none" || !ctx.state?.playtest;
    return [
      players.length
        ? el(
            "div",
            { class: "kit-players" },
            players.map((p) => {
              const done = given.has(`${kit.id}:${p.id}`);
              return el(
                "div",
                { class: "kit-player" },
                el("span", { class: "kit-player-dot", "aria-hidden": "true" }),
                el("span", { class: "kit-player-name", text: p.name || p.id, title: p.name || p.id }),
                el(
                  "button",
                  { type: "button", class: `btn btn-small ${done ? "btn-ghost done" : "btn-primary"}`, disabled: busy || !count(kit) ? true : null, onclick: () => giveTo(kit, [p.id], p.id) },
                  giving.has(p.id) ? el("span", { class: "spinner" }) : icon(done ? "check" : "gift"),
                  el("span", { text: done ? t("kits.give.done") : t("kits.give.short") }),
                ),
              );
            }),
          )
        : el("p", { class: "kit-note", text: playersError || t("kits.give.none") }),
      el(
        "button",
        { type: "button", class: "btn btn-primary btn-block kit-give-all", disabled: !players.length || busy || !count(kit) ? true : null, onclick: () => giveTo(kit, players.map((p) => p.id), "*") },
        giving.has("*") ? el("span", { class: "spinner" }) : icon("users"),
        el("span", { text: t("kits.give.all", { n: players.length }) }),
      ),
      vanilla ? el("p", { class: "kit-note small", text: t("kits.note.vanilla") }) : null,
    ].filter(Boolean);
  }

  function givePanel(kit) {
    return el(
      "div",
      { class: "kit-give" },
      el(
        "div",
        { class: "kits-row" },
        el("h3", { text: t("kits.give.title") }),
        live() ? el("button", { class: "icon-btn", title: t("kits.give.refresh"), onclick: async () => (await loadPlayers(), render(true)) }, icon("refresh")) : null,
      ),
      ...playerList(kit),
    );
  }

  // ---------------------------------------------------------------- the Give to list (a small window)
  function toggleMenu(kit, anchor) {
    if (menu && menu.kitId === kit.id && menu.anchor === anchor) return closeMenu();
    closeMenu();
    const node = el("div", { class: "kit-menu", role: "dialog", "aria-label": t("kits.give.title") });
    document.body.append(node);
    const outside = (e) => menu && !node.contains(e.target) && !menu.anchor.contains(e.target) && closeMenu();
    const keys = (e) => e.key === "Escape" && (e.stopPropagation(), closeMenu());
    setTimeout(() => document.addEventListener("pointerdown", outside, true));
    document.addEventListener("keydown", keys, true);
    window.addEventListener("resize", closeMenu);
    menu = {
      node,
      kitId: kit.id,
      anchor,
      close: () => {
        document.removeEventListener("pointerdown", outside, true);
        document.removeEventListener("keydown", keys, true);
        window.removeEventListener("resize", closeMenu);
      },
    };
    drawMenu();
    loadPlayers().then(drawMenu);
  }

  /** The button the list belongs to (the card draws itself again now and then: the new one). */
  function menuAnchor() {
    if (menu.anchor.isConnected && menu.anchor.offsetParent) return menu.anchor;
    return [...document.querySelectorAll("[data-give-for]")].find((n) => n.dataset.giveFor === menu.kitId && n.offsetParent) ?? null;
  }

  function drawMenu() {
    if (!menu) return;
    const kit = kits.find((k) => k.id === menu.kitId);
    const anchor = kit && live() ? menuAnchor() : null;
    if (!anchor) return closeMenu();
    menu.anchor = anchor;
    menu.node.replaceChildren(
      el(
        "div",
        { class: "kit-menu-head" },
        el("div", { class: "kit-menu-title" }, el("strong", { text: t("kits.give.title") }), el("span", { text: kit.name })),
        live() ? el("button", { type: "button", class: "icon-btn", title: t("kits.give.refresh"), onclick: async () => (await loadPlayers(), drawMenu()) }, icon("refresh")) : null,
      ),
      ...playerList(kit),
    );
    // Under the button (above it when there's no room), inside the window.
    const r = menu.anchor.getBoundingClientRect();
    const w = menu.node.offsetWidth;
    const h = menu.node.offsetHeight;
    let x = Math.min(r.right - w, window.innerWidth - w - 10);
    let y = r.bottom + 6;
    if (y + h > window.innerHeight - 10) y = Math.max(10, r.top - h - 6);
    menu.node.style.left = `${Math.max(10, x)}px`;
    menu.node.style.top = `${y}px`;
  }

  function closeMenu() {
    if (!menu) return;
    menu.close();
    menu.node.remove();
    menu = null;
  }

  function renderFull(force) {
    const kit = current();
    const key = JSON.stringify(["full", kits, kit?.id, live(), players, [...giving], [...given], playersError]);
    if (!force && key === drawnKey) return;
    drawnKey = key;
    selected = kit?.id ?? null;
    compact.root.hidden = true;
    for (const node of [...main.children]) if (node !== compact.root) node.remove();
    main.append(
      el(
        "div",
        { class: "kits-full" },
        el(
          "section",
          { class: "kits-col kits-list" },
          el("div", { class: "kits-row" }, el("h3", { text: t("kits.list") }), el("button", { class: "btn btn-small btn-primary", onclick: () => edit(null) }, icon("plus"), el("span", { text: t("kits.new") }))),
          kits.length ? el("div", { class: "kit-cards" }, kits.map(kitRow)) : el("p", { class: "kit-note", text: t("kits.empty") }),
        ),
        el(
          "section",
          { class: "kits-col kit-detail" },
          kit
            ? [
                el(
                  "div",
                  { class: "kits-row" },
                  el("h3", { class: "kit-detail-name", text: kit.name }),
                  builtInTag(kit),
                  el("button", { class: "btn btn-small btn-ghost", onclick: () => edit(kit) }, icon("sliders"), el("span", { text: t("kits.edit") })),
                  el("button", { class: "icon-btn", title: t("kits.duplicate"), onclick: () => duplicate(kit) }, icon("copy")),
                  el("button", { class: "icon-btn danger", title: t("kits.delete"), onclick: () => remove(kit) }, icon("trash")),
                ),
                preview(kit),
              ]
            : el("div", { class: "kits-empty" }, icon("gift"), el("p", { text: t("kits.empty") }), el("button", { class: "btn btn-primary", onclick: () => edit(null) }, icon("plus"), el("span", { text: t("kits.new") }))),
        ),
        kit ? el("section", { class: "kits-col kits-who" }, givePanel(kit)) : null,
      ),
    );
  }

  /** Give `kit` to these players (`who`: the one asked for, or "*" for Give to all). */
  async function giveTo(kit, ids, who) {
    if (!ids.length || giving.size) return;
    giving.add(who);
    redraw();
    try {
      const r = await invoke("kit_give", { kit, players: ids });
      toast([tn("kits.given", r.players, { name: kit.name }), r.exact ? "" : t("kits.given.loose")].filter(Boolean).join(" "), "ok");
      const marks = ids.map((id) => `${kit.id}:${id}`);
      for (const m of marks) given.add(m);
      setTimeout(() => (marks.forEach((m) => given.delete(m)), redraw()), 4000);
      if (who === "*") closeMenu();
    } catch (err) {
      toast(String(err), "error");
    } finally {
      giving.delete(who);
      redraw();
    }
  }

  /** The card and the Give to list, after a give. */
  const redraw = () => render(true);

  async function duplicate(kit) {
    const copy = { ...structuredClone(kit), id: crypto.randomUUID(), name: t("kits.copyName", { name: kit.name }) };
    if (await saveKits([...kits, copy])) {
      selected = copy.id;
      render(true);
    }
  }

  async function remove(kit) {
    const ok = await confirmDialog({ title: t("kits.delete.title", { name: kit.name }), text: t("kits.delete.text"), ok: t("kits.delete") });
    if (!ok) return;
    if (await saveKits(kits.filter((k) => k.id !== kit.id))) {
      selected = null;
      render(true);
    }
  }

  // ---------------------------------------------------------------- the editor
  async function edit(kit) {
    closeMenu();
    if (!full()) ctx.layout?.setMaximized("kits-card");
    await loadCatalog();
    const n = kits.length + 1;
    const copy = kit ? structuredClone(kit) : { id: crypto.randomUUID(), name: "", wear: [], belt: [], main: [] };
    editing = { kit: copy, isNew: !kit, saved: JSON.stringify(copy), untitled: t("kits.untitled", { n }) };
    renderEditor();
    main.querySelector(".kit-name").focus();
  }

  function renderEditor() {
    drawnKey = "";
    closePop();
    compact.root.hidden = true;
    for (const node of [...main.children]) if (node !== compact.root) node.remove();
    main.append(el("div", { class: "kit-editor" }, pickerColumn(), kitColumn()));
    renderTabs();
    renderGrid();
    renderBuild();
  }

  const tileSize = () => {
    try {
      const v = Number(localStorage.getItem(TILE_KEY));
      return v >= TILE.min && v <= TILE.max ? v : TILE.start;
    } catch {
      return TILE.start;
    }
  };

  function pickerColumn() {
    const search = el("input", { type: "search", class: "kit-search", placeholder: t("kits.search"), value: query });
    // The search looks in the chosen category (All: everywhere); the category stays.
    search.addEventListener("input", () => {
      query = search.value.trim().toLowerCase();
      renderGrid();
      renderTabs();
    });
    const size = el("input", { type: "range", min: String(TILE.min), max: String(TILE.max), step: String(TILE.step), value: String(tileSize()), "aria-label": t("kits.tileSize") });
    const grid = el("div", { class: "kit-grid", style: `--tile: ${tileSize()}px` });
    size.addEventListener("input", () => {
      grid.style.setProperty("--tile", `${size.value}px`);
      try {
        localStorage.setItem(TILE_KEY, size.value);
      } catch {
        /* not kept */
      }
    });
    return el(
      "section",
      { class: "kit-picker" },
      el(
        "div",
        { class: "kit-picker-head" },
        el("div", { class: "kit-search-box" }, icon("search"), search),
        el("label", { class: "kit-size", title: t("kits.tileSize") }, el("span", { class: "kit-size-small" }, icon("map")), size, el("span", { class: "kit-size-big" }, icon("map"))),
      ),
      el("div", { class: "kit-tabs", role: "tablist" }),
      grid,
    );
  }

  const found = (i) => !query || i.n.toLowerCase().includes(query) || i.s.includes(query);
  const inCategory = (i, c) => c === ALL || i.c === c;

  /** The chosen category's items the search finds. */
  function shown(c = category) {
    return [...(catalog?.values() ?? [])].filter((i) => inCategory(i, c) && found(i)).sort((a, b) => a.n.localeCompare(b.n));
  }

  function renderTabs() {
    const tabs = main.querySelector(".kit-tabs");
    if (!tabs) return;
    const items = [...(catalog?.values() ?? [])];
    const present = new Set(items.map((i) => i.c));
    tabs.replaceChildren(
      ...CATEGORIES.filter((c) => c === ALL || present.has(c)).map((c) => {
        // While searching each category says how many it finds.
        const hits = query ? items.filter((i) => inCategory(i, c) && found(i)).length : null;
        return el(
          "button",
          { type: "button", role: "tab", class: `kit-tab${hits === 0 ? " none" : ""}`, "data-cat": c, "aria-selected": String(c === category), onclick: () => ((category = c), renderTabs(), renderGrid()) },
          el("span", { text: CATEGORY_NAMES[c] }),
          hits === null ? null : el("span", { class: "kit-tab-count", text: String(hits) }),
        );
      }),
    );
  }

  function renderGrid() {
    const grid = main.querySelector(".kit-grid");
    if (!grid) return;
    const items = shown();
    grid.replaceChildren(
      ...(items.length
        ? items.map((def) => {
            const tile = el(
              "button",
              { type: "button", class: "item-tile", "data-cat": def.c, title: def.n, onclick: (e) => !dragged && askAmount(e.currentTarget, def) },
              itemPicture(def.s, true),
              el("span", { class: "item-tile-name", text: def.n }),
            );
            tile.addEventListener("pointerdown", (e) => dragFrom(e, { item: def.s }, tile));
            return tile;
          })
        : [noneFound()]),
    );
    grid.scrollTop = 0;
  }

  /** Nothing to show: none match here (maybe elsewhere: All), or no item list. */
  function noneFound() {
    if (!catalog?.size) return el("p", { class: "kit-note", text: t("kits.noItems") });
    const elsewhere = category !== ALL ? shown(ALL).length : 0;
    return el(
      "div",
      { class: "kit-none" },
      el("p", { class: "kit-note", text: elsewhere ? t("kits.noMatchIn", { category: CATEGORY_NAMES[category] ?? category }) : t("kits.noMatch") }),
      elsewhere ? el("button", { type: "button", class: "btn btn-small btn-ghost", onclick: () => ((category = ALL), renderTabs(), renderGrid()) }, icon("search"), el("span", { text: t("kits.showAll", { n: elsewhere }) })) : null,
    );
  }

  function kitColumn() {
    const kit = editing.kit;
    const name = el("input", { type: "text", class: "kit-name", maxlength: "60", placeholder: editing.untitled, value: kit.name });
    name.addEventListener("input", () => (kit.name = name.value));
    name.addEventListener("keydown", (e) => e.key === "Enter" && save());
    return el(
      "section",
      { class: "kit-build" },
      el("div", { class: "kit-build-head" }, el("h3", { text: editing.isNew ? t("kits.editor.new") : t("kits.editor.edit") }), name),
      el("div", { class: "kit-build-body" }),
      el("p", { class: "kit-note small", text: t("kits.editor.hint") }),
      el(
        "div",
        { class: "kit-build-foot" },
        el("button", { class: "btn btn-ghost", onclick: cancelEdit }, el("span", { text: t("common.cancel") })),
        el("button", { class: "btn btn-primary", onclick: save }, icon("check"), el("span", { text: t("kits.save") })),
      ),
    );
  }

  /** The kit's slots: items can be dragged between them, and onto them from the picker. */
  function renderBuild() {
    const body = main.querySelector(".kit-build-body");
    if (!body) return;
    const kit = editing.kit;
    body.replaceChildren(
      ...CONTAINERS.map((c) =>
        el(
          "div",
          { class: `kit-container c-${c}` },
          el("div", { class: "kit-container-name" }, el("span", { text: CONTAINER_NAMES[c] }), el("span", { class: "kit-container-count", text: `${kit[c].length}/${SLOTS[c]}` })),
          el(
            "div",
            { class: "kit-slots" },
            Array.from({ length: SLOTS[c] }, (_, slot) => buildSlot(c, slot)),
          ),
        ),
      ),
    );
  }

  function buildSlot(c, slot) {
    const it = editing.kit[c].find((i) => i.slot === slot);
    const node = el("div", { class: `kit-slot${it ? " filled" : ""}`, "data-c": c, "data-slot": String(slot), title: it ? `${itemOf(it.item).n} ×${it.amount}` : null });
    if (it) {
      node.append(itemPicture(it.item), el("span", { class: "kit-amount", text: `×${it.amount}` }));
      node.addEventListener("click", () => !dragged && askAmount(node, itemOf(it.item), { c, slot }));
      node.addEventListener("pointerdown", (e) => dragFrom(e, { c, slot, item: it.item, moving: true }, node));
    }
    return node;
  }

  /**
   * A drag (an item from the picker, or one already in the kit): a picture follows the pointer,
   * the slot under it lights up, and letting go there drops it.
   */
  function dragFrom(e, payload, source) {
    if (e.button !== 0) return;
    const sx = e.clientX;
    const sy = e.clientY;
    let ghost = null;
    let over = null;
    const slotAt = (ev) => document.elementFromPoint(ev.clientX, ev.clientY)?.closest(".kit-build .kit-slot") ?? null;
    const move = (ev) => {
      if (!ghost) {
        if (Math.hypot(ev.clientX - sx, ev.clientY - sy) < DRAG_START) return;
        closePop();
        ghost = el("div", { class: "kit-ghost" }, itemPicture(payload.item));
        document.body.append(ghost);
        document.body.classList.add("kit-dragging");
        source.classList.add("dragging");
      }
      ghost.style.left = `${ev.clientX}px`;
      ghost.style.top = `${ev.clientY}px`;
      const target = slotAt(ev);
      if (target !== over) {
        over?.classList.remove("drop");
        over = target;
        over?.classList.add("drop");
      }
    };
    const end = (ev) => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
      window.removeEventListener("pointercancel", end);
      if (!ghost) return;
      ghost.remove();
      document.body.classList.remove("kit-dragging");
      source.classList.remove("dragging");
      over?.classList.remove("drop");
      dragged = true;
      setTimeout(() => (dragged = false));
      if (!over || ev.type !== "pointerup") return;
      const at = { c: over.dataset.c, slot: Number(over.dataset.slot) };
      if (payload.moving) move2(payload, at);
      else dropNew(itemOf(payload.item), at, over);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end);
    window.addEventListener("pointercancel", end);
  }

  /** Move an item to another slot (swapping with what's there). Only clothes can be worn. */
  function move2(from, to) {
    if (from.c === to.c && from.slot === to.slot) return;
    const kit = editing.kit;
    const a = kit[from.c].find((i) => i.slot === from.slot);
    const b = kit[to.c].find((i) => i.slot === to.slot);
    if (!a) return;
    const wearable = (it) => !it || itemOf(it.item).c === "Attire";
    if ((to.c === "wear" && !wearable(a)) || (from.c === "wear" && b && !wearable(b))) {
      toast(t("kits.wearOnly"), "warn");
      return;
    }
    kit[from.c] = kit[from.c].filter((i) => i !== a);
    if (b) kit[to.c] = kit[to.c].filter((i) => i !== b);
    kit[to.c].push({ ...a, slot: to.slot });
    if (b) kit[from.c].push({ ...b, slot: from.slot });
    renderBuild();
    flash(to);
  }

  /** An item dragged from the picker onto a slot: how many, then it goes there. */
  function dropNew(def, at, node) {
    if (at.c === "wear" && def.c !== "Attire") {
      toast(t("kits.wearOnly"), "warn");
      return;
    }
    askAmount(node, def, null, at);
  }

  /** A free slot for a new item: its usual place first. */
  function freeSlot(def) {
    const kit = editing.kit;
    const order = [HOME[def.c] ?? "main", "main", "belt"].filter((c, i, a) => a.indexOf(c) === i && (c !== "wear" || def.c === "Attire"));
    for (const c of order) {
      for (let slot = 0; slot < SLOTS[c]; slot++) if (!kit[c].some((i) => i.slot === slot)) return { c, slot };
    }
    return null;
  }

  /**
   * How many: a little window at the item (in the picker: adds it, to `target` when it was
   * dropped on a slot; in the kit: changes the amount, or removes it).
   */
  function askAmount(anchor, def, at = null, target = null) {
    closePop();
    const existing = at ? editing.kit[at.c].find((i) => i.slot === at.slot) : null;
    const input = el("input", { type: "number", min: "1", max: String(MAX_AMOUNT), value: String(existing?.amount ?? (def.st > 1 ? Math.min(def.st, 100) : 1)) });
    const quick = [...new Set([1, 10, 100, def.st])].filter((n) => n <= Math.max(def.st, 1) || n === 1).sort((a, b) => a - b);
    if (def.st > 1 && def.st < BIG_STACK) quick.push(BIG_STACK);
    const done = () => {
      const amount = Math.max(1, Math.min(MAX_AMOUNT, Math.round(Number(input.value) || 1)));
      const kit = editing.kit;
      if (existing) existing.amount = amount;
      else if (target) {
        // Dropped on a slot: what's there moves to a free one (or goes, when there's none).
        const there = kit[target.c].find((i) => i.slot === target.slot);
        if (there) {
          kit[target.c] = kit[target.c].filter((i) => i !== there);
          const spot = freeSlot(itemOf(there.item));
          if (spot) kit[spot.c].push({ ...there, slot: spot.slot });
          else toast(t("kits.replaced", { name: itemOf(there.item).n }), "warn");
        }
        kit[target.c].push({ item: def.s, amount, slot: target.slot });
        flash(target);
      } else {
        const slot = freeSlot(def);
        if (!slot) return toast(t("kits.full"), "warn");
        kit[slot.c].push({ item: def.s, amount, slot: slot.slot });
        flash(slot);
      }
      closePop();
      renderBuild();
    };
    input.addEventListener("keydown", (e) => e.key === "Enter" && done());
    const node = el(
      "div",
      { class: "amount-pop", role: "dialog", "aria-label": def.n },
      el("div", { class: "amount-head" }, itemPicture(def.s), el("div", {}, el("strong", { text: def.n }), el("span", { text: CATEGORY_NAMES[def.c] ?? def.c }))),
      el("label", { class: "amount-label" }, el("span", { text: t("kits.amount") }), input),
      el(
        "div",
        { class: "amount-quick" },
        quick.map((n) => {
          // A stack goes in right away; the other amounts fill in the box.
          const stack = (n === def.st && n > 1) || n === BIG_STACK;
          return el(
            "button",
            { type: "button", class: `chip-btn${stack ? " stack" : ""}`, onclick: () => ((input.value = String(n)), stack ? done() : input.focus()) },
            el("span", { text: stack ? t("kits.stack", { n }) : `×${n}` }),
          );
        }),
      ),
      el(
        "div",
        { class: "amount-actions" },
        existing
          ? el("button", { type: "button", class: "btn btn-small btn-ghost danger", onclick: () => ((editing.kit[at.c] = editing.kit[at.c].filter((i) => i !== existing)), closePop(), renderBuild()) }, icon("trash"), el("span", { text: t("kits.remove") }))
          : null,
        el("button", { type: "button", class: "btn btn-small btn-primary", onclick: done }, el("span", { text: existing ? t("kits.apply") : t("kits.add") })),
      ),
    );
    main.querySelector(".kit-editor").append(node);
    place(node, anchor);
    const outside = (e) => !node.contains(e.target) && !anchor.contains(e.target) && closePop();
    setTimeout(() => document.addEventListener("pointerdown", outside, true));
    pop = { node, close: () => document.removeEventListener("pointerdown", outside, true) };
    input.select();
    input.focus();
  }

  /** Next to the item, inside the editor. */
  function place(node, anchor) {
    const host = main.querySelector(".kit-editor").getBoundingClientRect();
    const r = anchor.getBoundingClientRect();
    const w = node.offsetWidth;
    const h = node.offsetHeight;
    let x = r.left - host.left + r.width / 2 - w / 2;
    let y = r.bottom - host.top + 8;
    if (y + h > host.height - 8) y = r.top - host.top - h - 8;
    x = Math.max(8, Math.min(x, host.width - w - 8));
    y = Math.max(8, Math.min(y, host.height - h - 8));
    node.style.left = `${x}px`;
    node.style.top = `${y}px`;
  }

  function closePop() {
    if (!pop) return;
    pop.close();
    pop.node.remove();
    pop = null;
  }

  /** The slot an item just went into lights up. */
  function flash(at) {
    requestAnimationFrame(() => main.querySelector(`.kit-build .kit-slot[data-c="${at.c}"][data-slot="${at.slot}"]`)?.classList.add("added"));
  }

  const changed = () => editing && JSON.stringify(editing.kit) !== editing.saved;

  async function cancelEdit() {
    if (changed() && !(await confirmDialog({ title: t("kits.discard.title"), text: t("kits.discard.text"), ok: t("kits.discard") }))) return;
    editing = null;
    render(true);
  }

  async function save() {
    const kit = editing.kit;
    kit.name = kit.name.trim() || editing.untitled;
    const next = editing.isNew ? [...kits, kit] : kits.map((k) => (k.id === kit.id ? kit : k));
    if (!(await saveKits(next))) return;
    selected = kit.id;
    editing = null;
    render(true);
    toast(t("kits.saved", { name: kit.name }), "ok");
  }

  // ---------------------------------------------------------------- what shows, back
  /** The card as it should be now (`force`: even if nothing it shows seems to have changed). */
  function render(force = false) {
    card.hidden = !ctx.state || !kits;
    if (editing) {
      // Leaving full screen while editing: the editor waits for the next time.
      if (!full()) renderCompact();
      return;
    }
    if (full()) renderFull(force);
    else {
      if (force) drawnKey = "";
      renderCompact();
    }
    drawMenu();
  }

  /** Back (or Esc): out of the editor to the kits, then out of full screen. */
  async function back() {
    if (menu) return closeMenu();
    if (pop) return closePop();
    if (editing) return cancelEdit();
    ctx.layout?.setMaximized(null);
  }

  $("kits-back").addEventListener("click", back);
  // Esc while editing, renaming or asking how many is ours (the card stays full screen).
  document.addEventListener("keydown", (e) => {
    if (e.key !== "Escape" || !full() || !$("modal").hidden || renaming) return;
    if (pop || editing) back();
  });

  // Full screen or not: the card draws itself for it.
  new MutationObserver(() => {
    closeMenu();
    if (editing && full() && !main.querySelector(".kit-editor")) renderEditor();
    else render(true);
    if (full()) loadCatalog().then(() => !editing && render(true));
  }).observe(card, { attributes: true, attributeFilter: ["class"] });

  // The players (for Give to) while the card shows.
  setInterval(async () => {
    if (!ctx.expanded || editing) return;
    await loadPlayers();
    render();
  }, PLAYERS_EVERY);

  invoke("kits_built_in")
    .then((ids) => {
      builtIns = new Set(ids);
      render(true);
    })
    .catch(() => {});
  invoke("kits_load")
    .then((list) => (kits = list))
    .catch(() => (kits = []))
    .finally(() => render(true));

  // A player typed /droplaunch kit in the game: their inventory is a kit now.
  window.__TAURI__.event.listen("kit-saved", async (e) => {
    kits = await invoke("kits_load").catch(() => kits);
    if (!editing) selected = e.payload.id;
    render(true);
    toast(t("kits.fromGame.toast", { name: e.payload.name }), "ok");
  });

  /** Kit files dropped on DropLaunch: added; what isn't a kit is handed back (language files). */
  async function importFiles(paths) {
    try {
      const reply = await invoke("kits_import", { paths });
      if (reply.imported.length) {
        kits = await invoke("kits_load");
        selected = reply.imported.at(-1).id;
        render(true);
        toast(reply.imported.length === 1 ? t("kits.imported", { name: reply.imported[0].name }) : t("kits.importedMany", { n: reply.imported.length }), "ok");
      }
      return reply.rest;
    } catch (err) {
      toast(String(err), "error");
      return [];
    }
  }

  return {
    render: () => render(),
    importFiles,
    /** The kit chosen in the card (Settings, Dropped assets). */
    selectedId: () => current()?.id ?? null,
    select(id) {
      selected = id;
      render(true);
    },
    /** The kits again from their files (they may have changed in Settings). */
    async reload() {
      kits = await invoke("kits_load").catch(() => kits);
      render(true);
    },
    /** Esc belongs to the kits (an amount, the editor, a rename), not to leaving full screen. */
    takesEscape: () => Boolean(menu) || (full() && (Boolean(pop) || Boolean(editing) || renaming)),
  };
}
