// About: DropLaunch's version, and the libraries it's made with, each with its notice and
// license text (their licenses ask for that to go along with the app). The list comes from
// licenses.json, made from the build by `cargo run --example notices`, and is only loaded when
// the window opens.
import { $, el, icon, toast } from "./util.js";
import { t, tn } from "./i18n.js";

export function initAbout(ctx) {
  /** { crates: [{ name, version, license, repository, authors, files: [{ name, text }] }], texts } */
  let notices = null;
  let query = "";

  async function open() {
    const version = ctx.state?.version ? t("about.version", { version: ctx.state.version }) : "";
    $("about-sub").textContent = [version, t("about.madeBy")].filter(Boolean).join(" · ");
    $("about").hidden = false;
    $("about-close").focus();
    if (!notices) {
      $("about-list").replaceChildren(el("div", { class: "about-empty", text: t("common.loading") }));
      try {
        const reply = await fetch("licenses.json");
        if (!reply.ok) throw new Error(`HTTP ${reply.status}`);
        notices = await reply.json();
      } catch (err) {
        $("about-list").replaceChildren(el("div", { class: "about-empty", text: t("about.loadFailed", { error: err.message ?? err }) }));
        return;
      }
      $("about-intro").textContent = tn("about.introCount", notices.crates.length);
    }
    render();
  }

  function close() {
    $("about").hidden = true;
  }

  function matches(c) {
    if (!query) return true;
    return c.name.toLowerCase().includes(query) || (c.license ?? "").toLowerCase().includes(query);
  }

  function render() {
    if (!notices) return;
    const shown = notices.crates.filter(matches);
    const box = $("about-list");
    box.replaceChildren(...(shown.length ? shown.map(row) : [el("div", { class: "about-empty", text: t("about.noMatch", { query }) })]));
    box.scrollTop = 0;
  }

  /** One library: name, version and license; its texts once it's opened. */
  function row(c) {
    const item = el("div", { class: "lic", role: "listitem" });
    const head = el(
      "button",
      { type: "button", class: "lic-head", "aria-expanded": "false", title: c.repository ?? "" },
      icon("chevron"),
      el("span", { class: "lic-name", text: c.name }),
      el("span", { class: "lic-version", text: c.version }),
      el("span", { class: "lic-license", text: c.license, title: c.license }),
    );
    head.addEventListener("click", () => {
      const opening = !item.classList.contains("open");
      item.classList.toggle("open", opening);
      head.setAttribute("aria-expanded", String(opening));
      if (opening && !item.querySelector(".lic-body")) item.append(body(c));
      item.querySelector(".lic-body").hidden = !opening;
    });
    item.append(head);
    return item;
  }

  function body(c) {
    const meta = [c.authors?.length ? t("about.by", { authors: c.authors.join(", ") }) : null, t("about.license", { license: c.license })].filter(Boolean).join(" · ");
    return el(
      "div",
      { class: "lic-body" },
      el("p", { class: "lic-meta", text: meta }),
      c.repository?.startsWith("https://") ? el("a", { class: "ext-link lic-meta", href: c.repository, text: c.repository }) : null,
      ...c.files.flatMap((f) => [el("div", { class: "lic-file", text: f.name }), el("pre", { class: "lic-text", text: notices.texts[f.text] ?? "" })]),
    );
  }

  $("btn-about").addEventListener("click", () => open().catch((err) => toast(String(err), "error")));
  $("footer-licenses").addEventListener("click", () => open().catch((err) => toast(String(err), "error")));
  $("about-close").addEventListener("click", close);
  $("about").addEventListener("click", (e) => e.target === $("about") && close());
  $("about-search").addEventListener("input", (e) => {
    query = e.target.value.trim().toLowerCase();
    render();
  });
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && !$("about").hidden && $("modal").hidden) close();
  });

  return { open };
}
