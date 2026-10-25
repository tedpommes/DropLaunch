// Rent a server (a tab in Settings): hosting companies to rent a Rust server from, so friends can
// test a map on a server that's always online. DropLaunch's affiliate links (the same as v1's),
// with BisectHosting first; they open in the browser.
import { el, icon, invoke, toast } from "./util.js";
import { t } from "./i18n.js";

/** `logo` in ui/hosts/; `h`: the logo's height in the grid (they're drawn at different sizes). */
const FEATURED = {
  name: "BisectHosting",
  domain: "bisecthosting.com",
  url: "https://www.bisecthosting.com/clients/aff.php?aff=6921",
  logo: "bisect.png",
  brand: "#2fc8f5",
};

const HOSTS = [
  { name: "GameServerKings", domain: "gameserverkings.com", url: "https://www.gameserverkings.com/games/rust-server-hosting/?ref=3749", logo: "gameserverkings.png", h: 30, brand: "#e8c15a" },
  { name: "GameServers.com", domain: "gameservers.com", url: "https://www.gameservers.com/?ref=1667739", logo: "gameservers.png", h: 22, brand: "#f5a623" },
  { name: "GTX Gaming", domain: "gtxgaming.co.uk", url: "https://www.gtxgaming.co.uk/clientarea/aff.php?aff=4072", logo: "gtx.png", h: 40, brand: "#f7931e" },
  { name: "Host Havoc", domain: "hosthavoc.com", url: "https://hosthavoc.com/billing/aff.php?aff=2336&page=rust", logo: "hosthavoc.png", h: 28, brand: "#1d8cf8" },
  // The logo is black: drawn light on the dark card.
  { name: "Hostinger", domain: "hostinger.com", url: "https://hostinger.com?REFERRALCODE=SHATEDPOMEJA", logo: "hostinger.png", h: 26, brand: "#8c6cff", invert: true },
  { name: "Shockbyte", domain: "shockbyte.com", url: "https://shockbyte.com/billing/aff.php?aff=10803", logo: "shockbyte.png", h: 30, brand: "#3aa0ff" },
  { name: "ZAP-Hosting", domain: "zap-hosting.com", url: "https://zap-hosting.com/DropLaunch", logo: "zap.png", h: 52, brand: "#5fc24a" },
];

async function visit(host) {
  try {
    await invoke("open_link", { url: host.url });
    toast(t("rent.opens", { name: host.name }));
  } catch (err) {
    toast(String(err), "error");
  }
}

function logo(host, h) {
  return el("img", { src: `hosts/${host.logo}`, alt: host.name, class: host.invert ? "invert" : null, style: h ? `--h: ${h}px` : null, draggable: "false" });
}

function featured(host) {
  return el(
    "button",
    { type: "button", class: "rent-featured", style: `--brand: ${host.brand}`, title: t("rent.card.title", { domain: host.domain }), "aria-label": t("rent.card.label", { name: host.name }), onclick: () => visit(host) },
    el(
      "span",
      { class: "rent-featured-main" },
      el("span", { class: "rent-badge" }, icon("star"), el("span", { text: t("rent.featured") })),
      logo(host),
      el("span", { class: "rent-domain" }, el("span", { text: host.domain }), icon("external")),
    ),
    el("span", { class: "rent-cta" }, el("span", { text: t("rent.cta") }), icon("arrow-right")),
  );
}

function card(host) {
  return el(
    "button",
    { type: "button", class: "rent-card", style: `--brand: ${host.brand}`, title: t("rent.card.title", { domain: host.domain }), "aria-label": t("rent.card.label", { name: host.name }), onclick: () => visit(host) },
    el("span", { class: "rent-logo" }, logo(host, host.h)),
    el("span", { class: "rent-domain" }, el("span", { text: host.domain }), icon("external")),
  );
}

/** The hosts (BisectHosting big, then the others in a grid) and the affiliate note under them. */
export function rentPage() {
  return [
    el("div", { class: "rent-body" }, featured(FEATURED), el("div", { class: "rent-grid" }, HOSTS.map(card))),
    el("p", { class: "rent-foot" }, icon("info"), el("span", { text: t("rent.disclosure") })),
  ];
}
