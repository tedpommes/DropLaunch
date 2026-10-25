// Live graphs like a network monitor's: the last minutes as smooth filled curves that scroll by
// (newest on the right), with the values under the mouse. The server's performance uses them.
import { el } from "./util.js";
import { t } from "./i18n.js";

/** Redraws at most this often while a graph is on screen (it scrolls smoothly in between samples). */
const FRAME_MS = 50;
const graphs = new Set();

document.addEventListener("visibilitychange", () => {
  for (const g of graphs) g.wake();
});

/** "0 B/s", "512 B/s", "23 KB/s", "1.4 MB/s". */
export function fmtRate(bytes) {
  if (!bytes || bytes < 1) return "0 B/s";
  const units = ["B/s", "KB/s", "MB/s", "GB/s"];
  const i = Math.min(units.length - 1, Math.floor(Math.log(bytes) / Math.log(1024)));
  const v = bytes / 1024 ** i;
  return `${v.toFixed(i === 0 || v >= 100 ? 0 : v >= 10 ? 1 : 2).replace(/\.0+$/, "")} ${units[i]}`;
}

/** 1, 2, 2.5, 5 or 10 times a power of ten, at least `v`. */
export function niceCeil(v) {
  if (!(v > 0)) return 1;
  const exp = 10 ** Math.floor(Math.log10(v));
  const f = v / exp;
  return (f <= 1 ? 1 : f <= 2 ? 2 : f <= 2.5 ? 2.5 : f <= 5 ? 5 : 10) * exp;
}

/** The same for bytes: round numbers of B, KB or MB. */
export function niceBytes(v) {
  if (!(v > 0)) return 1024;
  const unit = 1024 ** Math.max(0, Math.floor(Math.log(v) / Math.log(1024)));
  return niceCeil(v / unit) * unit;
}

function rgba(hex, alpha) {
  const n = Number.parseInt(hex.slice(1), 16);
  return `rgba(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}, ${alpha})`;
}

/** A smooth curve through the points that never swings past them (monotone cubic). */
function curve(ctx, p) {
  const n = p.length;
  const dx = [];
  const slope = [];
  for (let i = 0; i < n - 1; i++) {
    dx[i] = p[i + 1][0] - p[i][0];
    slope[i] = dx[i] ? (p[i + 1][1] - p[i][1]) / dx[i] : 0;
  }
  const t = [slope[0]];
  for (let i = 1; i < n - 1; i++) {
    t[i] = slope[i - 1] * slope[i] <= 0 ? 0 : (3 * (dx[i - 1] + dx[i])) / ((2 * dx[i] + dx[i - 1]) / slope[i - 1] + (dx[i] + 2 * dx[i - 1]) / slope[i]);
  }
  t[n - 1] = slope[n - 2];
  ctx.moveTo(p[0][0], p[0][1]);
  for (let i = 0; i < n - 1; i++) {
    const d = dx[i] / 3;
    ctx.bezierCurveTo(p[i][0] + d, p[i][1] + t[i] * d, p[i + 1][0] - d, p[i + 1][1] - t[i + 1] * d, p[i + 1][0], p[i + 1][1]);
  }
}

function ago(ms) {
  const s = Math.round(ms / 1000);
  if (s < 2) return t("perf.ago.now");
  return s < 90 ? t("perf.ago.seconds", { n: s }) : t("perf.ago.minutes", { n: Math.round(s / 60) });
}

export class LiveGraph {
  /**
   * `views`: what it can show, by name: { series: [{ key, label, color }], format(value), nice(max),
   * floor (the scale's smallest top) }. With `separate: true` every line has a scale of its own
   * (each series then brings its own format, nice and floor), for things in different units. `span`: the time shown (ms). `every`: how often samples
   * come (ms); the curve runs that far behind, so it scrolls instead of jumping.
   */
  constructor({ views, view, span = 120000, every = 1000, height = 72, label = "" }) {
    this.views = views;
    this.viewName = view ?? Object.keys(views)[0];
    this.span = span;
    this.every = every;
    this.samples = [];
    this.extra = "";
    this.hover = null;
    this.visible = false;
    this.running = false;
    this.lastDraw = 0;
    this.canvas = el("canvas", { class: "graph-canvas", role: "img", "aria-label": label });
    // Floats over the page (fixed): a card or the window's edge never cuts it off.
    this.tip = el("div", { class: "graph-tip", hidden: true });
    this.legend = el("div", { class: "graph-legend" });
    this.plot = el("div", { class: "graph-plot", style: `height:${height}px` }, this.canvas);
    this.element = el("div", { class: "graph" }, this.plot, this.legend);
    this.plot.addEventListener("mousemove", (e) => {
      this.hover = e.clientX - this.plot.getBoundingClientRect().left;
      this.draw();
    });
    this.plot.addEventListener("mouseleave", () => {
      this.hover = null;
      this.tip.hidden = true;
      this.draw();
    });
    this.sizer = new ResizeObserver(() => this.draw());
    this.sizer.observe(this.plot);
    this.seen = new IntersectionObserver((entries) => {
      this.visible = entries.some((e) => e.isIntersecting);
      this.wake();
    });
    this.seen.observe(this.plot);
    graphs.add(this);
    this.renderLegend();
  }

  get view() {
    return this.views[this.viewName];
  }

  setView(name) {
    if (!this.views[name] || name === this.viewName) return;
    this.viewName = name;
    this.renderLegend();
    this.draw();
  }

  /** A sample: { key: value } for every series (of every view). */
  push(values, time = Date.now()) {
    this.samples.push({ t: time, v: values });
    const oldest = time - this.span - this.every * 3;
    while (this.samples.length > 2 && this.samples[1].t < oldest) this.samples.shift();
    this.renderLegend();
    this.wake();
  }

  /** A note after the values in the legend ("ping 42 ms"). */
  setExtra(text) {
    if (text === this.extra) return;
    this.extra = text;
    this.renderLegend();
  }

  clear() {
    this.samples = [];
    this.renderLegend();
    this.draw();
  }

  /** Gone for good (its connection ended). */
  destroy() {
    this.tip.remove();
    this.sizer.disconnect();
    this.seen.disconnect();
    graphs.delete(this);
    this.running = false;
  }

  renderLegend() {
    const last = this.samples.at(-1)?.v ?? {};
    this.legend.replaceChildren(
      ...this.view.series.map((s) =>
        el("span", { class: "graph-key" }, el("i", { style: `background:${s.color}` }), el("span", { text: s.label }), el("b", { text: last[s.key] == null ? "–" : this.formatOf(s)(last[s.key]) })),
      ),
      ...(this.extra ? [el("span", { class: "graph-extra", text: this.extra })] : []),
    );
  }

  formatOf(series) {
    return series.format ?? this.view.format;
  }

  /** Scroll while it's on screen; rest otherwise. */
  wake() {
    if (this.running || !this.visible || document.hidden || !this.canvas.isConnected) return;
    this.running = true;
    const frame = (now) => {
      if (!this.running || !this.visible || document.hidden || !this.canvas.isConnected) {
        this.running = false;
        this.tip.hidden = true;
        return;
      }
      if (now - this.lastDraw >= FRAME_MS) {
        this.lastDraw = now;
        this.draw();
      }
      requestAnimationFrame(frame);
    };
    requestAnimationFrame(frame);
  }

  draw() {
    const { canvas } = this;
    const w = this.plot.clientWidth;
    const h = this.plot.clientHeight;
    if (!w || !h) return;
    const dpr = window.devicePixelRatio || 1;
    if (canvas.width !== Math.round(w * dpr) || canvas.height !== Math.round(h * dpr)) {
      canvas.width = Math.round(w * dpr);
      canvas.height = Math.round(h * dpr);
    }
    const g = canvas.getContext("2d");
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    g.clearRect(0, 0, w, h);

    const view = this.view;
    const end = Date.now() - this.every;
    const start = end - this.span;
    const x = (t) => ((t - start) / this.span) * w;
    const points = this.samples.filter((s, i, all) => s.t >= start || all[i + 1]?.t >= start);
    // The top of the scale: one for all lines, or (separate) one per line.
    const scaleOf = (list) => {
      const first = list[0];
      let max = (view.separate ? first.floor : view.floor) ?? 1;
      for (const s of points) for (const series of list) max = Math.max(max, s.v[series.key] ?? 0);
      return ((view.separate ? first.nice : view.nice) ?? niceCeil)(max * 1.08);
    };
    const shared = view.separate ? null : scaleOf(view.series);
    const maxes = new Map(view.series.map((series) => [series.key, shared ?? scaleOf([series])]));
    const top = 16;
    const bottom = h - 1.5;
    const yOf = (series) => (v) => bottom - (Math.max(0, v ?? 0) / maxes.get(series.key)) * (bottom - top);

    // Faint lines at a quarter, half and three quarters of the scale.
    g.strokeStyle = "rgba(255, 255, 255, 0.06)";
    g.lineWidth = 1;
    g.beginPath();
    for (let i = 1; i <= 3; i++) {
      const ly = Math.round(top + ((bottom - top) * i) / 4) + 0.5;
      g.moveTo(0, ly);
      g.lineTo(w, ly);
    }
    g.stroke();

    for (const series of view.series) {
      const y = yOf(series);
      const p = points.map((s) => [x(s.t), y(s.v[series.key])]);
      if (p.length < 2) continue;
      // Many lines at once: lighter fills, so they don't cover each other.
      const fill = g.createLinearGradient(0, top, 0, bottom);
      fill.addColorStop(0, rgba(series.color, view.separate ? 0.14 : 0.42));
      fill.addColorStop(1, rgba(series.color, 0.02));
      g.beginPath();
      curve(g, p);
      g.lineTo(p.at(-1)[0], bottom);
      g.lineTo(p[0][0], bottom);
      g.closePath();
      g.fillStyle = fill;
      g.fill();
      g.beginPath();
      curve(g, p);
      g.strokeStyle = series.color;
      g.lineWidth = 1.6;
      g.stroke();
    }

    this.font ??= `600 10.5px ${getComputedStyle(this.plot).fontFamily}`;
    g.font = this.font;
    g.fillStyle = "rgba(255, 255, 255, 0.42)";
    g.textBaseline = "top";
    g.fillText(view.separate ? t("perf.separate") : view.format(shared), 6, 3);
    if (!points.length) {
      g.textAlign = "center";
      g.textBaseline = "middle";
      g.fillText(t("perf.waiting"), w / 2, (top + bottom) / 2);
      g.textAlign = "start";
    }

    // The values under the mouse.
    if (this.hover == null || !points.length || !this.plot.isConnected) {
      this.tip.hidden = true;
      return;
    }
    const near = points.reduce((best, s) => (Math.abs(x(s.t) - this.hover) < Math.abs(x(best.t) - this.hover) ? s : best));
    const nx = Math.round(x(near.t)) + 0.5;
    g.strokeStyle = "rgba(255, 255, 255, 0.3)";
    g.beginPath();
    g.moveTo(nx, top - 4);
    g.lineTo(nx, bottom);
    g.stroke();
    for (const series of view.series) {
      g.beginPath();
      g.arc(nx, yOf(series)(near.v[series.key]), 3, 0, Math.PI * 2);
      g.fillStyle = series.color;
      g.fill();
    }
    this.tip.replaceChildren(
      el("span", { class: "graph-tip-time", text: ago(Date.now() - near.t) }),
      ...view.series.map((s) => el("span", {}, el("i", { style: `background:${s.color}` }), el("span", { text: `${s.label} ${this.formatOf(s)(near.v[s.key] ?? 0)}` }))),
    );
    if (!this.tip.isConnected) document.body.append(this.tip);
    this.tip.hidden = false;
    // Next to the line, above the graph (below it when there's no room above).
    const rect = this.plot.getBoundingClientRect();
    const tw = this.tip.offsetWidth;
    const th = this.tip.offsetHeight;
    const lineX = rect.left + nx;
    const left = lineX + 12 + tw > innerWidth - 4 ? lineX - tw - 12 : lineX + 12;
    const above = rect.top - th - 6;
    this.tip.style.left = `${Math.max(4, Math.min(innerWidth - tw - 4, left))}px`;
    this.tip.style.top = `${above >= 4 ? above : Math.min(innerHeight - th - 4, rect.bottom + 6)}px`;
  }
}
