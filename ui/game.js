// Server files page: the Rust game on this PC and its map cache (v1's client cleanup). Rust keeps
// every map it loaded and can load an old copy of a map saved again under the same name, so
// DropLaunch removes a map's copies before starting it, and the whole cache can be cleared.
import { $, el, icon, invoke, fmtBytes, toast, confirmDialog } from "./util.js";
import { t, tn } from "./i18n.js";

export function initGame(ctx) {
  let info = null;

  async function load() {
    try {
      info = await invoke("client_cache");
    } catch {
      info = null;
    }
    render();
  }

  function render() {
    if (!ctx.state) return;
    const found = Boolean(info?.gameDir);
    const sub = $("game-sub");
    sub.textContent = !info ? t("game.looking") : found ? info.gameDir : t("game.notFound");
    sub.title = info?.gameDir ?? "";
    sub.classList.toggle("path-color", found);
    $("btn-game-cache").disabled = !found;

    const maps = info?.maps ?? 0;
    // Removing a map's old copies before it starts is switched in Settings.
    $("game-body").replaceChildren(
      el(
        "dl",
        { class: "kv" },
        el(
          "div",
          {},
          el("dt", { text: t("game.cache") }),
          el("dd", { class: found ? "" : "missing", text: !found ? "—" : maps ? tn("game.cache.maps", maps, { size: fmtBytes(info.bytes) }) : t("game.cache.empty") }),
        ),
      ),
      el("p", { class: "folder-note", text: ctx.state.cleanClientMaps !== false ? t("game.note.on") : t("game.note.off") }),
      el(
        "button",
        { class: "btn btn-small btn-ghost btn-block", disabled: maps ? null : true, title: maps ? "" : t("game.clear.nothing"), onclick: clear },
        icon("trash"),
        el("span", { text: t("game.clear") }),
      ),
    );
  }

  async function clear() {
    if (!info?.maps) return;
    const ok = await confirmDialog({
      title: t("game.clear.title"),
      text: [tn("game.clear.text", info.maps, { size: fmtBytes(info.bytes) }), ctx.state.backupClientMaps ? t("game.clear.backup") : ""].filter(Boolean).join(" "),
      ok: t("game.clear.ok"),
    });
    if (!ok) return;
    try {
      const reply = await invoke("clear_client_cache");
      info = reply.cache;
      const { removed, bytes, inUse } = reply.cleaned;
      const size = { size: fmtBytes(bytes) };
      const done = !removed ? t("game.cleared.none") : ctx.state.backupClientMaps ? tn("game.cleared.backedUp", removed, size) : tn("game.cleared", removed, size);
      toast(inUse ? tn("game.inUse", inUse, { done }) : done, inUse ? "warn" : "ok");
    } catch (err) {
      toast(String(err), "error");
    }
    render();
  }

  $("btn-game-cache").addEventListener("click", () => {
    if (info?.cacheDir) invoke("open_folder", { path: info.cacheDir }).catch((err) => toast(String(err)));
  });
  // Starting a map removes its copies, and Rust adds maps while it plays: look again on focus
  // (when this page shows; opening it looks too).
  window.addEventListener("focus", () => !$("view-files").hidden && load());
  $("tab-files").addEventListener("click", load);

  return { load, render };
}
