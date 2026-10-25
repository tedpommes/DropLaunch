// DropLaunch's texts: every label, message and tip comes from the language file. English is built
// in (lang/en.json); the backend hands over the language in use (a file next to DropLaunch.exe,
// or one dropped on it), with English for whatever that file leaves out.
//
//   t("key", { name })     plain text, {placeholders} filled in
//   tn("key", n, { … })    a count: key.one / key.other (or the language's own plural forms)
//   rich("key", { … })     nodes: the text may have <b>, <strong>, <em>, <code>, <a href="name">
//                          and <br> (made into elements here, never parsed as HTML; a link's
//                          address comes from LINKS below), and placeholders may be nodes
//   data-t="key" (content), data-t-title / -placeholder / -label (aria-label) in index.html:
//   filled in once at start.

const core = window.__TAURI__.core;

/** Links texts can point at: <a href="rustedit">…</a>. */
const LINKS = {
  rustedit: "https://github.com/k1lly0u/Oxide.Ext.RustEdit",
  steamcmd: "https://developer.valvesoftware.com/wiki/SteamCMD",
  carbon: "https://github.com/CarbonCommunity/Carbon",
  oxide: "https://github.com/OxideMod/Oxide.Rust",
};

/** { code, name, strings, languages, folder } from the backend; English from the file without it. */
const state = await core.invoke("language_state").catch(async () => {
  const file = await (await fetch("lang/en.json")).json();
  return { code: "en", name: "English", strings: file.strings, languages: [], folder: null };
});
const strings = state.strings;

export const language = {
  code: state.code,
  name: state.name,
  languages: state.languages,
  folder: state.folder,
  /** Dates in English follow Windows' own format ("11 Oct" or "Oct 11"); other languages their own. */
  dateLocale: state.code === "en" ? undefined : state.code,
};
document.documentElement.lang = state.code;

function safely(make, fallback) {
  try {
    return make(state.code);
  } catch {
    return make(fallback);
  }
}
const pluralRules = safely((code) => new Intl.PluralRules(code), "en");
const lists = safely((code) => new Intl.ListFormat(code, { type: "conjunction" }), "en");

function lookup(key) {
  const text = strings[key];
  if (text != null) return text;
  console.warn(`DropLaunch: no text for "${key}"`);
  return key;
}

const TAG = /<(\/?)(b|strong|em|code|a|br)(?:\s+href="([a-z0-9-]*)")?\s*\/?>/g;
const PLACEHOLDER = /\{(\w+)\}/g;

function fill(text, vars) {
  return vars ? text.replace(PLACEHOLDER, (all, name) => (vars[name] != null ? String(vars[name]) : all)) : text;
}

/** Plain text (tags dropped, <br> as a line break). */
export function t(key, vars) {
  return fill(lookup(key).replace(TAG, (_, __, tag) => (tag === "br" ? "\n" : "")), vars);
}

/** A count: "1 map" / "3 maps". `n` is also {n}. */
export function tn(key, n, vars = {}) {
  const form = `${key}.${pluralRules.select(n)}`;
  const text = strings[form] ?? strings[`${key}.other`] ?? lookup(form);
  return fill(text.replace(TAG, ""), { n: fmtNum(n), ...vars });
}

/** rich() for a count: "3 maps in <path>". */
export function richn(key, n, vars = {}) {
  const form = `${key}.${pluralRules.select(n)}`;
  return richText(strings[form] ?? strings[`${key}.other`] ?? lookup(form), { n: fmtNum(n), ...vars });
}

/** Nodes for a text with tags (and placeholders that may be nodes, like a coloured size). */
export function rich(key, vars = {}) {
  return richText(lookup(key), vars);
}

function richText(text, vars) {
  const root = document.createDocumentFragment();
  const stack = [root];
  let last = 0;
  for (const m of text.matchAll(TAG)) {
    addText(stack.at(-1), text.slice(last, m.index), vars);
    last = m.index + m[0].length;
    const [, close, tag, href] = m;
    if (tag === "br") {
      stack.at(-1).append(document.createElement("br"));
    } else if (close) {
      if (stack.length > 1 && stack.at(-1).dataset.tag === tag) stack.pop();
    } else {
      const url = tag === "a" ? LINKS[href] : null;
      const node = document.createElement(tag === "a" ? (url ? "a" : "span") : tag);
      node.dataset.tag = tag;
      if (url) {
        node.className = "ext-link";
        node.href = url;
        node.title = url.replace(/^https:\/\//, "");
      }
      stack.at(-1).append(node);
      stack.push(node);
    }
  }
  addText(stack.at(-1), text.slice(last), vars);
  for (const node of root.querySelectorAll("[data-tag]")) delete node.dataset.tag;
  return [...root.childNodes];
}

function addText(parent, text, vars) {
  let last = 0;
  for (const m of text.matchAll(PLACEHOLDER)) {
    parent.append(text.slice(last, m.index));
    const value = vars[m[1]];
    parent.append(value == null ? m[0] : value instanceof Node ? value : String(value));
    last = m.index + m[0].length;
  }
  parent.append(text.slice(last));
}

/** A number in the language's way ("6.5", "6,5"). */
export function fmtNum(value, digits = 0) {
  return value.toLocaleString(state.code, { minimumFractionDigits: digits, maximumFractionDigits: digits });
}

/** "SteamCMD and the server files". */
export const fmtList = (items) => lists.format(items);

/** The texts in index.html (data-t…), or in a part of it. */
export function applyTexts(root = document) {
  for (const node of root.querySelectorAll("[data-t]")) node.replaceChildren(...rich(node.dataset.t));
  const attrs = { tTitle: "title", tPlaceholder: "placeholder", tLabel: "aria-label" };
  for (const [data, attr] of Object.entries(attrs)) {
    const name = `data-${data.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)}`;
    for (const node of root.querySelectorAll(`[${name}]`)) node.setAttribute(attr, t(node.dataset[data]));
  }
}

applyTexts();
