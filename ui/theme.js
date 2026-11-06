// DropLaunch's colours (Settings › Visuals): the theme in use goes on the page as CSS variables
// over styles.css's own (Default's), each with an -rgb triplet for see-through shades of it
// (rgb(var(--accent-rgb) / 0.2)). They're kept in localStorage too, so index.html can put them on
// before anything shows at the next start. Canvases ask color() and redraw on "themechange".

const core = window.__TAURI__.core;
const KEY = "droplaunch.theme";

/** { id, themes: [{ id, name, mode, source, file, colors }], folder } from the backend. */
const state = await core.invoke("theme_state").catch(() => ({ id: "default", themes: [], folder: null }));

const triplet = (hex) => {
  const n = Number.parseInt(hex.slice(1, 7), 16);
  return `${(n >> 16) & 255} ${(n >> 8) & 255} ${n & 255}`;
};

let colors = new Map();

/** A theme colour as the page has it now: color("--accent") -> "#7a95e3". */
export function color(name) {
  if (!colors.has(name)) colors.set(name, getComputedStyle(document.documentElement).getPropertyValue(name).trim());
  return colors.get(name);
}

function apply(theme) {
  const vars = {};
  for (const [key, value] of Object.entries(theme.colors)) {
    vars[`--${key}`] = value;
    vars[`--${key}-rgb`] = triplet(value);
  }
  const root = document.documentElement;
  for (const [name, value] of Object.entries(vars)) root.style.setProperty(name, value);
  root.dataset.mode = theme.mode;
  colors = new Map();
  try {
    localStorage.setItem(KEY, JSON.stringify(vars));
  } catch {
    /* no storage: the next start shows Default for a moment */
  }
  window.dispatchEvent(new CustomEvent("themechange", { detail: theme }));
}

export const themes = {
  get id() {
    return state.id;
  },
  get list() {
    return state.themes;
  },
  get folder() {
    return state.folder;
  },
  get current() {
    return state.themes.find((t) => t.id === state.id) ?? null;
  },
  /** Use another theme (the backend keeps it and dresses the window's frame). */
  async use(id) {
    await core.invoke("set_theme", { id });
    state.id = id;
    if (this.current) apply(this.current);
  },
  /** The list again (themes were added or removed); the one in use put on again. */
  async reload() {
    Object.assign(state, await core.invoke("theme_state"));
    if (this.current) apply(this.current);
  },
};

if (themes.current) apply(themes.current);
