// Settings (the gear in the top bar): a page over the whole window that slides down from the top,
// with a tab for each kind of setting (the language, servers, notifications, the guide), what was
// dropped on DropLaunch (by kind, to pick from), Rent a server and About. Language files can be dropped on the window or the exe too; switching
// languages reloads the window (servers and SteamCMD keep going).
import { $, el, icon, invoke, toast, fmtWhen, confirmDialog, fitPath } from "./util.js";
import { t, tn, language } from "./i18n.js";
import { rentPage } from "./rent.js";

const ADDED_KEY = "droplaunch.languageAdded";
/** How long the page takes to slide away (the transition in styles.css). */
const SLIDE = 320;

/** The tabs in order. `tone`: the tab's colour (styles.css); `apart`: starts the lower group. */
const TABS = [
  { id: "servers", icon: "server", tone: "green", title: "settings.servers", lead: "settings.servers.lead" },
  { id: "notifications", icon: "bell", tone: "orange", title: "settings.notifications", lead: "settings.notifications.lead" },
  { id: "guide", icon: "bulb", tone: "yellow", title: "settings.guide", lead: "settings.guide.lead" },
  { id: "assets", icon: "upload", tone: "pink", title: "settings.assets", lead: "settings.assets.lead" },
  { id: "language", icon: "globe", tone: "violet", title: "settings.language", lead: "settings.language.lead" },
  { id: "rent", icon: "cloud", tone: "cyan", title: "rent.title", lead: "rent.lead", apart: true },
  { id: "about", icon: "info", tone: "blue", title: "settings.about", lead: "settings.about.lead" },
];

export function initSettings(ctx) {
  const page = $("settings");
  const nav = $("settings-nav");
  const body = $("settings-body");
  let languages = language.languages ?? [];
  let current = TABS[0].id;
  /** Filled in once; refresh() brings the values up to date. */
  let built = null;
  let hideTimer = 0;

  // ---------------------------------------------------------------- building blocks
  const text = (title, note) =>
    el("span", { class: "set-text" }, el("span", { class: "set-title", text: title }), note ? el("span", { class: "set-note", text: note }) : null);

  /** A setting that's on or off: the whole row flips it. */
  function toggle(title, note, onChange) {
    const input = el("input", { type: "checkbox", role: "switch" });
    input.addEventListener("change", () => onChange(input.checked));
    const node = el("label", { class: "set-row" }, text(title, note), el("span", { class: "switch" }, input, el("span", { class: "switch-track", "aria-hidden": "true" })));
    return { node, input };
  }

  /** A row that does something (opens a folder, a window); `iconName` at its end says what. */
  function action(title, note, iconName, onclick, tip) {
    return el("button", { type: "button", class: "set-row set-action", title: tip ?? null, onclick }, text(title, note), el("span", { class: "set-go" }, icon(iconName)));
  }

  const card = (...rows) => el("div", { class: "set-card" }, ...rows);
  /** What comes with DropLaunch (the procedural map's start file, English, Basic Build Kit). */
  const builtInTag = () => el("span", { class: "builtin-tag", text: t("common.builtIn") });
  const group = (title) => el("h4", { class: "set-group", text: title });

  // ---------------------------------------------------------------- the tabs
  function hero(tab) {
    return el(
      "header",
      { class: "set-hero" },
      el("span", { class: "set-hero-icon" }, icon(tab.icon)),
      el("div", {}, el("h3", { id: `settings-h-${tab.id}`, text: t(tab.title) }), el("p", { text: t(tab.lead) })),
    );
  }

  function build() {
    const b = {};
    b.languageTiles = el("div", { class: "lang-tiles", role: "radiogroup", "aria-label": t("settings.language.pick") });
    b.updateCheck = toggle(t("settings.updates"), t("settings.updates.note"), (on) => ctx.setOption({ updateCheck: on }));
    b.tips = toggle(t("settings.guide.tips"), t("settings.guide.tips.note"), (on) => ctx.guide.setOn(on));
    b.autoJoin = toggle(t("settings.autoJoin"), t("settings.autoJoin.note"), (on) => ctx.setOption({ autoJoin: on }));
    b.cleanMaps = toggle(t("settings.cleanMaps"), t("settings.cleanMaps.note"), (on) => ctx.setOption({ cleanClientMaps: on }));
    b.backupMaps = toggle(t("settings.backupMaps"), t("settings.backupMaps.note"), (on) => ctx.setOption({ backupClientMaps: on }));
    b.whitelist = toggle(t("settings.whitelist"), t("settings.whitelist.note"), (on) => ctx.setOption({ whitelist: on }));
    b.whitelistBox = el("div", { class: "wl" });
    b.bugsNote = el("span", { class: "set-note" });
    b.updateFramework = toggle(t("settings.updateFramework"), t("settings.updateFramework.note"), (on) => ctx.setOption({ updateFramework: on }));
    b.notifySaves = toggle(t("settings.notifySaves"), t("settings.notifySaves.note"), (on) => ctx.setOption({ notifySaves: on }));
    b.desktop = toggle(t("settings.desktop"), t("settings.desktop.note"), (on) => ctx.setOption({ desktopNotifications: on }));
    b.steps = el("ol", { class: "set-steps" });
    b.stepCount = el("span", { class: "set-count" });
    b.stepBar = el("span", { class: "set-bar", "aria-hidden": "true" });
    b.version = el("span", { class: "set-brand-version" });
    b.configNote = el("span", { class: "set-note path-note" });

    b.assetKinds = el("div", { class: "asset-kinds", role: "tablist" });
    b.assetList = el("div", { class: "set-card asset-list" });
    b.assetNote = el("p", { class: "set-note asset-note" });
    const content = {
      assets: [b.assetKinds, b.assetList, b.assetNote],
      language: [
        group(t("settings.language.pick")),
        b.languageTiles,
        el("p", { class: "set-drop" }, icon("upload"), el("span", { text: t("settings.language.note") })),
        group(t("settings.language.files")),
        card(
          action(t("settings.language.folder"), t("settings.language.folder.note"), "folder", openFolder),
          action(t("settings.language.export"), t("settings.language.export.note"), "download", exportEnglish),
        ),
      ],
      servers: [
        card(b.autoJoin.node, b.cleanMaps.node, b.backupMaps.node, action(t("settings.backupMaps.folder"), t("settings.backupMaps.folder.note"), "folder", openBackups), b.updateFramework.node),
        group(t("settings.whitelist.group")),
        card(b.whitelist.node, b.whitelistBox),
        group(t("settings.bugs.group")),
        card(
          el(
            "button",
            { type: "button", class: "set-row set-action", onclick: clearBugReports },
            el("span", { class: "set-text" }, el("span", { class: "set-title", text: t("settings.bugs.clear") }), b.bugsNote),
            el("span", { class: "set-go" }, icon("trash")),
          ),
        ),
      ],
      notifications: [card(b.notifySaves.node, b.desktop.node)],
      guide: [
        card(b.tips.node),
        card(
          el("div", { class: "set-row set-progress" }, el("span", { class: "set-title", text: t("guide.title") }), b.stepBar, b.stepCount),
          b.steps,
          action(t("guide.startOver"), t("guide.startOver.title"), "refresh", startGuideOver),
        ),
      ],
      rent: rentPage(),
      about: [
        el(
          "div",
          { class: "set-brand" },
          el("img", { src: "logo.png", alt: "", class: "set-brand-logo" }),
          el("div", {}, el("div", { class: "set-brand-name" }, el("span", { text: "DropLaunch" }), el("span", { class: "tag", text: t("topbar.preview") })), b.version),
        ),
        card(
          b.updateCheck.node,
          action(t("footer.licenses"), t("settings.licenses.note"), "arrow-right", () => ctx.about.open(), t("footer.licenses.title")),
          // Where the settings file is, under its name.
          el(
            "button",
            { type: "button", class: "set-row set-action", title: t("settings.configFolder.title"), onclick: showConfig },
            el("span", { class: "set-text" }, el("span", { class: "set-title", text: t("settings.configFolder") }), b.configNote),
            el("span", { class: "set-go" }, icon("folder")),
          ),
        ),
        card(el("button", { type: "button", class: "set-row set-action set-danger", onclick: resetEverything }, text(t("settings.reset"), t("settings.reset.note")), el("span", { class: "set-go" }, icon("refresh")))),
      ],
    };

    b.tabs = {};
    b.panes = {};
    for (const tab of TABS) {
      const button = el(
        "button",
        {
          type: "button",
          role: "tab",
          id: `settings-tab-${tab.id}`,
          class: `settings-tab${tab.apart ? " apart" : ""}`,
          "aria-controls": `settings-pane-${tab.id}`,
          "data-tone": tab.tone,
          onclick: () => show(tab.id),
        },
        el("span", { class: "settings-tab-icon" }, icon(tab.icon)),
        el("span", { class: "settings-tab-label", text: t(tab.title) }),
      );
      button.title = t(tab.title);
      b.tabs[tab.id] = button;
      b.panes[tab.id] = el(
        "section",
        { class: `settings-pane pane-${tab.id}`, id: `settings-pane-${tab.id}`, role: "tabpanel", "aria-labelledby": `settings-h-${tab.id}`, "data-tone": tab.tone, hidden: true },
        hero(tab),
        ...content[tab.id],
      );
    }
    nav.replaceChildren(...TABS.map((tab) => b.tabs[tab.id]));
    body.replaceChildren(...TABS.map((tab) => b.panes[tab.id]));
    nav.addEventListener("keydown", navKeys);
    built = b;
    show(current, false);
  }

  /** Up and down (or Home, End) move between the tabs. */
  function navKeys(e) {
    const index = TABS.findIndex((tab) => tab.id === current);
    const to = { ArrowDown: index + 1, ArrowUp: index - 1, Home: 0, End: TABS.length - 1 }[e.key];
    if (to == null) return;
    e.preventDefault();
    const tab = TABS[(to + TABS.length) % TABS.length];
    show(tab.id);
    built.tabs[tab.id].focus();
  }

  function show(id, animate = true) {
    current = id;
    if (id === "assets") loadAssets();
    if (id === "servers") loadBugCount();
    for (const tab of TABS) {
      const on = tab.id === id;
      built.tabs[tab.id].setAttribute("aria-selected", String(on));
      built.tabs[tab.id].tabIndex = on ? 0 : -1;
      const pane = built.panes[tab.id];
      pane.hidden = !on;
      if (on && animate) {
        pane.classList.remove("enter");
        void pane.offsetWidth;
        pane.classList.add("enter");
      }
    }
    body.scrollTop = 0;
  }

  // ---------------------------------------------------------------- dropped assets
  /** What assets_list said: { maps, zips, languages, startFiles, kits, kitsDir }. */
  let assets = null;
  let assetKind = "maps";
  const ASSET_KINDS = [
    { id: "maps", icon: "map", title: t("settings.assets.maps"), empty: t("settings.assets.empty.maps") },
    { id: "zips", icon: "folder", title: t("settings.assets.zips"), empty: t("settings.assets.empty.zips") },
    { id: "languages", icon: "globe", title: t("settings.assets.languages"), empty: "" },
    { id: "startFiles", icon: "file", title: t("settings.assets.startFiles"), empty: t("settings.assets.empty.startFiles") },
    { id: "kits", icon: "gift", title: t("settings.assets.kits"), empty: t("settings.assets.empty.kits") },
  ];
  const samePath = (a, b) => Boolean(a && b) && a.replaceAll("/", "\\").toLowerCase() === b.replaceAll("/", "\\").toLowerCase();
  const selectedMap = () => ctx.maps?.selected();

  async function loadAssets() {
    try {
      assets = await invoke("assets_list");
    } catch (err) {
      toast(String(err), "error");
      return;
    }
    renderAssets();
  }

  const reveal = (path) => invoke("reveal", { path }).catch((err) => toast(String(err), "error"));

  async function forgetAsset(path) {
    await invoke("assets_forget", { path }).catch((err) => toast(String(err), "error"));
    ctx.maps?.reload();
    loadAssets();
  }

  /** Each kind's rows: { key, title, sub, chosen, missing, choose, actions }. */
  function assetRows(kind) {
    const when = (secs, key) => (secs ? t(key, { when: fmtWhen(secs) }) : null);
    const showBtn = (path) => ({ icon: "folder", title: t("common.showExplorer"), run: () => reveal(path) });
    const forgetBtn = (path) => ({ icon: "x", title: t("settings.assets.forget"), run: () => forgetAsset(path) });
    const sel = selectedMap();
    switch (kind) {
      case "maps":
        return assets.maps.map((m) => ({
          key: m.path,
          title: m.name,
          sub: [when(m.added, "settings.assets.dropped")].filter(Boolean).join(" · "),
          path: m.path,
          missing: !m.exists,
          chosen: !sel?.procgen && samePath(sel?.path, m.path),
          // Only a map the list shows can be selected (one dropped but never started isn't there).
          choose: m.exists && ctx.maps?.has(m.path) ? () => ctx.maps.select(m.path) : null,
          actions: [
            m.exists ? { icon: "play", title: t("settings.assets.start"), run: () => (close(), ctx.maps.start({ path: m.path, name: m.name })) } : null,
            m.exists ? showBtn(m.path) : null,
            forgetBtn(m.path),
          ],
        }));
      case "zips":
        return assets.zips.map((z) => ({
          key: z.path,
          title: z.name,
          sub: [tn("settings.assets.zipMaps", z.made.length), when(z.added, "settings.assets.dropped")].filter(Boolean).join(" · "),
          path: z.path,
          missing: !z.exists,
          chosen: z.made.some((m) => samePath(sel?.path, m)),
          choose: z.made.length ? () => ctx.maps.select(z.made[0]) : null,
          actions: [z.exists ? showBtn(z.path) : z.made.length ? showBtn(z.made[0]) : null, forgetBtn(z.path)],
        }));
      case "languages":
        return assets.languages.map((l) => ({
          key: l.code,
          title: l.name,
          sub: [sourceNote(l), when(l.added, "settings.assets.dropped")].filter(Boolean).join(" · "),
          path: l.path,
          builtIn: l.source === "builtIn",
          chosen: l.code === language.code,
          choose: () => switchTo(l.code),
          actions: [l.path ? showBtn(l.path) : null],
        }));
      case "startFiles": {
        const forMap = (f) => (f.map ? !sel?.procgen && samePath(sel?.path, f.map) : Boolean(sel?.procgen));
        const branch = ctx.state?.branch;
        const onBranch = (f) => !branch || !assets.startFiles.some((o) => o.branch === branch) || f.branch === branch;
        // Only DropLaunch's normal file for a map is the one Start uses (the selected map's, on this
        // branch); a prefab one or an older file of your own can be started or shown, not chosen.
        const usable = (f) => f.own && !f.prefab;
        const used = assets.startFiles.find((f) => usable(f) && forMap(f) && onBranch(f));
        // Files that differ only in their map (and its name) are one start file: a row per set of
        // settings.
        const groups = new Map();
        for (const f of assets.startFiles) {
          const key = `${f.own}|${f.settings.join("\n")}`;
          if (!groups.has(key)) groups.set(key, []);
          groups.get(key).push(f);
        }
        return [...groups.values()].map((files) => {
          const f = files[0];
          // A server setting's value ("port" -> "28015").
          const arg = (name) => f.settings.find((a) => a.startsWith(`+server.${name} `))?.slice(name.length + 9);
          const maps = [...new Set(files.map((x) => x.mapName))];
          const kind = !f.own ? t("settings.assets.start.other") : f.prefab ? t("settings.assets.start.prefab") : !f.map ? t("settings.assets.start.procgen") : t("settings.assets.start.map");
          const one = files.length === 1;
          const listed = !f.map || ctx.maps?.has(f.map);
          const start = one && f.map && (f.prefab || !listed);
          const branches = (ctx.state?.installs?.length ?? 0) > 1 ? [...new Set(files.map((x) => x.branch))] : [];
          return {
            key: files.map((x) => x.path).join("|"),
            title: kind,
            sub: [
              tn("settings.assets.start.maps", maps.length, { maps: maps.slice(0, 3).join(", ") + (maps.length > 3 ? ", …" : "") }),
              arg("port") ? t("settings.assets.start.port", { port: arg("port") }) : null,
              arg("maxplayers") ? t("settings.assets.start.players", { n: arg("maxplayers") }) : null,
              branches.length ? branches.map((b) => t("settings.assets.branch", { branch: b })).join(", ") : null,
              when(Math.max(...files.map((x) => x.modified)), "settings.assets.written"),
            ]
              .filter(Boolean)
              .join(" · "),
            subTitle: [files.map((x) => x.name).join("\n"), "", ...f.settings].join("\n"),
            // DropLaunch's own procedural map file.
            builtIn: f.own && !f.map,
            chosen: files.includes(used),
            // One map's file, when the list shows that map.
            choose: one && usable(f) && listed ? () => ctx.maps.select(f.map ?? "procgen") : null,
            actions: [
              start ? { icon: "play", title: f.prefab ? t("maps.prefab.title", { name: f.mapName }) : t("settings.assets.start"), run: () => (close(), ctx.maps.start({ path: f.map, name: f.mapName }, { prefab: f.prefab })) } : null,
              showBtn(f.path),
            ],
          };
        });
      }
      case "kits":
        return assets.kits.map((k) => ({
          key: k.id,
          title: k.name,
          sub: [tn("kits.count", k.items), k.path.split(/[\\/]/).pop()].join(" · "),
          builtIn: k.builtIn,
          chosen: ctx.kits?.selectedId() === k.id,
          choose: () => ctx.kits?.select(k.id),
          actions: [showBtn(k.path)],
        }));
      default:
        return [];
    }
  }

  function renderAssets() {
    if (!built || !assets) return;
    built.assetKinds.replaceChildren(
      ...ASSET_KINDS.map((k) =>
        el(
          "button",
          { type: "button", role: "tab", class: "asset-kind", "aria-selected": String(k.id === assetKind), onclick: () => ((assetKind = k.id), renderAssets()) },
          icon(k.icon),
          el("span", { text: k.title }),
          el("span", { class: "asset-kind-count", text: String(k.id === "startFiles" ? assetRows(k.id).length : assets[k.id].length) }),
        ),
      ),
    );
    const kind = ASSET_KINDS.find((k) => k.id === assetKind);
    const rows = assetRows(assetKind);
    built.assetList.replaceChildren(
      ...(rows.length
        ? rows.map((r) =>
            el(
              "div",
              {
                class: `asset-row${r.chosen ? " chosen" : ""}${r.missing ? " missing" : ""}${r.choose ? "" : " fixed"}`,
                role: "radio",
                "aria-checked": String(Boolean(r.chosen)),
                "aria-disabled": r.choose ? null : "true",
                tabindex: r.choose ? "0" : "-1",
                title: r.choose ? t("settings.assets.choose") : null,
                onclick: () => r.choose && (r.choose(), setTimeout(renderAssets, 50)),
                onkeydown: (e) => (e.key === "Enter" || e.key === " ") && e.target === e.currentTarget && r.choose && (e.preventDefault(), r.choose(), setTimeout(renderAssets, 50)),
              },
              el("span", { class: "asset-mark" }, r.chosen ? icon("check") : null),
              el(
                "span",
                { class: "asset-text" },
                el("span", { class: "asset-title" }, el("span", { text: r.title }), r.builtIn ? builtInTag() : null, r.chosen ? el("span", { class: "asset-badge", text: t("settings.language.inUse") }) : null, r.missing ? el("span", { class: "asset-badge bad", text: t("settings.assets.missing") }) : null),
                // A path ends the line, shortened in the middle when there's no room for all of it.
                r.path
                  ? fitPath(el("span", { class: "asset-sub", title: [r.sub, r.path].filter(Boolean).join(" · ") }), r.path, { before: r.sub ? `${r.sub} · ` : "" })
                  : el("span", { class: "asset-sub", text: r.sub, title: r.subTitle ?? r.sub }),
              ),
              el(
                "span",
                { class: "asset-actions" },
                r.actions.filter(Boolean).map((a) => el("button", { type: "button", class: "icon-btn", title: a.title, onclick: (e) => (e.stopPropagation(), a.run()) }, icon(a.icon))),
              ),
            ),
          )
        : [el("p", { class: "set-note asset-empty", text: kind.empty })]),
    );
    const notes = { kits: t("settings.assets.note.kits"), languages: t("settings.assets.note.languages"), startFiles: t("settings.assets.note.startFiles") };
    built.assetNote.textContent = notes[assetKind] ?? t("settings.assets.note.maps");
    if (assetKind === "kits" && assets.kitsDir) {
      built.assetNote.append(" ", el("button", { type: "button", class: "btn btn-link", onclick: () => invoke("open_folder", { path: assets.kitsDir }).catch((err) => toast(String(err), "error")) }, el("span", { text: t("settings.assets.kitsFolder") })));
    }
  }

  // ---------------------------------------------------------------- values
  /** Where a language file is (a built-in one gets the Built in tag instead). */
  const sourceNote = (l) => (l.source === "folder" ? t("settings.language.fromFolder") : l.source === "added" ? t("settings.language.fromAdded") : null);

  let tilesKey = "";
  function renderLanguages() {
    const key = JSON.stringify([language.code, languages.map((l) => [l.code, l.name, l.source])]);
    if (key === tilesKey) return;
    tilesKey = key;
    built.languageTiles.replaceChildren(
      ...languages.map((l) => {
        const on = l.code === language.code;
        return el(
          "button",
          { type: "button", class: `lang-tile${on ? " on" : ""}`, role: "radio", "aria-checked": String(on), onclick: () => switchTo(l.code) },
          el("span", { class: "lang-code", text: l.code.toUpperCase() }),
          el("span", { class: "lang-text" }, el("span", { class: "lang-name", text: l.name }), l.source === "builtIn" ? el("span", { class: "lang-source" }, builtInTag()) : el("span", { class: "lang-source", text: sourceNote(l) })),
          on ? el("span", { class: "lang-in-use" }, icon("check"), el("span", { text: t("settings.language.inUse") })) : null,
        );
      }),
    );
  }

  function renderSteps() {
    const steps = ctx.guide?.steps() ?? [];
    const done = steps.filter((s) => s.done).length;
    built.stepCount.textContent = t("guide.count", { n: done, total: steps.length });
    built.stepBar.replaceChildren(...steps.map((s) => el("i", { class: s.done ? "done" : "" })));
    built.steps.replaceChildren(
      ...steps.map((s) => el("li", { class: s.done ? "done" : "" }, el("span", { class: "set-step-mark" }, s.done ? icon("check") : null), el("span", { text: s.title }))),
    );
  }

  /** The values as they are now (the language list may have grown). */
  function refresh() {
    const s = ctx.state;
    const b = built;
    renderLanguages();
    if (!s) return;
    b.updateCheck.input.checked = s.updateCheck !== false;
    b.tips.input.checked = Boolean(s.guide?.on);
    b.autoJoin.input.checked = Boolean(s.autoJoin);
    b.cleanMaps.input.checked = s.cleanClientMaps !== false;
    b.backupMaps.input.checked = Boolean(s.backupClientMaps);
    renderWhitelist();
    b.updateFramework.input.checked = s.updateFramework !== false;
    b.notifySaves.input.checked = s.notifySaves !== false;
    b.desktop.input.checked = s.desktopNotifications !== false;
    b.version.textContent = [t("about.version", { version: s.version }), t("about.madeBy")].join(" · ");
    b.configNote.textContent = s.configFile;
    b.configNote.title = s.configFile;
    renderSteps();
  }

  // ---------------------------------------------------------------- bug reports
  /** How many bug reports are kept (the clear row says so). */
  let bugCount = null;
  async function loadBugCount() {
    try {
      bugCount = await invoke("bug_reports_summary");
    } catch {
      bugCount = null;
    }
    if (!built) return;
    const kept = !bugCount ? "" : bugCount.reports ? tn("settings.bugs.count", bugCount.reports, { maps: tn("settings.bugs.maps", bugCount.maps) }) : t("settings.bugs.none");
    built.bugsNote.textContent = [kept, t("settings.bugs.note")].filter(Boolean).join(" ");
  }

  /** Every map's bug reports put aside, numbering from #1 again (asks first). */
  async function clearBugReports() {
    if (bugCount && !bugCount.reports) {
      toast(t("settings.bugs.nothing"));
      return;
    }
    const ok = await confirmDialog({
      title: t("settings.bugs.title"),
      text: t("settings.bugs.text"),
      choices: [{ label: t("settings.bugs.ok"), value: true, danger: true }],
    });
    if (!ok) return;
    try {
      const n = await invoke("bug_reports_clear");
      toast(tn("settings.bugs.done", n), "ok");
    } catch (err) {
      toast(String(err), "error");
    }
    loadBugCount();
  }

  // ---------------------------------------------------------------- reset
  /** Every setting back to its default, then the window starts over (the welcome screen shows). */
  async function resetEverything() {
    if (ctx.busy?.()) {
      toast(t("settings.reset.busy"), "warn");
      return;
    }
    const ok = await confirmDialog({
      title: t("settings.reset.title"),
      text: t("settings.reset.text"),
      choices: [{ label: t("settings.reset.ok"), value: true, danger: true }],
    });
    if (!ok) return;
    try {
      await invoke("reset_settings");
    } catch (err) {
      toast(String(err), "error");
      return;
    }
    // The window's own choices too (view, folded cards, sizes, the last tab…).
    try {
      for (const key of Object.keys(localStorage)) if (key.startsWith("droplaunch.")) localStorage.removeItem(key);
      sessionStorage.clear();
    } catch {
      /* nothing kept */
    }
    location.reload();
  }

  // ---------------------------------------------------------------- the whitelist
  /** Who can join: the owner (always), the players added, a box to add one. */
  function renderWhitelist() {
    const s = ctx.state;
    const b = built;
    const owner = s.owner;
    const framework = s.framework && s.framework !== "none";
    b.whitelist.input.checked = Boolean(s.whitelist);
    b.whitelist.input.disabled = !owner || !framework;
    b.whitelist.node.classList.toggle("disabled", !owner || !framework);
    if (!owner || !framework) {
      b.whitelistBox.replaceChildren(
        el(
          "div",
          { class: "wl-need" },
          icon("info"),
          el("span", { text: !framework ? t("settings.whitelist.needsFramework") : t("settings.whitelist.needsOwner") }),
          el("button", { type: "button", class: "btn btn-small btn-ghost", onclick: () => (close(), ctx.openSetup?.()) }, el("span", { text: t("mods.setupOptions") })),
        ),
      );
      return;
    }
    const remove = (id) => ctx.setOption({ whitelistIds: (s.whitelistIds ?? []).filter((x) => x !== id) });
    const input = el("input", { type: "text", class: "text-input wl-input", placeholder: t("settings.whitelist.placeholder"), spellcheck: "false", autocomplete: "off", "aria-label": t("settings.whitelist.add") });
    const add = async () => {
      const id = input.value.trim();
      if (!id) return input.focus();
      if (await ctx.setOption({ whitelistIds: [...(s.whitelistIds ?? []), id] })) input.value = "";
      else input.focus();
    };
    input.addEventListener("keydown", (e) => e.key === "Enter" && add());
    const row = (id, name, badge, onRemove) =>
      el(
        "div",
        { class: "wl-row" },
        name ? el("span", { class: "wl-pic", text: (name.trim()[0] ?? "?").toUpperCase() }) : el("span", { class: "wl-pic" }, icon("users")),
        el("span", { class: "wl-text" }, name ? el("b", { text: name }) : null, el("code", { text: id })),
        badge ? el("span", { class: "wl-badge", text: badge }) : null,
        onRemove ? el("button", { type: "button", class: "icon-btn", title: t("settings.whitelist.remove"), onclick: onRemove }, icon("x")) : null,
      );
    b.whitelistBox.classList.toggle("off", !s.whitelist);
    b.whitelistBox.replaceChildren(
      el(
        "div",
        { class: "wl-list" },
        row(owner.steamId, owner.name, t("settings.whitelist.owner"), null),
        ...(s.whitelistIds ?? []).filter((id) => id !== owner.steamId).map((id) => row(id, "", null, () => remove(id))),
      ),
      el("div", { class: "wl-add" }, input, el("button", { type: "button", class: "btn btn-small btn-primary", onclick: add }, icon("plus"), el("span", { text: t("settings.whitelist.add") }))),
    );
  }

  function openBackups() {
    invoke("open_folder", { path: ctx.state.mapBackupsDir }).catch(() => toast(t("settings.backupMaps.none")));
  }

  // ---------------------------------------------------------------- open, close
  /** `tab`: the one to show (else the last one). */
  async function open(tab) {
    if (!built) build();
    if (tab && built.tabs[tab]) show(tab, false);
    refresh();
    if (current === "assets") loadAssets();
    if (current === "servers") loadBugCount();
    clearTimeout(hideTimer);
    page.hidden = false;
    // Laid out above the window first, so it slides down.
    void page.offsetWidth;
    page.classList.add("open");
    built.tabs[current].focus({ preventScroll: true });
    try {
      languages = (await invoke("language_state")).languages;
      refresh();
    } catch {
      /* the list from the start */
    }
  }

  function close() {
    if (page.hidden || !page.classList.contains("open")) return;
    page.classList.remove("open");
    clearTimeout(hideTimer);
    hideTimer = setTimeout(() => (page.hidden = true), SLIDE);
    $("btn-settings").focus({ preventScroll: true });
  }

  // ---------------------------------------------------------------- what the rows do
  /** Another language: the window reloads in it (a running SteamCMD job would lose its progress there). */
  async function switchTo(code) {
    if (code === language.code) return;
    if (ctx.busy?.()) {
      toast(t("settings.language.busy"), "warn");
      return;
    }
    try {
      await invoke("set_language", { code });
      location.reload();
    } catch (err) {
      toast(String(err), "error");
    }
  }

  /** Language files dropped on the window or the exe: added, and the last one is used. */
  /** `use`: switch to the (last) one added; else it's only added (Settings has it). */
  async function addLanguages(paths, { use = true } = {}) {
    let reply;
    try {
      reply = await invoke("add_languages", { paths });
    } catch (err) {
      toast(String(err), "error");
      return;
    }
    if (reply.problems.length) toast(reply.problems.join(" · "), "error");
    const added = reply.added.at(-1);
    if (!added) return;
    if (!use) {
      toast(t("settings.language.addedOnly", { name: added.name }), "ok");
      return;
    }
    if (ctx.busy?.()) {
      toast(t("settings.language.addedLater", { name: added.name }), "ok");
      return;
    }
    try {
      await invoke("set_language", { code: added.code });
      sessionStorage.setItem(ADDED_KEY, added.name);
    } catch (err) {
      toast(String(err), "error");
      return;
    }
    location.reload();
  }

  async function openFolder() {
    try {
      await invoke("open_languages_folder");
    } catch (err) {
      toast(String(err), "error");
    }
  }

  async function exportEnglish() {
    try {
      const path = await invoke("export_language");
      if (path) toast(t("settings.language.exported", { path }), "ok");
    } catch (err) {
      toast(String(err), "error");
    }
  }

  async function startGuideOver() {
    await ctx.guide.setOn(true);
    await ctx.guide.startOver();
    refresh();
    toast(t("settings.guide.again"), "ok");
  }

  function showConfig() {
    if (ctx.state?.configFile) invoke("reveal", { path: ctx.state.configFile }).catch((err) => toast(String(err), "error"));
  }

  $("btn-settings").addEventListener("click", () => open());
  $("settings-close").addEventListener("click", close);
  // Before the windows that can open on top of it (a question, the licenses) close themselves on
  // the same Escape. Only Settings closes (not also what's under it, like the kit editor).
  const onTop = ["modal", "about", "app-update", "map-viewer"];
  document.addEventListener(
    "keydown",
    (e) => {
      if (e.key !== "Escape" || page.hidden || !onTop.every((id) => $(id).hidden)) return;
      e.stopImmediatePropagation();
      close();
    },
    true,
  );

  // Just added by dropping a file (the window reloaded in it).
  try {
    const name = sessionStorage.getItem(ADDED_KEY);
    if (name) {
      sessionStorage.removeItem(ADDED_KEY);
      setTimeout(() => toast(t("settings.language.added", { name }), "ok"), 400);
    }
  } catch {
    /* no session storage: no note */
  }

  return { open, close, addLanguages, render: () => built && !page.hidden && refresh() };
}
