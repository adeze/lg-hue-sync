const DOMAIN = "lg_hue_sync";
const CONTROLS = [
  ["switch", "screen_sync", "Screen sync"],
  ["switch", "display", "Display"],
  ["switch", "hue_screen_sync", "Hue sync"],
  ["switch", "nanoleaf_screen_sync", "Nanoleaf 4D sync"],
  ["select", "color_preset", "Color preset"],
  ["select", "response", "Response"],
  ["select", "spatial_blending", "Spatial blending"],
];
const STATUS = [["sensor", "pipeline", "Pipeline"]];
const MAINTENANCE = [["button", "restart_pipeline", "Restart pipeline"]];
const COLOUR_LABELS = { "Soft color": "Soft", Neutral: "Default", "Vivid color": "Vivid" };
const SEGMENT_STYLES = `
  qsync-card { display: flex; flex-direction: column; gap: 8px; }
  qsync-card > [role="status"]:empty { display: none; }
  qsync-card .qsync-refresh:where(:not([hidden])) {
    display: inline-flex; align-items: center; justify-content: center;
    gap: 8px; min-height: 44px; margin-block: 8px;
    border: 1px solid var(--divider-color); border-radius: 12px; padding: 0 14px;
    background: var(--secondary-background-color); color: var(--primary-text-color);
    font: inherit; cursor: pointer;
  }
  qsync-card .qsync-refresh:focus-visible { outline: 2px solid var(--primary-color); }
  qsync-card .qsync-refresh:disabled { color: var(--disabled-text-color); cursor: default; }
  qsync-card .qsync-mode { border: 0; padding: 0; margin: 0 0 14px; min-inline-size: 0; }
  qsync-card .qsync-mode legend { margin-block-end: 6px; color: var(--secondary-text-color); }
  qsync-card .qsync-segments {
    position: relative; display: grid; grid-auto-flow: column; grid-auto-columns: minmax(0, 1fr);
    padding: 3px; border: 1px solid var(--divider-color);
    border-radius: 14px; background: var(--secondary-background-color);
  }
  qsync-card .qsync-indicator {
    position: absolute; inset-block: 3px; inset-inline-start: 3px;
    inline-size: calc((100% - 6px) / var(--count)); border-radius: 10px;
    background: var(--primary-color); opacity: 0; pointer-events: none;
    translate: calc(100% * var(--selected)) 0;
    transition: translate 180ms ease, opacity 120ms ease;
  }
  qsync-card .qsync-segments:dir(rtl) .qsync-indicator { translate: calc(-100% * var(--selected)) 0; }
  qsync-card .qsync-segments[data-selected] .qsync-indicator { opacity: 1; }
  qsync-card .qsync-segments button {
    position: relative; min-inline-size: 0; min-block-size: 44px; padding: 4px;
    border: 0; border-radius: 10px; background: transparent;
    color: var(--primary-text-color); font: inherit; cursor: pointer; overflow-wrap: anywhere;
  }
  qsync-card .qsync-segments button[aria-pressed="true"] {
    color: var(--text-primary-color, white); font-weight: 600;
  }
  qsync-card .qsync-segments button:focus-visible { outline: 2px solid currentColor; outline-offset: -3px; }
  qsync-card .qsync-segments button:disabled { cursor: default; color: var(--disabled-text-color); }
  qsync-card .qsync-switches, qsync-card .qsync-dials {
    display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 130px), 1fr));
    gap: 8px;
  }
  qsync-card .qsync-dials { margin-block: 8px; }
  qsync-card .qsync-switch, qsync-card .qsync-dial {
    border: 1px solid var(--divider-color); border-radius: 14px; padding: 10px;
    background: var(--ha-card-background, var(--card-background-color));
    color: var(--primary-text-color); font: inherit; text-align: center;
  }
  qsync-card .qsync-switch { min-height: 64px; cursor: pointer; font-weight: 600; }
  qsync-card .qsync-switch[aria-checked="true"] {
    background: var(--primary-color); color: var(--text-primary-color, white);
  }
  qsync-card .qsync-switch:disabled, qsync-card .qsync-dial[aria-disabled="true"] {
    opacity: .55; cursor: default;
  }
  qsync-card .qsync-switch:focus-visible, qsync-card .qsync-dial:focus-visible {
    outline: 2px solid var(--primary-text-color); outline-offset: -4px;
  }
  qsync-card .qsync-dial { touch-action: pan-y; cursor: ew-resize; user-select: none; }
  qsync-card .qsync-dial-knob {
    display: grid; place-items: start center; margin: 6px auto; width: 36px; height: 36px;
    border-radius: 50%; border: 2px solid var(--divider-color);
    transform: rotate(var(--angle, -135deg)); color: var(--primary-color);
  }
  @media (prefers-reduced-motion: reduce) {
    qsync-card .qsync-indicator { transition: none; }
  }
`;
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
        (key !== "screen_sync" || !/[_\.](hue|nanoleaf)_screen_sync$/.test(entry.entity_id)) &&
        hass.states[entry.entity_id],
    )?.entity_id;
    return entity ? [{ entity, name }] : [];
  });
  const controls = rows(CONTROLS);
  const modes = controls.filter(({entity}) => entity.startsWith("select."));
  const switches = controls.filter(({entity}) => entity.startsWith("switch."));
  const entities = [];
  let diagnostics = [];
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
    }
    if (label === "Diagnostics") diagnostics = section;
    else if (section.length) entities.push({ type: "section", label }, ...section);
  }
  const url = config.dashboard_url ?? hass.devices?.[device]?.configuration_url;
  if (url && /^https?:\/\//i.test(url)) {
    entities.push({ type: "weblink", name: "Open QSync admin", url, icon: "mdi:open-in-new" });
  }
  if (!switches.length && !entities.some((row) => row.entity) && !modes.length) {
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
    modes, switches, diagnostics, tuning: rows(TUNING),
    refreshDiagnostics: rows([["button", "refresh_diagnostics", "Refresh diagnostics"]])[0]?.entity,
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
  #diagnostics = document.createElement("details");
  #diagnosticsCard;
  #refreshDiagnostics = document.createElement("button");
  #refreshEntity;
  #styles = document.createElement("style");
  #switches = document.createElement("div");
  #dials = document.createElement("div");
  #controlsSignature;
  #controlUpdates = [];
  #controlMessage = document.createElement("div");
  #advancedCard;
  #modeSignature;
  #modeGroups = [];
  #modeMessage = document.createElement("div");
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
    panel.style.padding = "12px 16px";
    this.#preview.append(summary, this.#message, regionToggle, screen);
    panel.append(this.#preview);
    this.#preview.addEventListener("toggle", this.#updatePreview);
    this.#ambient.style.padding = "16px";
    this.#advanced.style.padding = "12px 16px";
    const tuningTitle = document.createElement("summary");
    tuningTitle.textContent = "Advanced tuning";
    this.#advanced.append(tuningTitle);
    this.#diagnostics.style.padding = "12px 16px";
    const diagnosticsTitle = document.createElement("summary");
    diagnosticsTitle.textContent = "Diagnostics";
    this.#refreshDiagnostics.type = "button";
    this.#refreshDiagnostics.className = "qsync-refresh";
    const refreshIcon = document.createElement("ha-icon");
    refreshIcon.setAttribute("icon", "mdi:refresh");
    refreshIcon.setAttribute("aria-hidden", "true");
    this.#refreshDiagnostics.append(refreshIcon, "Refresh diagnostics");
    this.#refreshDiagnostics.addEventListener("click", async () => {
      this.#refreshDiagnostics.setAttribute("aria-busy", "true");
      this.#updateRefreshButton();
      try {
        await this.#setControl("button", "press", this.#refreshEntity, "diagnostics");
      } finally {
        this.#refreshDiagnostics.removeAttribute("aria-busy");
        this.#updateRefreshButton();
      }
    });
    this.#diagnostics.append(diagnosticsTitle, this.#refreshDiagnostics);
    this.#advanced.append(this.#dials);
    this.#switches.className = "qsync-switches";
    this.#dials.className = "qsync-dials";
    this.#controlMessage.setAttribute("role", "status");
    this.#styles.textContent = SEGMENT_STYLES;
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
    return { columns: 12, min_columns: 6, rows: "auto" };
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
      const { modes = [], switches = [], diagnostics = [], tuning = [], refreshDiagnostics, ...config } =
        cardConfig(this.#config, this.#hass);
      this.#refreshEntity = refreshDiagnostics;
      this.#updateRefreshButton();
      const numbers = tuning.filter(({entity}) => entity.startsWith("number."));
      const controlsSignature = JSON.stringify([switches, numbers]);
      if (controlsSignature !== this.#controlsSignature) {
        this.#controlsSignature = controlsSignature;
        this.#controlUpdates = [];
        this.#switches.replaceChildren(...switches.map((row) => this.#renderSwitch(row)));
        this.#dials.replaceChildren(...numbers.map((row) => this.#renderDial(row)));
      }
      this.#controlUpdates.forEach((update) => update());
      const modeState = modes.map(({entity, name}) => {
        const state = this.#hass.states[entity];
        return [entity, name, state?.state, state?.attributes.options];
      });
      const modeSignature = JSON.stringify(modeState.map(([entity, name, , options]) => [entity, name, options]));
      if (modeSignature !== this.#modeSignature) {
        this.#modeSignature = modeSignature;
        this.#renderModes(modeState);
      }
      for (const { entity, track, buttons, options } of this.#modeGroups) {
        const state = this.#hass.states[entity]?.state;
        const selected = options.indexOf(state);
        track.style.setProperty("--selected", String(Math.max(0, selected)));
        track.toggleAttribute("data-selected", selected >= 0);
        buttons.forEach((button, index) => {
          button.disabled = !state || state === "unavailable";
          button.setAttribute("aria-pressed", String(index === selected));
        });
      }
      const signature = JSON.stringify([
        config, modes.length > 0, switches.length, diagnostics, tuning, refreshDiagnostics,
        this.#config.show_preview, this.#config.show_tuning,
      ]);
      if (signature !== this.#signature) {
        this.#card = createCardElement(config);
        this.#signature = signature;
        this.#pipeline = config.entities?.find((row) => row.name === "Pipeline")?.entity;
        this.#advancedCard?.remove();
        const tuningRows = [
          ["Processing", tuning.filter(({entity}) => entity.startsWith("switch."))],
          ["Save and reset", tuning.filter(({entity}) => entity.startsWith("button."))],
        ].flatMap(([label, rows]) => rows.length ? [{ type: "section", label }, ...rows] : []);
        this.#advancedCard = tuningRows.length ? createCardElement({
          type: "entities", show_header_toggle: false, entities: tuningRows,
        }) : undefined;
        if (this.#advancedCard) this.#advanced.append(this.#advancedCard);
        this.#diagnosticsCard?.remove();
        this.#diagnosticsCard = diagnostics.length ? createCardElement({
          type: "entities", show_header_toggle: false, entities: diagnostics,
        }) : undefined;
        if (this.#diagnosticsCard) this.#diagnostics.append(this.#diagnosticsCard);
        this.replaceChildren(
          this.#styles,
          ...(switches.length ? [this.#switches] : []),
          this.#controlMessage,
          ...(this.#pipeline && this.#config.show_preview !== false ? [this.#preview.parentElement] : []),
          ...(modes.length ? [this.#ambient] : []),
          this.#card,
          ...(this.#diagnosticsCard || refreshDiagnostics ? [this.#diagnostics] : []),
          ...(tuning.length && this.#config.show_tuning !== false ? [this.#advanced] : []),
        );
      }
      this.#card.hass = this.#hass;
      if (this.#advancedCard) this.#advancedCard.hass = this.#hass;
      if (this.#diagnosticsCard) this.#diagnosticsCard.hass = this.#hass;
      this.#updatePreview();
    }).catch((error) => {
      this.textContent = `Unable to load the QSync card: ${error.message}`;
    });
  }

  #updateRefreshButton() {
    const state = this.#hass.states[this.#refreshEntity]?.state;
    this.#refreshDiagnostics.hidden = !this.#refreshEntity;
    this.#refreshDiagnostics.disabled = !state || state === "unavailable" ||
      this.#refreshDiagnostics.hasAttribute("aria-busy");
  }

  async #setControl(domain, service, entity, name, data = {}) {
    this.#controlMessage.textContent = "";
    try {
      await this.#hass.callService(domain, service, { entity_id: entity, ...data });
    } catch {
      this.#controlMessage.textContent = "Could not " + (service === "press" ? "refresh " : "set ") + name + ".";
    }
  }

  #renderSwitch({ entity, name }) {
    const button = document.createElement("button");
    button.className = "qsync-switch";
    button.type = "button";
    button.setAttribute("role", "switch");
    button.setAttribute("aria-label", name);
    button.addEventListener("click", () => {
      const state = this.#hass.states[entity]?.state;
      if (state === "on" || state === "off") {
        this.#setControl("switch", state === "on" ? "turn_off" : "turn_on", entity, name);
      }
    });
    this.#controlUpdates.push(() => {
      const state = this.#hass.states[entity]?.state;
      const on = state === "on";
      button.disabled = !on && state !== "off";
      button.setAttribute("aria-checked", String(on));
      button.textContent = name + (button.disabled ? " unavailable" : on ? " on" : " off");
    });
    return button;
  }

  #renderDial({ entity, name }) {
    const dial = document.createElement("div");
    dial.className = "qsync-dial";
    dial.setAttribute("role", "slider");
    dial.setAttribute("aria-label", name);
    const knob = document.createElement("span");
    knob.className = "qsync-dial-knob";
    knob.setAttribute("aria-hidden", "true");
    knob.textContent = "•";
    const readout = document.createElement("span");
    dial.append(name, knob, readout);
    let range;
    let drag;
    let pending = false;
    const show = (value) => {
      dial.setAttribute("aria-valuenow", value);
      readout.textContent = String(value) + (range.unit ? " " + range.unit : "");
      knob.style.setProperty("--angle", (-135 + 270 * (value - range.min) / (range.max - range.min)) + "deg");
    };
    const update = () => {
      const state = this.#hass.states[entity];
      const value = Number(state?.state || NaN);
      const { min, max, step, unit_of_measurement: unit } = state?.attributes ?? {};
      range = [min, max, step, value].every(Number.isFinite) && max > min && step > 0
        ? { min, max, step, unit, value }
        : undefined;
      dial.tabIndex = range ? 0 : -1;
      dial.setAttribute("aria-disabled", String(!range));
      if (!range) {
        drag = undefined;
        for (const attr of ["aria-valuemin", "aria-valuemax", "aria-valuenow"]) dial.removeAttribute(attr);
        readout.textContent = "Unavailable";
        return;
      }
      dial.setAttribute("aria-valuemin", min);
      dial.setAttribute("aria-valuemax", max);
      if (!drag && !pending) show(value);
    };
    const stepped = (value) => {
      const tick = Math.round((value - range.min) / range.step);
      const rounded = Number((range.min + tick * range.step).toPrecision(12));
      return Math.max(range.min, Math.min(range.max, rounded));
    };
    const commit = async (value) => {
      if (!range || value === range.value) return update();
      pending = true;
      show(value);
      await this.#setControl("number", "set_value", entity, name, { value });
      pending = false;
      update();
    };
    dial.addEventListener("pointerdown", (event) => {
      if (!range || drag || pending || event.button !== 0) return;
      drag = { id: event.pointerId, x: event.clientX, start: range.value, value: range.value };
      dial.setPointerCapture(event.pointerId);
    });
    dial.addEventListener("pointermove", (event) => {
      if (drag?.id !== event.pointerId) return;
      const value = stepped(drag.start + Math.round((event.clientX - drag.x) / 8) * range.step);
      if (value === drag.value) return;
      drag.value = value;
      show(value);
      dial.dispatchEvent(new CustomEvent("haptic", { detail: "selection", bubbles: true, composed: true }));
    });
    dial.addEventListener("pointerup", (event) => {
      if (drag?.id !== event.pointerId) return;
      const { value } = drag;
      drag = undefined;
      commit(value);
    });
    dial.addEventListener("lostpointercapture", (event) => {
      if (drag?.id !== event.pointerId) return;
      drag = undefined;
      update();
    });
    dial.addEventListener("keydown", (event) => {
      if (!range || drag || pending) return;
      const { value, step, min, max } = range;
      const values = { ArrowLeft: value - step, ArrowDown: value - step,
        ArrowRight: value + step, ArrowUp: value + step, Home: min, End: max };
      if (!Object.hasOwn(values, event.key)) return;
      event.preventDefault();
      commit(stepped(values[event.key]));
    });
    this.#controlUpdates.push(update);
    return dial;
  }

  #renderModes(modes) {
    this.#ambient.replaceChildren();
    this.#modeGroups = [];
    const title = document.createElement("h2");
    title.textContent = "Ambient style";
    title.style.cssText = "font:inherit;font-size:1.25rem;margin:0 0 12px";
    this.#ambient.append(title);
    this.#modeMessage.setAttribute("role", "status");
    for (const [entity, name, , options] of modes) {
      if (!Array.isArray(options) || !options.length) continue;
      const group = document.createElement("fieldset");
      group.className = "qsync-mode";
      const legend = document.createElement("legend");
      legend.textContent = name === "Color preset" ? "Colour" : name;
      const track = document.createElement("div");
      track.className = "qsync-segments";
      track.style.setProperty("--count", String(options.length));
      const indicator = document.createElement("span");
      indicator.className = "qsync-indicator";
      indicator.setAttribute("aria-hidden", "true");
      track.append(indicator);
      const buttons = options.map((option) => {
        const button = document.createElement("button");
        button.type = "button";
        button.textContent = name === "Color preset" ? (COLOUR_LABELS[option] ?? option) : option;
        button.addEventListener("click", async () => {
          if (this.#hass.states[entity]?.state === option) return;
          this.#modeMessage.textContent = "";
          button.dispatchEvent(new CustomEvent("haptic", {
            detail: "selection", bubbles: true, composed: true,
          }));
          try {
            await this.#hass.callService("select", "select_option", { entity_id: entity, option });
          } catch {
            this.#modeMessage.textContent = `Could not set ${name}.`;
          }
        });
        return button;
      });
      track.append(...buttons);
      group.append(legend, track);
      this.#ambient.append(group);
      this.#modeGroups.push({ entity, options, track, buttons });
    }
    this.#ambient.append(this.#modeMessage);
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
