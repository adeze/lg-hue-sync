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

  #update() {
    if (!this.#config || !this.#hass) return;
    this.#helpers ??= Promise.resolve().then(() => window.loadCardHelpers());
    this.#helpers.then(({ createCardElement }) => {
      const config = cardConfig(this.#config, this.#hass);
      const signature = JSON.stringify(config);
      if (signature !== this.#signature) {
        this.#card = createCardElement(config);
        this.#signature = signature;
        this.replaceChildren(this.#card);
      }
      this.#card.hass = this.#hass;
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
