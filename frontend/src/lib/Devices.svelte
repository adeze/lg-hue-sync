<script lang="ts">
  import SettingToggle from './SettingToggle.svelte';
  import { Button } from './components/ui/button';
  import { Link } from '@lucide/svelte';
  import Fader from './Fader.svelte';
  import type { Dashboard } from './dashboard.svelte';
  let { model }: { model: Dashboard } = $props();
  let hueIp = $state(''),
    nanoleafIp = $state(''),
    area = $state('');
  let areas = $state<{ id: string; name: string }[]>([]),
    areaMessage = $state(''),
    loaded = $state(false);
  let alignmentDraft = $state<{
    start_corner: string;
    reverse_direction: boolean;
    perimeter_offset: number;
  } | null>(null);
  let settings = $derived(model.settings);
  let alignment = $derived(
    alignmentDraft ??
      settings?.nanoleaf_alignment ?? {
        start_corner: 'bottom-center',
        reverse_direction: false,
        perimeter_offset: 0,
      },
  );
  async function run(path: string, body?: unknown) {
    try {
      await model.command(path, body);
    } catch {
      /* Shared error banner reports failures. */
    }
  }
  async function loadAreas() {
    try {
      const { request } = await import('./api');
      const result = await request<{
        areas: { id: string; name: string }[];
        selected_area_id: string;
      }>('/api/hue/areas');
      areas = result.areas;
      area = result.selected_area_id;
      areaMessage = `${areas.length} Entertainment Area${areas.length === 1 ? '' : 's'} found.`;
    } catch (error) {
      areaMessage = String(error);
    }
  }
  function showDevices(event: Event) {
    if ((event.currentTarget as HTMLDetailsElement).open && !loaded) {
      loaded = true;
      void loadAreas();
    }
  }
  async function saveAlignment() {
    try {
      await model.command('/api/nanoleaf/alignment', alignment);
      if (model.settings)
        model.settings = { ...model.settings, nanoleaf_alignment: { ...alignment } };
      alignmentDraft = null;
    } catch {
      /* Shared error banner reports failures. */
    }
  }
  const patterns = [
    ['red', 'Red'],
    ['green', 'Green'],
    ['blue', 'Blue'],
    ['white', 'White'],
    ['warm-white', 'Warm 3000K'],
    ['daylight-white', 'Daylight D65'],
    ['quadrants', 'Quadrants'],
    ['perimeter', '4D tracer'],
    ['off', 'Return to sync'],
  ];
  const trimStops = [0, 0.5, 1, 1.5, 2, 2.5];
</script>

<details class="card accordion" ontoggle={showDevices} id="devicesSection">
  <summary>Devices <span class="muted">Hue &amp; Nanoleaf</span></summary>
  <div class="section-body">
    <SettingToggle
      title="Follow TV power"
      description="Pause lights in standby and resume when the TV turns on."
      checked={settings?.auto_tv_power ?? true}
      disabled={!settings || model.busy}
      onchange={(checked) => model.patch({ auto_tv_power: checked })}
    />
    <section class="device-subsection">
      <h2>Philips Hue</h2>
      <p class="muted">
        Press the Bridge button, then pair within 45 seconds. Pairing stays on the TV.
      </p>
      <label class="field"
        >Bridge address <input
          bind:value={hueIp}
          placeholder="Leave blank to discover"
          inputmode="decimal"
        /></label
      >
      <Button
        variant="secondary"
        disabled={model.busy}
        onclick={() => run('/api/pair/hue', { ip: hueIp.trim() || null })}
        ><Link aria-hidden="true" />Pair Hue Bridge</Button
      >
      <p class="device-status">
        {model.status?.hue_connected ? 'Connected' : 'Disconnected'}{model.status?.hue_bridge_ip
          ? ` · ${model.status.hue_bridge_ip}`
          : ''}
      </p>
      <label class="field"
        >Entertainment Area <select
          bind:value={area}
          disabled={!areas.length || model.busy}
          onchange={() => run('/api/hue/area', { area_id: area })}
          >{#each areas as item (item.id)}<option value={item.id}>{item.name}</option
            >{/each}</select
        ></label
      >
      <div class="button-row">
        <Button variant="secondary" onclick={loadAreas}>Refresh areas</Button><Button
          variant="secondary"
          disabled={model.busy}
          onclick={() => run('/api/sync-bridge')}>Refresh selected layout</Button
        >
      </div>
      <p class="muted" role="status">{areaMessage}</p>
      <SettingToggle
        title="Hue Entertainment sync"
        description="Release Hue lights to the Hue app while Nanoleaf can keep running."
        checked={settings?.hue_sync_enabled ?? false}
        disabled={!settings || model.busy}
        onchange={(checked) => model.patch({ hue_sync_enabled: checked })}
      />
      {#if settings}<Fader
          label="Hue output trim"
          value={settings.hue_output_brightness}
          stops={trimStops}
          format={(v) => `${v.toFixed(2)}×`}
          help="Final Hue brightness trim; independent of Nanoleaf."
          vertical={false}
          onchange={(v) => model.patch({ hue_output_brightness: v })}
          onhelp={() => {}}
          disabled={model.busy}
        />{/if}
    </section>
    <section class="device-subsection">
      <h2>Nanoleaf 4D</h2>
      <p class="muted">Hold the controller power button 5–7 seconds until it blinks, then pair.</p>
      <label class="field"
        >Controller address <input
          bind:value={nanoleafIp}
          placeholder={model.status?.nanoleaf_ip || 'Nanoleaf address'}
          inputmode="decimal"
        /></label
      >
      <Button
        variant="secondary"
        disabled={model.busy}
        onclick={() => run('/api/pair/nanoleaf', { ip: nanoleafIp.trim() || null })}
        >Pair Nanoleaf 4D</Button
      >
      <p class="device-status">
        {model.status?.nanoleaf_connected ? 'Connected' : 'Disconnected'}{model.status?.nanoleaf_ip
          ? ` · ${model.status.nanoleaf_ip}`
          : ''}
      </p>
      <SettingToggle
        title="Nanoleaf 4D sync"
        description="Stop TV-driven 4D updates while Hue can keep running."
        checked={settings?.nanoleaf_sync_enabled ?? false}
        disabled={!settings || model.busy}
        onchange={(checked) => model.patch({ nanoleaf_sync_enabled: checked })}
      />
      {#if settings}<Fader
          label="Nanoleaf output trim"
          value={settings.nanoleaf_output_brightness}
          stops={trimStops}
          format={(v) => `${v.toFixed(2)}×`}
          help="Balances 4D output without changing screen sampling."
          vertical={false}
          onchange={(v) => model.patch({ nanoleaf_output_brightness: v })}
          onhelp={() => {}}
          disabled={model.busy}
        />{/if}
    </section>
  </div>
</details>
<details class="card accordion">
  <summary>Calibration</summary>
  <div class="section-body">
    <p class="muted">
      Visual matching only; these patterns do not measure light or alter the TV picture settings.
    </p>
    <a class="text-link" href="/capture-patterns" target="_blank" rel="noopener"
      >Open TV capture patterns ↗</a
    >
    <div class="button-row">
      {#each patterns as [value, label] (value)}<Button
          variant="secondary"
          disabled={model.busy}
          onclick={() => run(`/api/test-pattern/${value}`)}>{label}</Button
        >{/each}
    </div>
    <section class="device-subsection">
      <h2>Nanoleaf alignment</h2>
      <p class="muted">Use the 4D tracer to locate the first physical segment.</p>
      <label class="field"
        >Start corner <select
          value={alignment.start_corner}
          onchange={(event) =>
            (alignmentDraft = { ...alignment, start_corner: event.currentTarget.value })}
          >{#each ['bottom-center', 'bottom-left', 'top-left', 'top-right', 'bottom-right'] as corner (corner)}<option
              value={corner}>{corner.replaceAll('-', ' ')}</option
            >{/each}</select
        ></label
      >
      <SettingToggle
        title="Reverse direction"
        description="Reverse the order of the Nanoleaf perimeter segments."
        checked={alignment.reverse_direction}
        onchange={(checked) => (alignmentDraft = { ...alignment, reverse_direction: checked })}
      />
      <label class="field"
        >Perimeter offset · {alignment.perimeter_offset}<input
          type="range"
          min="0"
          max="39"
          step="1"
          value={alignment.perimeter_offset}
          oninput={(event) =>
            (alignmentDraft = {
              ...alignment,
              perimeter_offset: Number(event.currentTarget.value),
            })}
        /></label
      >
      <Button variant="secondary" disabled={model.busy} onclick={saveAlignment}
        >Apply alignment</Button
      >
    </section>
  </div>
</details>
<details class="card accordion">
  <summary>Maintenance</summary>
  <div class="section-body">
    <SettingToggle
      title="Bonjour discovery"
      description="Advertise dashboard and API on the local network. Changes save without a restart."
      checked={model.status?.bonjour_enabled ?? false}
      disabled={!model.status || model.busy}
      onchange={(checked) => run('/api/bonjour', { enabled: checked })}
    />
    <Button
      variant="secondary"
      disabled={model.busy}
      onclick={() => {
        if (confirm('Restart capture and lighting pipelines? Output pauses briefly.'))
          void run('/api/restart');
      }}>Restart pipeline</Button
    >
  </div>
</details>
