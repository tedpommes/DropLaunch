// The server console's command line: suggestions from the server's own command list (read from
// the installed server, so they match its build and mods) and a history kept between sessions
// (console-history.txt in DropLaunch's app data).
//
// Keys: Tab takes the highlighted (or first) suggestion, ↑/↓ move through the suggestions while
// they show and through the history otherwise, Enter sends (or takes a suggestion picked with
// the arrows), Esc closes the list.
import { el, icon, invoke } from "./util.js";
import { t } from "./i18n.js";

const MAX_SUGGESTIONS = 8;
const MAX_HISTORY_MATCHES = 3;

/**
 * @param {object} o
 * @param {HTMLFormElement} o.form
 * @param {HTMLInputElement} o.input
 * @param {(text: string) => void} o.send
 */
export function initConsoleInput({ form, input, send }) {
  /** The server's commands: { name, help, source }. */
  let commands = [];
  let commandsKey = null;
  /** Newest first. */
  let history = [];
  let historyIndex = -1;
  let draft = "";
  /** Going through the history: no suggestions until something's typed. */
  let browsing = false;
  let items = [];
  /** The highlighted suggestion (-1: none picked with the arrows yet). */
  let active = -1;

  // On top of everything (fixed), so no card around the console cuts it off.
  const list = el("ul", { class: "cc-list", id: "console-suggest", role: "listbox" });
  list.hidden = true;
  document.body.append(list);
  input.setAttribute("role", "combobox");
  input.setAttribute("aria-autocomplete", "list");
  input.setAttribute("aria-controls", "console-suggest");
  input.setAttribute("aria-expanded", "false");

  invoke("console_history")
    .then((lines) => (history = lines.slice().reverse()))
    .catch(() => {});

  /** The server's commands, read again when the server (or its branch) changed. */
  async function load(key) {
    if (key === commandsKey) return;
    commandsKey = key;
    try {
      commands = await invoke("console_commands");
    } catch {
      commands = [];
      commandsKey = null;
    }
    if (document.activeElement === input) render();
  }

  function suggestions(text) {
    const typed = text.trimStart();
    const q = typed.toLowerCase();
    if (!q || browsing) return [];
    const space = q.indexOf(" ");
    if (space >= 0) {
      // Typing the arguments: what the command does (and takes).
      const name = q.slice(0, space);
      const known = commands.find((c) => c.name === name);
      return known?.help ? [{ kind: "hint", ...known }] : [];
    }
    const out = [];
    for (const h of history) {
      if (out.length >= MAX_HISTORY_MATCHES) break;
      const lower = h.toLowerCase();
      if (lower.startsWith(q) && lower !== q && !out.some((o) => o.name === h)) out.push({ kind: "history", name: h });
    }
    // The name starts with it, then its last part does ("seed" -> server.seed), then it's in it.
    const groups = [[], [], []];
    for (const c of commands) {
      const name = c.name.toLowerCase();
      if (name.startsWith(q)) groups[0].push(c);
      else if (name.slice(name.lastIndexOf(".") + 1).startsWith(q)) groups[1].push(c);
      else if (q.length > 1 && name.includes(q)) groups[2].push(c);
    }
    for (const group of groups) group.sort((a, b) => a.name.length - b.name.length || a.name.localeCompare(b.name));
    for (const c of groups.flat()) {
      if (out.length >= MAX_SUGGESTIONS) break;
      out.push({ kind: "command", ...c });
    }
    return out;
  }

  const pickable = () => items.filter((i) => i.kind !== "hint").length;

  function render() {
    items = document.activeElement === input ? suggestions(input.value) : [];
    if (!items.length) return close();
    if (active >= pickable()) active = -1;
    const q = input.value.trimStart().toLowerCase();
    list.replaceChildren(...items.map((item, index) => row(item, index, q)));
    list.hidden = false;
    input.setAttribute("aria-expanded", "true");
    place();
  }

  function row(item, index, q) {
    if (item.kind === "hint") {
      return el("li", { class: "cc-hint" }, el("code", { text: item.name }), el("span", { text: item.help }));
    }
    return el(
      "li",
      {
        role: "option",
        class: `cc-item${index === active ? " active" : ""}`,
        "aria-selected": String(index === active),
        title: item.help || null,
        // Before the input loses focus.
        onmousedown: (e) => {
          e.preventDefault();
          accept(index);
        },
      },
      el("span", { class: "cc-icon" }, item.kind === "history" ? icon("history") : null),
      el("code", {}, ...marked(item.name, q)),
      item.help ? el("span", { class: "cc-help", text: item.help }) : null,
      item.source && item.source !== "rust" ? el("span", { class: "cc-src", text: item.source === "carbon" ? t("framework.carbon") : t("framework.oxide") }) : null,
    );
  }

  /** The typed part in bold. */
  function marked(name, q) {
    const at = q ? name.toLowerCase().indexOf(q) : -1;
    if (at < 0) return [name];
    return [name.slice(0, at), el("b", { text: name.slice(at, at + q.length) }), name.slice(at + q.length)];
  }

  /** Right above the command line, as wide as it. */
  function place() {
    const box = form.getBoundingClientRect();
    list.style.left = `${box.left}px`;
    list.style.width = `${box.width}px`;
    list.style.bottom = `${window.innerHeight - box.top + 6}px`;
    list.style.maxHeight = `${Math.max(120, Math.min(300, box.top - 16))}px`;
  }

  function close() {
    items = [];
    list.hidden = true;
    input.setAttribute("aria-expanded", "false");
  }

  function accept(index) {
    const item = items[index];
    if (!item || item.kind === "hint") return;
    input.value = item.kind === "history" ? item.name : `${item.name} `;
    active = -1;
    input.focus();
    input.setSelectionRange(input.value.length, input.value.length);
    render();
  }

  function moveHistory(step) {
    if (!history.length) return;
    if (historyIndex === -1) draft = input.value;
    historyIndex = Math.max(-1, Math.min(history.length - 1, historyIndex + step));
    input.value = historyIndex >= 0 ? history[historyIndex] : draft;
    browsing = historyIndex >= 0;
    close();
  }

  input.addEventListener("keydown", (e) => {
    const open = !list.hidden && pickable() > 0;
    if (e.key === "ArrowUp" || e.key === "ArrowDown") {
      e.preventDefault();
      if (open && !browsing) {
        const n = pickable();
        active = e.key === "ArrowDown" ? (active + 1) % n : active <= 0 ? n - 1 : active - 1;
        render();
        list.querySelector(".active")?.scrollIntoView({ block: "nearest" });
      } else {
        moveHistory(e.key === "ArrowUp" ? 1 : -1);
      }
    } else if (e.key === "Tab" && open) {
      e.preventDefault();
      accept(Math.max(0, active));
    } else if (e.key === "Enter" && open && active >= 0) {
      e.preventDefault();
      accept(active);
    } else if (e.key === "Escape" && !list.hidden) {
      e.preventDefault();
      e.stopPropagation();
      close();
    }
  });
  input.addEventListener("input", () => {
    historyIndex = -1;
    browsing = false;
    active = -1;
    render();
  });
  input.addEventListener("focus", render);
  input.addEventListener("blur", close);
  window.addEventListener("resize", () => !list.hidden && place());

  form.addEventListener("submit", (e) => {
    e.preventDefault();
    const text = input.value.trim();
    if (!text) return;
    history = [text, ...history.filter((h) => h !== text)].slice(0, 200);
    historyIndex = -1;
    browsing = false;
    input.value = "";
    close();
    invoke("console_history_add", { text }).catch(() => {});
    send(text);
  });

  return { load, close };
}
