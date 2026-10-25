// A dropdown that matches the app. The native <select> stays (hidden) as the source of truth,
// so other code keeps using select.value, select.disabled and "change" events. The list is a
// fixed-position popup aligned to the button's edges, so it lines up with the focus outline
// and can't be clipped by a scrolling container (like the setup window).
import { el, icon } from "./util.js";

let openDropdown = null;

export class Dropdown {
  /**
   * @param {HTMLSelectElement} select
   * @param {{ label?: string, meta?: (value: string) => string, title?: (value: string) => string }} options
   */
  constructor(select, { label, meta, title } = {}) {
    this.select = select;
    this.meta = meta;
    this.titleFor = title;
    this.items = [];
    this.active = -1;
    this.typed = "";

    select.classList.add("dd-native");
    select.tabIndex = -1;
    select.setAttribute("aria-hidden", "true");
    this.valueEl = el("span", { class: "dd-value" });
    this.button = el(
      "button",
      { type: "button", class: "dd-btn", "aria-haspopup": "listbox", "aria-expanded": "false" },
      label ? el("span", { class: "dd-label", text: label }) : null,
      this.valueEl,
      icon("chevron", "dd-chevron"),
    );
    this.root = el("div", { class: `dd ${select.dataset.size === "sm" ? "dd-sm" : ""}`.trim() }, this.button);
    select.after(this.root);
    this.list = el("ul", { class: "dd-list", role: "listbox", tabindex: "-1" });
    this.list.hidden = true;
    document.body.append(this.list);

    this.button.addEventListener("click", () => (this.isOpen() ? this.close() : this.open()));
    this.button.addEventListener("keydown", (e) => {
      if (["ArrowDown", "ArrowUp", "Enter", " "].includes(e.key)) {
        e.preventDefault();
        this.open();
      }
    });
    this.list.addEventListener("keydown", (e) => this.onKey(e));
    this.list.addEventListener("pointermove", (e) => {
      const li = e.target.closest("li[data-index]");
      if (li) this.setActive(Number(li.dataset.index), false);
    });
    this.list.addEventListener("click", (e) => {
      const li = e.target.closest("li[data-index]");
      if (li) this.choose(this.items[Number(li.dataset.index)].value);
    });
    select.addEventListener("change", () => this.sync());
    new MutationObserver(() => this.sync()).observe(select, { childList: true, subtree: true, attributes: true, attributeFilter: ["disabled"] });
    this.sync();
  }

  isOpen() {
    return !this.list.hidden;
  }

  sync() {
    const option = this.select.selectedOptions[0];
    this.valueEl.textContent = option ? option.textContent : "";
    this.button.disabled = this.select.disabled;
    this.button.title = option && this.titleFor ? this.titleFor(option.value) : "";
    if (this.select.disabled && this.isOpen()) this.close();
  }

  build() {
    this.items = [...this.select.options].map((o) => ({ value: o.value, text: o.textContent }));
    this.list.replaceChildren(
      ...this.items.map((item, index) => {
        const selected = item.value === this.select.value;
        const meta = this.meta?.(item.value);
        return el(
          "li",
          { role: "option", "aria-selected": String(selected), dataset: { index: String(index) }, class: selected ? "selected" : "" },
          icon("check", "dd-check"),
          el("span", { class: "dd-name", text: item.text }),
          meta ? el("span", { class: "dd-meta", text: meta }) : null,
        );
      }),
    );
  }

  open() {
    if (this.select.disabled || this.isOpen()) return;
    openDropdown?.close();
    openDropdown = this;
    this.build();
    this.list.hidden = false;
    this.root.classList.add("open");
    this.button.setAttribute("aria-expanded", "true");
    this.position();
    this.setActive(Math.max(0, this.items.findIndex((i) => i.value === this.select.value)));
    this.list.focus({ preventScroll: true });
  }

  position() {
    const rect = this.button.getBoundingClientRect();
    const list = this.list;
    list.style.minWidth = `${rect.width}px`;
    list.style.left = `${rect.left}px`;
    list.style.top = "0px";
    const height = list.offsetHeight;
    const below = window.innerHeight - rect.bottom - 8;
    const top = below >= height || rect.top < height ? rect.bottom + 6 : rect.top - 6 - height;
    list.style.top = `${Math.max(8, top)}px`;
    // Keep it on screen horizontally too.
    const overflow = rect.left + list.offsetWidth - (window.innerWidth - 8);
    if (overflow > 0) list.style.left = `${Math.max(8, rect.left - overflow)}px`;
    list.classList.toggle("above", top < rect.top);
  }

  close() {
    if (!this.isOpen()) return;
    this.list.hidden = true;
    this.root.classList.remove("open");
    this.button.setAttribute("aria-expanded", "false");
    if (openDropdown === this) openDropdown = null;
  }

  setActive(index, scroll = true) {
    this.active = index;
    [...this.list.children].forEach((li, i) => li.classList.toggle("active", i === index));
    if (scroll) this.list.children[index]?.scrollIntoView({ block: "nearest" });
  }

  choose(value) {
    this.close();
    this.button.focus();
    if (this.select.value !== value) {
      this.select.value = value;
      this.select.dispatchEvent(new Event("change", { bubbles: true }));
    }
    this.sync();
  }

  onKey(e) {
    const last = this.items.length - 1;
    if (e.key === "ArrowDown") this.setActive(Math.min(last, this.active + 1));
    else if (e.key === "ArrowUp") this.setActive(Math.max(0, this.active - 1));
    else if (e.key === "Home") this.setActive(0);
    else if (e.key === "End") this.setActive(last);
    else if (e.key === "Enter" || e.key === " ") {
      if (this.items[this.active]) this.choose(this.items[this.active].value);
    } else if (e.key === "Escape") {
      e.stopPropagation();
      this.close();
      this.button.focus();
    } else if (e.key === "Tab") {
      this.close();
    } else if (e.key.length === 1) {
      // Type to jump.
      clearTimeout(this.typedTimer);
      this.typed += e.key.toLowerCase();
      this.typedTimer = setTimeout(() => (this.typed = ""), 600);
      const hit = this.items.findIndex((i) => i.text.toLowerCase().startsWith(this.typed));
      if (hit >= 0) this.setActive(hit);
    } else return;
    e.preventDefault();
  }
}

// One place closes whatever dropdown is open: clicks elsewhere, resizing, scrolling.
document.addEventListener("pointerdown", (e) => {
  if (openDropdown && !openDropdown.root.contains(e.target) && !openDropdown.list.contains(e.target)) openDropdown.close();
});
window.addEventListener("resize", () => openDropdown?.close());
window.addEventListener("blur", () => openDropdown?.close());
document.addEventListener(
  "scroll",
  (e) => {
    if (openDropdown && !openDropdown.list.contains(e.target)) openDropdown.close();
  },
  true,
);
