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
const TUNING = [
  ["number", "brightness", "Brightness"],
  ["number", "saturation", "Saturation"],
  ["number", "highlights", "Highlights"],
  ["number", "blend", "Blend"],
  ["number", "gamma", "Gamma"],
  ["number", "response_strength", "Response strength"],
  ["number", "scene_limit", "Scene limit"],
  ["number", "black_gate", "Black gate"],
  ["number", "depth_influence", "Depth influence"],
  ["number", "black_screen_hold", "Black-screen hold"],
  ["number", "black_screen_fade", "Black-screen fade"],
  ["number", "hue_output_brightness", "Hue output brightness"],
  ["number", "nanoleaf_output_brightness", "Nanoleaf output brightness"],
  ["switch", "strict_black", "Strict black"],
  ["switch", "direct_rgb_mode", "Direct RGB mode"],
  ["switch", "letterbox_compensation", "Letterbox compensation"],
  ["switch", "midtone_lift_legacy", "Midtone lift (legacy)"],
  ["switch", "hue_screen_sync", "Hue screen sync"],
  ["switch", "nanoleaf_screen_sync", "Nanoleaf screen sync"],
  ["switch", "follow_tv_power", "Follow TV power"],
  ["button", "reset_colour", "Reset colour"],
  ["button", "save_settings", "Save settings"],
];
const SIGNAL = [
  ["sensor", "hdmi_frame_rate", "HDMI frame rate"],
  ["sensor", "hdmi_hdr_format", "HDMI HDR format"],
  ["sensor", "display_refresh_rate", "Display refresh rate"],
  ["sensor", "arc_status", "ARC status"],
  ["sensor", "audio_transport_codec", "Audio transport codec"],
  ["sensor", "audio_transport_rate", "Audio transport rate"],
  ["sensor", "audio_output_codec", "Audio output codec"],
  ["event", "pipeline_activity", "Pipeline activity"],
  ["button", "refresh_diagnostics", "Refresh diagnostics"],
];
const SIGNAL_ENTITIES = new Set([
  "video_frame_rate", "video_hdr", "display_refresh_rate", "output_arc_state",
  "transport_codec", "transport_sample_rate_hz", "decoder_output_codec",
]);

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
        (entry.entity_id.endsWith(`_${key}`) || entry.entity_id === `${domain}.${key}`) &&
        hass.states[entry.entity_id],
    )?.entity_id;
    return entity ? [{ entity, name }] : [];
  });
  const controls = rows(CONTROLS);
  const modes = controls.filter(({entity}) => entity.startsWith("select."));
  const entities = controls.filter(({entity}) => !entity.startsWith("select."));
  for (const [label, fields] of [["Status", STATUS], ["Diagnostics", SIGNAL], ["Maintenance", MAINTENANCE]]) {
    const section = rows(fields);
    if (label === "Diagnostics") {
      const pipeline = rows(STATUS)[0]?.entity;
      const attributes = hass.states[pipeline]?.attributes ?? {};
      for (const [key, label] of [["video", "HDMI"], ["display", "Display"], ["transport", "Audio transport"], ["decoder", "Audio decoder"], ["output", "eARC / audio output"]]) {
        const prefix = `signal_${key}_`;
        for (const attribute of Object.keys(attributes).filter((name) => name.startsWith(prefix))) {
          if (SIGNAL_ENTITIES.has(attribute.slice("signal_".length))) continue;
          const field = attribute.slice(prefix.length);
          section.push({ type: "attribute", entity: pipeline, attribute,
            name: `${label} · ${field.replaceAll("_", " ")}` });
        }
      }
      if (attributes.diagnostics_sampled_at) {
        section.push({ type: "attribute", entity: pipeline, attribute: "diagnostics_sampled_at",
          name: "Last sample", format: "datetime" });
      }
    }
    if (label === "Status") {
      const pipeline = section.find((row) => row.name === "Pipeline")?.entity;
      const attributes = hass.states[pipeline]?.attributes;
      for (const [attribute, name, suffix] of [
        ["capture_fps", "Capture rate", " fps"],
        ["light_update_fps", "Light update rate", " fps"],
        ["capture_resolution", "Capture resolution", ""],
        ["daemon_version", "TV daemon version", ""],
      ]) {
        if (attributes?.[attribute] != null) {
          section.push({ type: "attribute", entity: pipeline, attribute, name, suffix });
        }
      }
      const tv = attributes?.tv_device;
      const player = tv && Object.values(hass.entities ?? {}).find(
        (entry) => entry.device_id === tv && entry.entity_id.startsWith("media_player.") &&
          hass.states[entry.entity_id],
      );
      if (player) section.push({ entity: player.entity_id, name: "LG TV" });
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
  return {
    type: "entities", title: config.title ?? "QSync", show_header_toggle: false, entities,
    modes, tuning: rows(TUNING),
  };
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
  #ambient = document.createElement("ha-card");
  #advanced = document.createElement("details");
  #advancedCard;
  #modeSignature;
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
    this.#ambient.style.cssText = "margin-block:8px;padding:16px";
    this.#advanced.style.cssText = "margin-block:8px;padding:12px 16px";
    const tuningTitle = document.createElement("summary");
    tuningTitle.textContent = "Advanced tuning";
    this.#advanced.append(tuningTitle);
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

  static async getConfigElement() {
    return document.createElement("qsync-card-editor");
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
      const { modes = [], tuning = [], ...config } = cardConfig(this.#config, this.#hass);
      const modeState = modes.map(({entity, name}) => {
        const state = this.#hass.states[entity];
        return [entity, name, state?.state, state?.attributes.options];
      });
      const modeSignature = JSON.stringify(modeState);
      if (modeSignature !== this.#modeSignature) {
        this.#modeSignature = modeSignature;
        this.#renderModes(modeState);
      }
      const signature = JSON.stringify([
        config, modes.length > 0, tuning, this.#config.show_preview, this.#config.show_tuning,
      ]);
      if (signature !== this.#signature) {
        this.#card = createCardElement(config);
        this.#signature = signature;
        this.#pipeline = config.entities?.find((row) => row.name === "Pipeline")?.entity;
        this.#advancedCard?.remove();
        this.#advancedCard = tuning.length ? createCardElement({
          type: "entities", show_header_toggle: false, entities: tuning,
        }) : undefined;
        if (this.#advancedCard) this.#advanced.append(this.#advancedCard);
        this.replaceChildren(
          ...(this.#pipeline && this.#config.show_preview !== false ? [this.#preview.parentElement] : []),
          ...(modes.length ? [this.#ambient] : []),
          this.#card,
          ...(this.#advancedCard && this.#config.show_tuning !== false ? [this.#advanced] : []),
        );
      }
      this.#card.hass = this.#hass;
      if (this.#advancedCard) this.#advancedCard.hass = this.#hass;
      this.#updatePreview();
    }).catch((error) => {
      this.textContent = `Unable to load the QSync card: ${error.message}`;
    });
  }

  #renderModes(modes) {
    this.#ambient.replaceChildren();
    const title = document.createElement("h2");
    title.textContent = "Ambient style";
    title.style.cssText = "font:inherit;font-size:1.25rem;margin:0 0 12px";
    this.#ambient.append(title);
    const message = document.createElement("div");
    message.setAttribute("role", "status");
    for (const [entity, name, state, options] of modes) {
      if (!Array.isArray(options)) continue;
      const group = document.createElement("fieldset");
      group.style.cssText = "border:0;padding:0;margin:0 0 12px;display:flex;flex-wrap:wrap;gap:6px";
      const legend = document.createElement("legend");
      legend.style.marginBottom = "6px";
      legend.textContent = name === "Color preset" ? "Colour" : name;
      group.append(legend);
      for (const option of options) {
        const button = document.createElement("button");
        const selected = state === option;
        button.type = "button";
        button.textContent = option;
        button.disabled = state === "unavailable";
        button.setAttribute("aria-pressed", String(selected));
        button.style.cssText = "font:inherit;padding:8px 12px;border-radius:var(--ha-card-border-radius,12px);" +
          "border:1px solid var(--divider-color);cursor:pointer;color:var(--primary-text-color);" +
          `background:var(--${selected ? "secondary-background-color" : "card-background-color"});` +
          (selected ? "border-color:var(--primary-color);font-weight:600;" : "");
        button.addEventListener("click", async () => {
          message.textContent = "";
          button.disabled = true;
          try {
            await this.#hass.callService("select", "select_option", { entity_id: entity, option });
          } catch {
            message.textContent = `Could not set ${name}.`;
          } finally {
            button.disabled = state === "unavailable";
          }
        });
        group.append(button);
      }
      this.#ambient.append(group);
    }
    this.#ambient.append(message);
  }
}

class QSyncCardEditor extends HTMLElement {
  #form = document.createElement("ha-form");
  #config;

  constructor() {
    super();
    this.#form.schema = [
      { name: "device_id", selector: { device: { filter: [{ integration: DOMAIN }] } } },
      { name: "title", selector: { text: {} } },
      { name: "show_preview", selector: { boolean: {} } },
      { name: "show_tuning", selector: { boolean: {} } },
    ];
    const labels = {
      device_id: "QSync device", title: "Card title",
      show_preview: "Show HDMI preview", show_tuning: "Show advanced tuning",
    };
    this.#form.computeLabel = (field) => labels[field.name];
    this.#form.addEventListener("value-changed", (event) => {
      const config = { ...this.#config, ...event.detail.value };
      if (!event.detail.value.device_id) delete config.device_id;
      this.dispatchEvent(new CustomEvent("config-changed", {
        detail: { config }, bubbles: true, composed: true,
      }));
    });
    this.append(this.#form);
  }

  setConfig(config) {
    this.#config = config;
    this.#form.data = { show_preview: true, show_tuning: true, ...config };
  }

  set hass(hass) {
    this.#form.hass = hass;
  }
}

if (customElements.get("home-assistant") && !customElements.get("qsync-card")) {
  customElements.define("qsync-card-editor", QSyncCardEditor);
  customElements.define("qsync-card", QSyncCard);
  (window.customCards ??= []).push({
    type: "qsync-card",
    name: "QSync",
    description: "Screen sync, tuning modes, and status from LG Hue Sync.",
  });
}
