// Right-click menus. The WebView's own menu (Back, Print, Share...) does nothing useful here, so
// it's replaced everywhere except in text fields.
import { el, icon, copyText } from "./util.js";
import { t } from "./i18n.js";

let open = null;

export function closeMenu() {
  open?.remove();
  open = null;
}

/** Show a menu at (x, y). Items: { label, icon, kind, disabled, run } or "sep"; falsy ones are skipped. */
export function contextMenu(x, y, items) {
  closeMenu();
  const list = items.filter(Boolean).filter((item, i, all) => item !== "sep" || (i > 0 && i < all.length - 1 && all[i - 1] !== "sep"));
  const menu = el(
    "ul",
    { class: "ctx-menu", role: "menu" },
    ...list.map((item) =>
      item === "sep"
        ? el("li", { class: "sep", role: "separator" })
        : el(
            "li",
            {
              role: "menuitem",
              class: [item.kind, item.disabled ? "disabled" : ""].filter(Boolean).join(" "),
              "aria-disabled": item.disabled ? "true" : null,
              onclick: () => {
                if (item.disabled) return;
                closeMenu();
                item.run();
              },
            },
            item.icon ? icon(item.icon) : null,
            el("span", { text: item.label }),
          ),
    ),
  );
  document.body.append(menu);
  // Keep it inside the window.
  const box = menu.getBoundingClientRect();
  menu.style.left = `${Math.max(8, Math.min(x, innerWidth - box.width - 8))}px`;
  menu.style.top = `${y + box.height > innerHeight - 8 ? Math.max(8, y - box.height) : y}px`;
  open = menu;
}

document.addEventListener("mousedown", (e) => {
  if (open && !open.contains(e.target)) closeMenu();
});
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") closeMenu();
});
window.addEventListener("blur", closeMenu);
window.addEventListener("resize", closeMenu);
document.addEventListener("scroll", closeMenu, true);

document.addEventListener("contextmenu", (e) => {
  if (e.defaultPrevented || e.target.closest("input, textarea, [contenteditable]")) return;
  e.preventDefault();
  // Selected text (console, paths) can still be copied.
  const selected = String(window.getSelection() ?? "").trim();
  if (selected) contextMenu(e.clientX, e.clientY, [{ label: t("common.copy"), icon: "copy", run: () => copyText(selected, t("common.selection")) }]);
  else closeMenu();
});
