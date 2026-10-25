// A zoomable picture of a Rust map (drawn from its .map file by the backend) with the in-game
// grid, zoom (wheel, buttons) and drag, and layers of things placed by world position (report
// pins, testers). The playtest page and the map viewer use it.
import { el, icon } from "./util.js";
import { t } from "./i18n.js";

const ZOOM_MAX = 8;
/** Rust's grid squares, as the playtest plugin measured them: 146 m from the map's top left. */
const CELL = 146;

/** Rust's grid letters: A..Z, then AA, AB, ... */
export function column(i) {
  let name = "";
  for (let n = i; n >= 0; n = Math.floor(n / 26) - 1) name = String.fromCharCode(65 + (n % 26)) + name;
  return name;
}

/** A picture as the backend sends it (16-byte "DLM1" header, then RGBA): { canvas, pixels, size }. */
export function readPicture(buffer) {
  const data = new DataView(buffer);
  if (String.fromCharCode(data.getUint8(0), data.getUint8(1), data.getUint8(2), data.getUint8(3)) !== "DLM1") throw new Error(t("map.notPicture"));
  const pixels = data.getUint32(4, true);
  const size = data.getUint32(8, true);
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = pixels;
  canvas.getContext("2d").putImageData(new ImageData(new Uint8ClampedArray(buffer, 16, pixels * pixels * 4), pixels, pixels), 0, 0);
  return { canvas, pixels, size };
}

export class MapCanvas {
  /**
   * Fills `wrap` with a square map. `onDraw(g)`: draws extras over the map and grid; `onBlankClick()`:
   * a click on the map itself (not a drag); `onContextMenu(event, x, z)`: a right click (world spot).
   */
  constructor(wrap, { onDraw, onBlankClick, onContextMenu } = {}) {
    this.wrap = wrap;
    this.onDraw = onDraw;
    this.picture = null;
    this.gridInfo = null;
    this.showGrid = true;
    /** Monuments as the server names them: [{ name, x, z }], shown as labels. */
    this.monuments = [];
    this.showNames = true;
    this.fallbackSize = 4000;
    this.zoom = 1;
    this.ox = 0;
    this.oy = 0;
    this.side = 0;
    this.layers = [];
    this.canvas = el("canvas");
    this.empty = el("div", { class: "pt-map-empty", hidden: true });
    const button = (title, name, run) => el("button", { class: "icon-btn", title, onclick: run }, icon(name));
    this.controls = el(
      "div",
      { class: "pt-zoom" },
      button(t("map.zoomIn"), "plus", () => this.zoomAt(1.5, this.side / 2, this.side / 2)),
      button(t("map.zoomOut"), "minus", () => this.zoomAt(1 / 1.5, this.side / 2, this.side / 2)),
      button(t("map.reset"), "expand", () => this.reset()),
    );
    this.map = el("div", { class: "pt-map" }, this.canvas, this.empty, this.controls);
    wrap.append(this.map);
    new ResizeObserver(() => this.fit()).observe(wrap);

    const blocked = (e) => e.target.closest(".pt-pin, .pt-player, .pt-zoom");
    this.map.addEventListener(
      "wheel",
      (e) => {
        e.preventDefault();
        const rect = this.map.getBoundingClientRect();
        this.zoomAt(e.deltaY < 0 ? 1.25 : 0.8, e.clientX - rect.left, e.clientY - rect.top);
      },
      { passive: false },
    );
    let drag = null;
    this.map.addEventListener("pointerdown", (e) => {
      if (e.button !== 0 || blocked(e)) return;
      drag = { x: e.clientX, y: e.clientY, ox: this.ox, oy: this.oy, moved: false };
      this.map.setPointerCapture(e.pointerId);
    });
    this.map.addEventListener("pointermove", (e) => {
      if (!drag) return;
      const dx = e.clientX - drag.x;
      const dy = e.clientY - drag.y;
      if (Math.abs(dx) + Math.abs(dy) > 3) drag.moved = true;
      this.ox = drag.ox + dx;
      this.oy = drag.oy + dy;
      this.clamp();
      this.redraw();
    });
    this.map.addEventListener("pointerup", () => {
      if (drag && !drag.moved) onBlankClick?.();
      drag = null;
    });
    this.map.addEventListener("dblclick", (e) => {
      if (!blocked(e)) this.reset();
    });
    this.map.addEventListener("contextmenu", (e) => {
      if (!onContextMenu || blocked(e)) return;
      e.preventDefault();
      const rect = this.map.getBoundingClientRect();
      const [x, z] = this.toWorld(e.clientX - rect.left, e.clientY - rect.top);
      onContextMenu(e, x, z);
    });
  }

  /** A layer for things placed by world position (children with data-x / data-z). */
  addLayer() {
    const layer = el("div", { class: "pt-layer" });
    this.map.insertBefore(layer, this.empty);
    this.layers.push(layer);
    return layer;
  }

  setPicture(picture) {
    this.picture = picture;
    this.redraw();
  }

  /** A note over the map ("Drawing the map…"); empty hides it. */
  setMessage(text) {
    this.empty.textContent = text ?? "";
    this.empty.hidden = !text;
  }

  worldSize() {
    return this.picture?.size || this.fallbackSize || 4000;
  }

  /** Where Rust's grid lies: as the plugin measured it on the server, else Rust's usual squares. */
  grid() {
    const size = this.worldSize();
    const g = this.gridInfo;
    if (g?.cell > 1) {
      return { cell: g.cell, cellZ: g.cellZ || g.cell, x0: g.x0, z0: g.z0, firstRow: Number.parseInt(String(g.first).replace(/^[A-Z]+/, ""), 10) || 0 };
    }
    return { cell: CELL, cellZ: CELL, x0: -size / 2, z0: size / 2, firstRow: 0 };
  }

  /** The grid square of a world spot ("G12"). */
  gridName(x, z) {
    const { cell, cellZ, x0, z0, firstRow } = this.grid();
    return `${column(Math.max(0, Math.floor((x - x0) / cell)))}${Math.max(0, Math.floor((z0 - z) / cellZ)) + firstRow}`;
  }

  toScreen(x, z) {
    const size = this.worldSize();
    return [(x / size + 0.5) * this.side * this.zoom + this.ox, (0.5 - z / size) * this.side * this.zoom + this.oy];
  }

  toWorld(sx, sy) {
    const size = this.worldSize();
    const full = this.side * this.zoom;
    return [((sx - this.ox) / full - 0.5) * size, (0.5 - (sy - this.oy) / full) * size];
  }

  /** The square fills the room it has. */
  fit() {
    const next = Math.floor(Math.max(120, Math.min(this.wrap.clientWidth, this.wrap.clientHeight)));
    if (!this.wrap.clientWidth || next === this.side) return;
    const scale = this.side ? next / this.side : 1;
    this.side = next;
    this.ox *= scale;
    this.oy *= scale;
    this.map.style.width = this.map.style.height = `${next}px`;
    this.redraw();
  }

  clamp() {
    const full = this.side * this.zoom;
    this.ox = Math.min(0, Math.max(this.side - full, this.ox));
    this.oy = Math.min(0, Math.max(this.side - full, this.oy));
  }

  zoomAt(factor, cx, cy) {
    const next = Math.min(ZOOM_MAX, Math.max(1, this.zoom * factor));
    this.ox = cx - ((cx - this.ox) * next) / this.zoom;
    this.oy = cy - ((cy - this.oy) * next) / this.zoom;
    this.zoom = next;
    this.clamp();
    this.redraw();
  }

  reset() {
    this.zoom = 1;
    this.ox = this.oy = 0;
    this.redraw();
  }

  /** Bring a spot into view (zoomed in a bit when the whole map shows). */
  centerOn(x, z) {
    if (this.zoom < 2) this.zoom = 2.5;
    const size = this.worldSize();
    this.ox = this.side / 2 - (x / size + 0.5) * this.side * this.zoom;
    this.oy = this.side / 2 - (0.5 - z / size) * this.side * this.zoom;
    this.clamp();
    this.redraw();
  }

  place(node) {
    const [x, y] = this.toScreen(Number(node.dataset.x), Number(node.dataset.z));
    node.style.transform = `translate(${x}px, ${y}px)`;
    node.hidden = x < -20 || y < -20 || x > this.side + 20 || y > this.side + 20;
  }

  /** After zooming or moving: everything straight to its new place (no gliding). */
  placeAll() {
    this.map.classList.add("pt-moving");
    for (const layer of this.layers) for (const node of layer.children) this.place(node);
    requestAnimationFrame(() => requestAnimationFrame(() => this.map.classList.remove("pt-moving")));
  }

  redraw() {
    this.draw();
    this.placeAll();
  }

  draw() {
    const side = this.side;
    if (!side) return;
    const dpr = window.devicePixelRatio || 1;
    this.canvas.width = Math.round(side * dpr);
    this.canvas.height = Math.round(side * dpr);
    const g = this.canvas.getContext("2d");
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    g.fillStyle = "#0e2a33";
    g.fillRect(0, 0, side, side);
    const full = side * this.zoom;
    if (this.picture) {
      g.imageSmoothingEnabled = true;
      g.imageSmoothingQuality = "high";
      g.drawImage(this.picture.canvas, this.ox, this.oy, full, full);
      if (this.showGrid) this.drawGrid(g, full);
      if (this.showNames && this.monuments.length) this.drawNames(g);
    }
    this.onDraw?.(g);
  }

  /** The monuments' names where they are, like the in-game map (outlined so they read on any ground). */
  drawNames(g) {
    this.font ??= getComputedStyle(this.map).fontFamily;
    g.font = `650 ${this.zoom >= 2 ? 12.5 : 11}px ${this.font}`;
    g.textAlign = "center";
    g.textBaseline = "middle";
    g.lineJoin = "round";
    g.lineWidth = 3.5;
    g.strokeStyle = "rgba(10, 12, 16, 0.85)";
    g.fillStyle = "#fff";
    for (const m of this.monuments) {
      const [x, y] = this.toScreen(m.x, m.z);
      if (x < -100 || y < -20 || x > this.side + 100 || y > this.side + 20) continue;
      g.strokeText(m.name, x, y);
      g.fillText(m.name, x, y);
    }
    g.textAlign = "start";
  }

  drawGrid(g, full) {
    const side = this.side;
    const size = this.worldSize();
    const { cell, cellZ, x0, z0, firstRow } = this.grid();
    const px = (cell / size) * full;
    const pz = (cellZ / size) * full;
    const left = ((x0 + size / 2) / size) * full + this.ox;
    const top = ((size / 2 - z0) / size) * full + this.oy;
    const cols = Math.ceil((size / 2 - x0) / cell);
    const rows = Math.ceil((z0 + size / 2) / cellZ);
    g.strokeStyle = "rgba(255, 255, 255, 0.13)";
    g.lineWidth = 1;
    g.beginPath();
    for (let i = 0; i <= cols; i++) {
      const x = Math.round(left + i * px) + 0.5;
      if (x < 0 || x > side) continue;
      g.moveTo(x, Math.max(0, top));
      g.lineTo(x, Math.min(side, top + rows * pz));
    }
    for (let j = 0; j <= rows; j++) {
      const y = Math.round(top + j * pz) + 0.5;
      if (y < 0 || y > side) continue;
      g.moveTo(Math.max(0, left), y);
      g.lineTo(Math.min(side, left + cols * px), y);
    }
    g.stroke();
    this.font ??= getComputedStyle(this.map).fontFamily;
    g.font = `600 ${px >= 44 ? 11 : 9.5}px ${this.font}`;
    g.textBaseline = "top";
    g.fillStyle = "rgba(255, 255, 255, 0.55)";
    if (px >= 34) {
      // Like the in-game map: every square has its name in the corner.
      for (let i = 0; i < cols; i++) {
        const x = left + i * px;
        if (x + px < 0 || x > side) continue;
        for (let j = 0; j < rows; j++) {
          const y = top + j * pz;
          if (y + pz < 0 || y > side) continue;
          g.fillText(`${column(i)}${j + firstRow}`, x + 3, y + 3);
        }
      }
    } else {
      // Small squares: letters along the top, numbers down the side (as many as fit).
      const across = Math.max(1, Math.ceil((cols > 26 ? 22 : 14) / px));
      const down = Math.max(1, Math.ceil(14 / pz));
      for (let i = 0; i < cols; i += across) g.fillText(column(i), left + i * px + 2, Math.max(2, top + 2));
      for (let j = down; j < rows; j += down) g.fillText(String(j + firstRow), Math.max(2, left + 2), top + j * pz + 2);
    }
  }
}

/** A report's pin (red open, green fixed): a button placed at the spot it was sent from. */
export function reportPin(r, { selected = false, onClick } = {}) {
  const pin = el(
    "button",
    {
      class: `pt-pin${r.fixed ? " fixed" : ""}${selected ? " sel" : ""}`,
      title: t(r.fixed ? "map.pin.titleFixed" : "map.pin.title", { n: r.n, text: r.text, name: r.name, grid: r.grid }),
      "aria-label": t("map.pin.label", { n: r.n, text: r.text }),
      onclick: (e) => {
        e.stopPropagation();
        onClick?.(r);
      },
    },
    el("span", { text: String(r.n) }),
  );
  pin.dataset.x = r.x;
  pin.dataset.z = r.z;
  return pin;
}
