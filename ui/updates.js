// New DropLaunch versions from GitHub (signed releases; DropLaunch swaps itself and restarts).
// A few seconds after start (and every few hours) DropLaunch looks for one; a new version opens
// a window asking to update (unless "Don't show this again" was ticked for that version), and
// the top bar's Update DropLaunch button opens it again.
import { $, invoke, Channel, fmtBytes, toast } from "./util.js";
import { t } from "./i18n.js";

const RELEASES = "https://github.com/tedpommes/Rust-DropLaunch/releases/latest";
const RECHECK_MS = 6 * 3600 * 1000;

export function initUpdates(ctx) {
  let update = null;
  let installing = false;
  let failed = false;

  async function check() {
    if (installing) return;
    try {
      update = await invoke("check_update");
    } catch {
      return; // offline, GitHub down: nothing to say
    }
    renderButton();
    if (update && update.version !== ctx.state?.skippedUpdate && $("app-update").hidden) open();
  }

  function renderButton() {
    const button = $("btn-update");
    button.hidden = !update;
    if (!update) return;
    button.disabled = installing;
    button.title = t("update.button.title", { version: update.version, current: update.current });
  }

  function open() {
    if (!update) return;
    failed = false;
    $("app-update-title").textContent = t("update.titleVersion", { version: update.version });
    const sub = { current: update.current, date: update.published, size: fmtBytes(update.size) };
    $("app-update-sub").textContent = update.published ? t("update.subReleased", sub) : t("update.sub", sub);
    const notes = $("app-update-notes");
    notes.textContent = update.notes || "";
    notes.hidden = !update.notes;
    const lines = [t("update.note.restart")];
    if (ctx.server?.isLive()) lines.push(t("update.note.server"));
    setNote(lines.join(" "));
    $("app-update-skip").checked = ctx.state?.skippedUpdate === update.version;
    $("app-update-skip-row").hidden = false;
    $("app-update-progress").hidden = true;
    $("app-update-go-label").textContent = t("update.button");
    $("app-update-go").disabled = false;
    $("app-update-later").disabled = false;
    $("app-update").hidden = false;
    $("app-update-go").focus();
  }

  function setNote(text, error = false) {
    const note = $("app-update-note");
    note.textContent = text;
    note.classList.toggle("error", error);
  }

  /** "Don't show this again" is kept for this version only. */
  async function close() {
    if (installing) return;
    $("app-update").hidden = true;
    const skip = $("app-update-skip").checked ? update?.version ?? "" : "";
    if ((ctx.state?.skippedUpdate ?? "") !== skip) await ctx.setOption({ skippedUpdate: skip });
  }

  async function install() {
    if (failed) {
      invoke("open_link", { url: update?.page ?? RELEASES }).catch((err) => toast(String(err)));
      return;
    }
    installing = true;
    renderButton();
    $("app-update-go").disabled = true;
    $("app-update-later").disabled = true;
    $("app-update-skip-row").hidden = true;
    $("app-update-progress").hidden = false;
    $("app-update-fill").style.width = "0%";
    $("app-update-status").textContent = t("update.downloading");
    const channel = new Channel();
    channel.onmessage = (p) => {
      const pct = p.total ? Math.floor((p.done * 100) / p.total) : 0;
      $("app-update-fill").style.width = `${pct}%`;
      $("app-update-status").textContent = t("update.downloadingProgress", { done: fmtBytes(p.done), total: fmtBytes(p.total) });
    };
    try {
      await invoke("install_update", { onEvent: channel });
      $("app-update-fill").style.width = "100%";
      $("app-update-status").textContent = t("update.restarting");
    } catch (err) {
      installing = false;
      failed = true;
      renderButton();
      $("app-update-progress").hidden = true;
      setNote(`${err}`, true);
      $("app-update-go-label").textContent = t("update.openPage");
      $("app-update-go").disabled = false;
      $("app-update-later").disabled = false;
    }
  }

  $("btn-update").addEventListener("click", open);
  $("app-update-later").addEventListener("click", close);
  $("app-update-go").addEventListener("click", install);
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && !$("app-update").hidden) close();
  });

  /** Look once a little after start, then every few hours. */
  function start() {
    const auto = () => ctx.state?.updateCheck !== false && check();
    setTimeout(auto, 4000);
    setInterval(auto, RECHECK_MS);
  }

  return { check, start, render: renderButton };
}
