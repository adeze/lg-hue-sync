const DOMAIN = "lg_hue_sync";
const CONTROLS = [
  ["switch", "screen_sync", "Screen sync"],
  ["switch", "display", "Display"],
  ["select", "color_preset", "Color preset"],
  ["select", "response", "Response"],
  ["select", "spatial_blending", "Spatial blending"],
];
const STATUS = [
  ["sensor", "pipeline", "Pipeline"],
  ["binary_sensor", "hue_connection", "Hue connection"],
  ["binary_sensor", "nanoleaf_connection", "Nanoleaf connection"],
];
const MAINTENANCE = [["button", "restart_pipeline", "Restart pipeline"]];

function devices(hass) {
  return [...new Set(Object.values(hass.entities ?? {})
    .filter((entry) => entry.platform === DOMAIN && entry.device_id)
    .map((entry) => entry.device_id))];
}

function cardConfig(config, hass) {
  const matches = devices(hass);
  const device = config.device_id ?? (matches.length === 1 ? matches[0] : undefined);
  const entries = Object.values(hass.entities ?? {}).filter(
    (entry) => device && entry.platform === DOMAIN && entry.device_id === device,
  );
  const rows = (fields) => fields.flatMap(([domain, key, name]) => {
    const entity = config.entities?.[key] ?? entries.find(
      (entry) => entry.entity_id.startsWith(`${domain}.`) &&
        entry.entity_id.endsWith(`_${key}`) && hass.states[entry.entity_id],
    )?.entity_id;
    return entity ? [{ entity, name }] : [];
  });
  const entities = rows(CONTROLS);
  for (const [label, fields] of [["Status", STATUS], ["Maintenance", MAINTENANCE]]) {
    const section = rows(fields);
    if (label === "Status") {
      const pipeline = section.find((row) => row.name === "Pipeline")?.entity;
      const attributes = hass.states[pipeline]?.attributes;
      for (const [attribute, name, suffix] of [
        ["capture_fps", "Capture rate", " fps"],
        ["light_update_fps", "Light update rate", " fps"],
        ["capture_resolution", "Capture resolution", ""],
      ]) {
        if (attributes?.[attribute] != null) {
          section.push({ type: "attribute", entity: pipeline, attribute, name, suffix });
        }
      }
      for (const [key, name, icon] of [
        ["hue_device", "Open Hue Bridge", "mdi:bridge"],
        ["nanoleaf_device", "Open Nanoleaf controller", "mdi:led-strip-variant"],
      ]) {
        const id = attributes?.[key];
        if (id && hass.devices?.[id]) {
          section.push({ type: "weblink", name, icon, url: `/config/devices/device/${encodeURIComponent(id)}` });
        }
      }
    }
    if (section.length) entities.push({ type: "section", label }, ...section);
  }
  const url = config.dashboard_url ?? hass.devices?.[device]?.configuration_url;
  if (url && /^https?:\/\//i.test(url)) {
    entities.push({ type: "weblink", name: "Open QSync admin", url, icon: "mdi:open-in-new" });
  }
  if (!entities.some((row) => row.entity)) {
    return {
      type: "markdown",
      title: config.title ?? "QSync",
      content: matches.length > 1 && !device
        ? "Choose a QSync device by setting `device_id` in this card."
        : "Set up LG Hue Sync in Home Assistant, or specify its entity IDs in this card.",
    };
  }
  return { type: "entities", title: config.title ?? "QSync", show_header_toggle: false, entities };
}

// Extra modules can load before Home Assistant replaces the browser's element registry.
for (let attempts = 0; attempts < 300 && !customElements.get("home-assistant"); attempts++) {
  await new Promise((resolve) => setTimeout(resolve, 100));
}

class QSyncCard extends HTMLElement {
  #config;
  #hass;
  #helpers;
  #card;
  #signature;
  #pipeline;
  #preview = document.createElement("details");
  #canvas = document.createElement("canvas");
  #message = document.createElement("div");
  #regions = document.createElement("div");
  #showRegions = document.createElement("input");
  #inView = false;
  #subscription;
  #observer = new IntersectionObserver(([entry]) => {
    this.#inView = entry.isIntersecting;
    this.#updatePreview();
  });

  constructor() {
    super();
    const summary = document.createElement("summary");
    summary.textContent = "Live HDMI preview";
    summary.style.cursor = "pointer";
    this.#canvas.style.cssText = "display:block;width:100%;height:auto";
    this.#canvas.setAttribute("role", "img");
    this.#canvas.setAttribute("aria-label", "Live HDMI thumbnail");
    this.#message.setAttribute("role", "status");
    const screen = document.createElement("div");
    screen.style.position = "relative";
    this.#regions.style.cssText = "position:absolute;inset:0;pointer-events:none";
    this.#regions.setAttribute("aria-label", "Hue and Nanoleaf screen sampling regions");
    screen.append(this.#canvas, this.#regions);
    const regionToggle = document.createElement("label");
    this.#showRegions.type = "checkbox";
    this.#showRegions.checked = true;
    this.#showRegions.addEventListener("change", () => {
      this.#regions.hidden = !this.#showRegions.checked;
    });
    regionToggle.append(this.#showRegions, " Light regions");
    const panel = document.createElement("ha-card");
    panel.style.cssText = "margin-top:8px;padding:12px 16px";
    this.#preview.append(summary, this.#message, regionToggle, screen);
    panel.append(this.#preview);
    this.#preview.addEventListener("toggle", this.#updatePreview);
    this.#draw(null, "Waiting for HDMI capture…");
  }

  connectedCallback() {
    document.addEventListener("visibilitychange", this.#updatePreview);
    this.#observer.observe(this.#preview);
  }

  disconnectedCallback() {
    document.removeEventListener("visibilitychange", this.#updatePreview);
    this.#observer.disconnect();
    this.#inView = false;
    this.#updatePreview();
  }

  static getStubConfig() {
    return {};
  }

  setConfig(config) {
    this.#config = config;
    this.#update();
  }

  set hass(hass) {
    this.#hass = hass;
    this.#update();
  }

  getCardSize() {
    return this.#card?.getCardSize?.() ?? 5;
  }

  getGridOptions() {
    return this.#card?.getGridOptions?.() ?? { columns: 12, min_columns: 6 };
  }

  #draw(frame, message = "Waiting for active HDMI capture…") {
    this.#canvas.hidden = !frame;
    this.#canvas.style.display = frame ? "block" : "none";
    this.#message.textContent = frame ? "" : message;
    if (!frame) {
      this.#canvas.width = 0;
      this.#regions.replaceChildren();
      return;
    }
    const { width, height, rgb } = frame;
    if (this.#canvas.width !== width) this.#canvas.width = width;
    if (this.#canvas.height !== height) this.#canvas.height = height;
    const context = this.#canvas.getContext("2d");
    const image = context.createImageData(width, height);
    for (let pixel = 0; pixel < width * height; pixel++) {
      const src = pixel * 3;
      const dst = pixel * 4;
      image.data[dst] = rgb[src];
      image.data[dst + 1] = rgb[src + 1];
      image.data[dst + 2] = rgb[src + 2];
      image.data[dst + 3] = 255;
    }
    context.putImageData(image, 0, 0);
    this.#drawRegions(frame);
  }

  #drawRegions({ hue = [], nanoleaf = [] }) {
    const regions = hue.map(({ bounds: [left, top, right, bottom], name }) => {
      const region = document.createElement("div");
      region.style.cssText = "position:absolute;box-sizing:border-box;border:1px solid white;" +
        "background:#ffffff20;overflow:hidden";
      region.style.left = `${left * 100}%`;
      region.style.top = `${top * 100}%`;
      region.style.width = `${(right - left) * 100}%`;
      region.style.height = `${(bottom - top) * 100}%`;
      const label = document.createElement("span");
      label.style.cssText = "background:#000b;color:white;font-size:0.7rem;padding:1px 3px";
      label.textContent = name;
      region.append(label);
      return region;
    });
    // Same perimeter order as the QSync admin preview.
    const edges = {
      top: Array.from({ length: 13 }, (_, index) => 16 + index),
      right: Array.from({ length: 7 }, (_, index) => 29 + index),
      bottom: Array.from({ length: 13 }, (_, index) => index < 7 ? 7 - index : 46 - index),
      left: Array.from({ length: 7 }, (_, index) => 15 - index),
    };
    if (nanoleaf.length) {
      for (const [side, indexes] of Object.entries(edges)) {
        const horizontal = side === "top" || side === "bottom";
        const edge = document.createElement("div");
        edge.style.cssText = horizontal
          ? "position:absolute;inset-inline:0;height:5px;display:flex"
          : "position:absolute;inset-block:0;width:5px;display:flex;flex-direction:column";
        edge.style[side] = "0";
        for (const index of indexes) {
          const segment = document.createElement("span");
          segment.style.flex = "1";
          const rgb = nanoleaf[index];
          if (rgb) segment.style.background = `rgb(${rgb.join(",")})`;
          edge.append(segment);
        }
        regions.push(edge);
      }
    }
    this.#regions.replaceChildren(...regions);
  }

  #updatePreview = () => {
    const connection = this.#hass?.connection;
    const entityId = this.#pipeline;
    const pipeline = this.#hass?.states[entityId];
    const active = this.isConnected && this.#inView && !document.hidden &&
      this.#preview.open && pipeline && pipeline.state !== "unavailable" &&
      connection && this.#hass.connected !== false;
    const current = this.#subscription;
    if (active && current?.entityId === entityId && current.connection === connection) return;
    if (!active && !current) return;
    this.#subscription = undefined;
    current?.unsubscribe?.()?.catch(() => {});
    this.#draw(null);
    if (!active) return;

    const subscription = { entityId, connection };
    this.#subscription = subscription;
    connection.subscribeMessage((frame) => {
      if (this.#subscription === subscription) this.#draw(frame);
    }, { type: "lg_hue_sync/preview", entity_id: entityId }).then((unsubscribe) => {
      if (this.#subscription === subscription) subscription.unsubscribe = unsubscribe;
      else return unsubscribe();
    }).catch(() => {
      if (this.#subscription === subscription) {
        this.#draw(null, "Could not subscribe to QSync preview. Close and reopen to retry.");
      }
    });
  };

  #update() {
    if (!this.#config || !this.#hass) return;
    this.#helpers ??= Promise.resolve().then(() => window.loadCardHelpers());
    this.#helpers.then(({ createCardElement }) => {
      const config = cardConfig(this.#config, this.#hass);
      const signature = JSON.stringify(config);
      if (signature !== this.#signature) {
        this.#card = createCardElement(config);
        this.#signature = signature;
        this.#pipeline = config.entities?.find((row) => row.name === "Pipeline")?.entity;
        this.replaceChildren(...(this.#pipeline ? [this.#preview.parentElement] : []), this.#card);
      }
      this.#card.hass = this.#hass;
      this.#updatePreview();
    }).catch((error) => {
      this.textContent = `Unable to load the QSync card: ${error.message}`;
    });
  }
}

if (customElements.get("home-assistant") && !customElements.get("qsync-card")) {
  customElements.define("qsync-card", QSyncCard);
  (window.customCards ??= []).push({
    type: "qsync-card",
    name: "QSync",
    description: "Screen sync, tuning modes, and status from LG Hue Sync.",
  });
}
