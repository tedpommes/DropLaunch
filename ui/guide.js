// The guide, for people new to hosting servers and making maps: a tip about a step when it comes
// up (pointing at what it's about, once each), and the Getting started checklist on the Maps page.
// Switched on with "Show me around" on the first start, or in Settings (Guide).
import { $, el, icon, toast } from "./util.js";
import { t, rich } from "./i18n.js";

const OPEN_KEY = "droplaunch.guideOpen";
/** How often the guide looks whether a tip's moment has come. */
const EVERY = 250;
/** A tip waits this long once its moment has come (nothing flashes by while the page changes). */
const SETTLE = 400;
/** And this long after the last one. */
const QUIET = 500;

export function initGuide(ctx) {
  const card = $("guide-card");
  /** Saved but not in ctx.state yet. */
  const seenNow = new Set();
  const doneNow = new Set();
  /** The tip on screen: { def, node, ring, forced, misses, key, raf }. */
  let tip = null;
  /** The tip whose moment came, and since when (it shows once that lasted SETTLE). */
  let candidate = null;
  let quietUntil = 0;
  /** The checklist step shown with its text and Show me (null: the first one to do). */
  let focus = null;
  /** Steps ticked just now (they get a little animation). */
  const fresh = new Set();
  let cardKey = "";

  const guide = () => ctx.state?.guide ?? { on: false, seen: [], done: [] };
  const isOn = () => Boolean(guide().on);
  const seen = (id) => guide().seen.includes(id) || seenNow.has(id);
  const done = (id) => guide().done.includes(id) || doneNow.has(id);

  // ---------------------------------------------------------------- what's going on
  const view = () => ctx.server?.view();
  const live = () => Boolean(ctx.server?.isLive());
  const phase = () => (live() || view()?.phase === "crashed" ? view().phase : null);
  const players = () => view()?.info?.players ?? 0;
  const setupOpen = () => !$("setup").hidden;
  const setupOptions = () => setupOpen() && !$("setup-options").hidden && $("setup-tasks").hidden;
  const setupRunning = () => setupOpen() && !$("setup-tasks").hidden && !$("setup-cancel").hidden;
  const mapsPage = () => !$("view-maps").hidden && !ctx.playtest?.isOpen();
  const panel = (selector) => $("server-panel").querySelector(selector);
  // Texts (keys in the language file): {server} and {steamcmd} are download sizes, green or red
  // for the room on the drive.
  const textOf = (def) => (typeof def.text === "function" ? def.text() : def.text);
  const sized = (key) => rich(key, { server: ctx.spaceTag("server"), steamcmd: ctx.spaceTag("steamcmd") });
  const plain = (key) => t(key, { server: t("units.serverSize"), steamcmd: t("units.steamcmdSize") });

  // ---------------------------------------------------------------- the checklist
  const STEPS = [
    {
      id: "install",
      title: t("guide.step.install.title"),
      text: "guide.step.install.text",
      check: () => !ctx.state.setupNeeded,
      show: async () => {
        await ctx.openSetup();
        if ($("setup").hidden) toast(t("guide.toast.setupRunning"));
      },
    },
    {
      id: "start",
      title: t("guide.step.start.title"),
      text: "guide.step.start.text",
      check: () => live(),
      show: () => showMe("start", t("guide.toast.pickMap")),
    },
    {
      id: "join",
      title: t("guide.step.join.title"),
      text: "guide.step.join.text",
      check: () => phase() === "ready" && players() > 0,
      show: () => (phase() === "ready" ? showMe("ready") : toast(t("guide.toast.startFirst"))),
    },
    {
      id: "bug",
      title: t("guide.step.bug.title"),
      text: "guide.step.bug.text",
      check: () => (ctx.playtest?.openCount() ?? 0) > 0,
      show: () => {
        const s = ctx.state;
        if (s.framework === "none" || !s.playtest) {
          ctx.showFiles();
          const mods = $("mods-card");
          if (mods.classList.contains("folded")) mods.querySelector(".fold-btn").click();
          if (s.framework === "none") toast(t("guide.toast.needsFramework"));
          else showMe("bug-switch");
        } else if (phase() === "ready") showMe("bug-chat");
        else toast(t("guide.toast.startAndJoin"));
      },
    },
  ];

  /** Steps happen whether the guide is on or not: they're ticked once seen happening. */
  function trackSteps() {
    const ticked = STEPS.filter((s) => !done(s.id) && s.check()).map((s) => s.id);
    if (!ticked.length) return;
    for (const id of ticked) {
      doneNow.add(id);
      fresh.add(id);
      setTimeout(() => fresh.delete(id), 1600);
    }
    save({ done: ticked });
  }

  // ---------------------------------------------------------------- tips
  // In the order they're offered: the first one whose moment has come, that wasn't read yet and
  // whose target is on screen, shows. `manual`: only from Show me.
  const TIPS = [
    // The setup window.
    {
      id: "setup-downloads",
      when: () => setupOptions() && ctx.state.setupNeeded,
      target: () => $("setup-reqs"),
      title: t("tip.setupDownloads.title"),
      text: "tip.setupDownloads.text",
    },
    {
      id: "setup-framework",
      when: setupOptions,
      target: () => $("setup-framework"),
      title: t("common.modFramework"),
      text: "tip.setupFramework.text",
    },
    {
      id: "setup-owner",
      when: setupOptions,
      target: () => $("setup-owner-row"),
      title: t("tip.setupOwner.title"),
      text: "tip.setupOwner.text",
    },
    {
      id: "setup-go",
      when: () => setupOptions() && ctx.state.setupNeeded,
      target: () => $("setup-fix"),
      title: t("tip.setupGo.title"),
      text: "tip.setupGo.text",
    },
    {
      id: "setup-running",
      when: setupRunning,
      target: () => $("setup-tasks"),
      title: t("tip.setupRunning.title"),
      text: "tip.setupRunning.text",
    },
    // The Maps page.
    {
      id: "checklist",
      when: () => !card.hidden,
      target: () => card,
      title: t("tip.checklist.title"),
      text: "tip.checklist.text",
    },
    {
      id: "maps",
      when: () => mapsPage() && !ctx.state.setupNeeded && !live(),
      target: () => $("map-list"),
      title: t("tip.maps.title"),
      text: "tip.maps.text",
    },
    {
      id: "start",
      when: () => mapsPage() && !ctx.state.setupNeeded && !live() && !ctx.startBlocked(),
      target: () => (ctx.expanded ? $("btn-start") : $("map-list").querySelector(".map-row.selected") ?? $("map-list").querySelector(".map-row")),
      title: t("tip.start.title"),
      text: () => (ctx.expanded ? "tip.start.textMore" : "tip.start.text"),
    },
    {
      id: "booting",
      when: () => mapsPage() && phase() === "booting",
      target: () => panel(".boot"),
      title: t("tip.booting.title"),
      text: "tip.booting.text",
    },
    {
      id: "ready",
      when: () => mapsPage() && phase() === "ready" && players() === 0,
      target: () => panel(".join-row .btn-primary, .cp-join .btn-primary"),
      title: t("tip.ready.title"),
      text: "tip.ready.text",
    },
    {
      id: "joined",
      when: () => mapsPage() && phase() === "ready" && players() > 0,
      target: () => panel(".srv-actions .btn-ghost, .cp-buttons .btn-ghost"),
      title: t("tip.joined.title"),
      text: () => (ctx.state.owner ? "tip.joined.textOwner" : "tip.joined.text"),
    },
    {
      id: "stop",
      when: () => mapsPage() && phase() === "ready" && seen("joined"),
      target: () => panel(".srv-actions .btn-danger, .cp-buttons .btn-danger"),
      title: t("tip.stop.title"),
      text: "tip.stop.text",
    },
    {
      id: "crashed",
      when: () => mapsPage() && phase() === "crashed",
      target: () => panel(".srv-message.bad"),
      title: t("tip.crashed.title"),
      text: "tip.crashed.text",
    },
    {
      id: "more",
      when: () => !ctx.expanded && done("start") && !live(),
      target: () => $("btn-view"),
      title: t("topbar.more"),
      text: "tip.more.text",
    },
    {
      id: "update",
      when: () => mapsPage() && !$("strip").hidden && $("strip").classList.contains("update"),
      target: () => $("strip"),
      title: t("tip.update.title"),
      text: "tip.update.text",
    },
    // Only from Show me.
    {
      id: "bug-switch",
      manual: true,
      target: () => $("mods-playtest-row"),
      title: t("plugin.add"),
      text: "tip.bugSwitch.text",
    },
    {
      id: "bug-chat",
      manual: true,
      target: () => panel(".pt-open"),
      title: t("tip.bugChat.title"),
      text: "tip.bugChat.text",
    },
  ];

  /** Where a tip's target is on screen, or null: missing, too small, scrolled away or covered. */
  function onScreen(node) {
    if (!node?.isConnected) return null;
    const r = node.getBoundingClientRect();
    if (r.width < 4 || r.height < 4 || r.bottom < 0 || r.right < 0 || r.top > innerHeight || r.left > innerWidth) return null;
    const x = Math.min(Math.max(r.left + r.width / 2, 2), innerWidth - 2);
    const y = Math.min(Math.max(r.top + Math.min(r.height / 2, 20), 2), innerHeight - 2);
    // Not under a window on top (setup, a question): what's there is the target, or this tip.
    const hit = document.elementFromPoint(x, y);
    return hit && (node.contains(hit) || tip?.node.contains(hit)) ? r : null;
  }

  function choose() {
    if (tip) {
      const ok = (tip.forced || tip.def.when()) && onScreen(tip.def.target());
      // A moment's blink (a re-render) doesn't close it.
      tip.misses = ok ? 0 : tip.misses + 1;
      if (tip.misses > 2) hideTip();
      return;
    }
    if (Date.now() < quietUntil) return;
    const next = TIPS.find((d) => !d.manual && !seen(d.id) && d.when() && onScreen(d.target()));
    if (!next) {
      candidate = null;
      return;
    }
    if (candidate?.id !== next.id) candidate = { id: next.id, since: Date.now() };
    else if (Date.now() - candidate.since >= SETTLE) showTip(next, false);
  }

  function showTip(def, forced) {
    hideTip();
    candidate = null;
    const titleId = `guide-tip-${def.id}`;
    const node = el(
      "div",
      { class: "guide-tip", role: "dialog", "aria-labelledby": titleId },
      el("span", { class: "guide-arrow", "aria-hidden": "true" }),
      el("div", { class: "guide-tip-head" }, icon("bulb"), el("strong", { id: titleId, text: def.title })),
      el("p", {}, sized(textOf(def))),
      el(
        "div",
        { class: "guide-tip-actions" },
        el("button", { class: "btn btn-link guide-off", title: t("guide.tipsOff.title"), onclick: () => setOn(false) }, el("span", { text: t("guide.tipsOff") })),
        el("button", { class: "btn btn-small btn-primary", onclick: () => gotIt(def.id) }, el("span", { text: t("guide.gotIt") })),
      ),
    );
    const ring = el("div", { class: "guide-ring", "aria-hidden": "true" });
    document.body.append(ring, node);
    tip = { def, node, ring, forced, misses: 0, key: "", raf: 0 };
    follow();
  }

  /** Keeps the tip next to its target while things move (resizing, scrolling, cards growing). */
  function follow() {
    if (!tip) return;
    const r = tip.def.target()?.getBoundingClientRect();
    const key = r ? `${r.left}|${r.top}|${r.width}|${r.height}|${innerWidth}|${innerHeight}` : "";
    if (r && key !== tip.key) {
      tip.key = key;
      place(r);
    }
    tip.raf = requestAnimationFrame(follow);
  }

  /** Under the target if there's room, else above, beside, or (a big one) over its top. */
  function place(r) {
    const { node, ring } = tip;
    const w = node.offsetWidth;
    const h = node.offsetHeight;
    const gap = 12;
    const margin = 12;
    const clamp = (v, lo, hi) => Math.max(lo, Math.min(v, hi));
    let side = "inside";
    if (innerHeight - r.bottom >= h + gap + margin) side = "below";
    else if (r.top >= h + gap + margin) side = "above";
    else if (innerWidth - r.right >= w + gap + margin) side = "right";
    else if (r.left >= w + gap + margin) side = "left";
    let x;
    let y;
    if (side === "below" || side === "above") {
      x = clamp(r.left + r.width / 2 - w / 2, margin, innerWidth - w - margin);
      y = side === "below" ? r.bottom + gap : r.top - gap - h;
      node.style.setProperty("--arrow", `${clamp(r.left + r.width / 2 - x, 18, w - 18)}px`);
    } else if (side === "right" || side === "left") {
      y = clamp(r.top + r.height / 2 - h / 2, margin, innerHeight - h - margin);
      x = side === "right" ? r.right + gap : r.left - gap - w;
      node.style.setProperty("--arrow", `${clamp(r.top + r.height / 2 - y, 18, h - 18)}px`);
    } else {
      x = clamp(r.left + r.width / 2 - w / 2, margin, innerWidth - w - margin);
      y = clamp(r.top + 20, margin, innerHeight - h - margin);
    }
    node.dataset.side = side;
    node.style.left = `${Math.round(x)}px`;
    node.style.top = `${Math.round(y)}px`;
    // The ring stays inside the window (a card at the edge).
    const left = Math.max(2, r.left - 4);
    const top = Math.max(2, r.top - 4);
    ring.style.left = `${left}px`;
    ring.style.top = `${top}px`;
    ring.style.width = `${Math.min(innerWidth - 2, r.right + 4) - left}px`;
    ring.style.height = `${Math.min(innerHeight - 2, r.bottom + 4) - top}px`;
  }

  function hideTip() {
    if (!tip) return;
    cancelAnimationFrame(tip.raf);
    tip.node.remove();
    tip.ring.remove();
    tip = null;
  }

  /** `id`: the tip that was read (the one on screen, unless another one took its place). */
  function gotIt(id = tip?.def.id) {
    if (!id) return;
    if (tip?.def.id === id) hideTip();
    quietUntil = Date.now() + QUIET;
    if (!seen(id)) {
      seenNow.add(id);
      save({ seen: [id] });
    }
  }

  // Doing what the tip is about counts as read.
  document.addEventListener(
    "click",
    (e) => {
      if (!tip || tip.node.contains(e.target) || !tip.def.target()?.contains(e.target)) return;
      const id = tip.def.id;
      setTimeout(() => gotIt(id));
    },
    true,
  );

  /** Show me: that step's tip now (read or not), once its target is on screen. */
  async function showMe(id, fallback) {
    const def = TIPS.find((d) => d.id === id);
    hideTip();
    if (!["bug-switch"].includes(id) && $("view-maps").hidden) ctx.setTab("maps");
    if (ctx.playtest?.isOpen() && id !== "bug-chat") ctx.playtest.close();
    await new Promise((r) => setTimeout(r, 120));
    def.target()?.scrollIntoView({ block: "nearest" });
    await new Promise((r) => requestAnimationFrame(r));
    if (onScreen(def.target())) showTip(def, true);
    else toast(fallback ?? plain(textOf(def)));
  }

  // ---------------------------------------------------------------- the card
  let open = (() => {
    try {
      return localStorage.getItem(OPEN_KEY) === "1";
    } catch {
      return false;
    }
  })();

  function renderCard() {
    const allDone = STEPS.every((s) => done(s.id));
    const hidden = !isOn() || (allDone && seen("checklist-done"));
    const next = STEPS.find((s) => s.id === focus && !done(s.id)) ?? STEPS.find((s) => !done(s.id));
    const key = JSON.stringify([hidden, open, allDone, next?.id, STEPS.map((s) => done(s.id)), [...fresh]]);
    if (key === cardKey) return;
    cardKey = key;
    card.hidden = hidden;
    if (hidden) return;
    const count = STEPS.filter((s) => done(s.id)).length;
    const head = el(
      "div",
      { class: "guide-head" },
      el("span", { class: "guide-icon" }, icon("bulb")),
      el("h2", { text: allDone ? t("guide.allDone") : t("guide.title") }),
      el("span", { class: "guide-count", text: t("guide.count", { n: count, total: STEPS.length }) }),
      el("span", { class: "guide-bar", "aria-hidden": "true" }, ...STEPS.map((s) => el("i", { class: done(s.id) ? "done" : "" }))),
      el("span", { class: "spacer" }),
      allDone
        ? null
        : el(
            "button",
            { class: "icon-btn guide-fold", title: open ? t("guide.fold.less") : t("guide.fold.more"), "aria-expanded": String(open), onclick: () => setOpen(!open) },
            icon("chevron"),
          ),
      el("button", { class: "icon-btn", title: t("guide.off.title"), onclick: () => setOn(false) }, icon("x")),
    );
    if (allDone) {
      card.replaceChildren(
        head,
        el(
          "div",
          { class: "guide-next" },
          el("div", { class: "guide-next-text" }, el("span", { text: t("guide.basics") })),
          el("button", { class: "btn btn-small btn-ghost", onclick: () => save({ seen: ["checklist-done"] }) }, el("span", { text: t("guide.hide") })),
        ),
      );
      return;
    }
    const showButton = (s) => el("button", { class: "btn btn-small btn-primary", onclick: () => s.show() }, el("span", { text: t("guide.showMe") }));
    if (!open) {
      card.replaceChildren(
        head,
        el(
          "div",
          { class: "guide-next" },
          el("div", { class: "guide-next-text" }, el("b", { text: t("guide.next", { title: next.title }) }), el("span", {}, sized(next.text))),
          showButton(next),
        ),
      );
      return;
    }
    card.replaceChildren(
      head,
      el(
        "ol",
        { class: "guide-steps" },
        STEPS.map((s) => {
          const isDone = done(s.id);
          const current = s === next;
          return el(
            "li",
            {
              class: `guide-step${isDone ? " done" : ""}${current ? " current" : ""}${fresh.has(s.id) ? " just-done" : ""}`,
              onclick: isDone || current ? null : () => ((focus = s.id), render()),
              title: isDone || current ? null : t("guide.showStep"),
            },
            el("span", { class: "guide-mark" }, isDone ? icon("check") : null),
            el("span", { class: "guide-step-title", text: s.title }),
            current ? showButton(s) : null,
            current ? el("span", { class: "guide-step-text" }, sized(s.text)) : null,
          );
        }),
      ),
      el(
        "div",
        { class: "guide-foot" },
        el("button", { class: "btn btn-link", title: t("guide.startOver.title"), onclick: startOver }, el("span", { text: t("guide.startOver") })),
      ),
    );
  }

  function setOpen(on) {
    open = on;
    try {
      localStorage.setItem(OPEN_KEY, on ? "1" : "0");
    } catch {}
    render();
  }

  // ---------------------------------------------------------------- on, off, saved
  function save(patch) {
    return ctx.setOption({ guide: patch });
  }

  async function setOn(on) {
    hideTip();
    candidate = null;
    quietUntil = Date.now() + QUIET;
    await save({ on });
    if (!on) toast(t("guide.offToast"));
  }

  async function startOver() {
    hideTip();
    seenNow.clear();
    doneNow.clear();
    focus = null;
    await save({ reset: true });
  }

  function render() {
    if (!ctx.state) return;
    trackSteps();
    renderCard();
    if (!isOn()) {
      hideTip();
      candidate = null;
      return;
    }
    choose();
  }

  setInterval(render, EVERY);

  return {
    render,
    /** "Show me around" on the first start (and Settings). */
    setOn,
    startOver,
    /** The Getting started steps, for Settings: [{ id, title, done }]. */
    steps: () => STEPS.map((s) => ({ id: s.id, title: s.title, done: done(s.id) })),
  };
}
