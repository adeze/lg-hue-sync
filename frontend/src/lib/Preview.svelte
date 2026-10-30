<script lang="ts">
  import { onDestroy } from 'svelte';
  import { Toggle } from './components/ui/toggle';
  import { Button } from './components/ui/button';
  import {
    FrozenPreview,
    deviceColor,
    drawPreview,
    groupHueZones,
    orientationDescription,
    orientationPath,
    rgbCss,
  } from './preview';
  import type { PreviewSettings, PreviewState, PreviewStatus } from './preview-types';

  let {
    status,
    settings,
    active,
  }: { status: PreviewStatus | null; settings: PreviewSettings | null; active: boolean } = $props();
  let visible = $state(true);
  let regions = $state(true);
  let preview = $state.raw<PreviewState>({
    busy: false,
    frame: null,
    rendered: null,
    message: 'Capture an HDMI frame to preview colour adjustments.',
  });
  const controller = new FrozenPreview((state) => {
    preview = state;
  });
  const devices = $derived(groupHueZones(status?.hue_zones || []));
  const supported = $derived(
    status?.pipeline_state === 'running' && status.capture_hardware === true,
  );
  const edges = $derived([
    {
      name: 'top',
      colors: Array.from({ length: 13 }, (_, index) => status?.nanoleaf_colors?.[16 + index]),
    },
    {
      name: 'right',
      colors: Array.from({ length: 7 }, (_, index) => status?.nanoleaf_colors?.[29 + index]),
    },
    {
      name: 'bottom',
      colors: Array.from({ length: 13 }, (_, index) =>
        index < 7 ? status?.nanoleaf_colors?.[7 - index] : status?.nanoleaf_colors?.[46 - index],
      ),
    },
    {
      name: 'left',
      colors: Array.from({ length: 7 }, (_, index) => status?.nanoleaf_colors?.[15 - index]),
    },
  ]);
  $effect(() => {
    controller.update(settings, active && visible, supported);
  });
  onDestroy(() => controller.cancel());
</script>

<svelte:document
  onvisibilitychange={() => {
    visible = !document.hidden;
  }}
/>

<section class="card frame-editor" aria-labelledby="frameEditorTitle">
  <div class="preview-heading">
    <h2 id="frameEditorTitle">Colour preview</h2>
    <details class="preview-help">
      <summary aria-label="About colour preview">ⓘ</summary>
      <p>
        Capture an HDMI frame to preview colour adjustments. Sliders also change the lights. Peak
        highlights, spatial blending, temporal response and physical lamp colour are not simulated.
        Capture freezes one image; adjustments reuse it until you capture again.
      </p>
    </details>
  </div>
  <p class="preview-message" role="status" aria-live="polite">{preview.message}</p>
  <div
    class="screen"
    style:aspect-ratio={preview.rendered
      ? `${preview.rendered.width} / ${preview.rendered.height}`
      : '16 / 9'}
  >
    <div role="img" aria-label="Captured HDMI frame with current colour adjustments">
      <canvas
        width="128"
        height="72"
        aria-hidden="true"
        {@attach (canvas) => {
          $effect(() => {
            if (preview.rendered) drawPreview(canvas, preview.rendered);
          });
        }}
      ></canvas>
    </div>
    {#if regions}
      <div class="zones" aria-label="Hue screen sampling zones">
        {#each devices as device (device.key)}
          {@const color = deviceColor(device, status?.hue_colors || [])}
          {@const orientation = orientationDescription(device.positions)}
          <div
            class="zone"
            style:inset-inline-start={`${device.x_min * 100}%`}
            style:inset-block-start={`${device.y_min * 100}%`}
            style:inline-size={`${(device.x_max - device.x_min) * 100}%`}
            style:block-size={`${(device.y_max - device.y_min) * 100}%`}
            style:border-color={rgbCss(color)}
            style:background={rgbCss(color, 0.18)}
            title={`${device.channels.map((zone) => `Channel ${zone.channel_id}: ${zone.name}`).join('\n')}\nHue layout: ${orientation || 'orientation unavailable'}`}
          >
            <span class="zone-name">{device.name}</span>
            {#if orientation}
              <span class="orientation" role="img" aria-label={orientation} title={orientation}>
                <svg
                  viewBox="0 0 32 40"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  aria-hidden="true"><path d={orientationPath(device.positions)} /></svg
                >
              </span>
            {/if}
          </div>
        {/each}
      </div>
      {#each edges as edge (edge.name)}
        <div class={`edge edge-${edge.name}`} aria-hidden="true">
          {#each edge.colors as color, index (`${edge.name}-${index}`)}<span
              style:background={rgbCss(color)}
            ></span>{/each}
        </div>
      {/each}
    {/if}
  </div>
  <div class="preview-actions">
    <Button
      onclick={() => controller.capture()}
      disabled={!active || !visible || !supported || !settings || preview.busy}
    >
      <svg
        width="18"
        height="18"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.8"
        aria-hidden="true"
        ><path d="m8 6 2-3h4l2 3h4a1 1 0 0 1 1 1v13H3V7a1 1 0 0 1 1-1Z" /><circle
          cx="12"
          cy="12"
          r="4"
        /></svg
      >
      Capture frame
    </Button>
    <Toggle variant="outline" bind:pressed={regions}>Light regions</Toggle>
  </div>
</section>

<style>
  .frame-editor {
    display: grid;
    gap: 0.75rem;
  }
  .preview-heading,
  .preview-actions {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }
  .preview-heading h2 {
    font-size: 0.95rem;
    font-weight: 650;
  }
  .preview-help {
    position: relative;
    margin-inline-start: auto;
  }
  .preview-help summary {
    display: grid;
    place-items: center;
    min-inline-size: 44px;
    min-block-size: 44px;
    cursor: pointer;
    list-style: none;
    color: var(--text-muted);
  }
  .preview-help summary::after {
    content: none;
  }
  .preview-help summary::-webkit-details-marker {
    display: none;
  }
  .preview-help summary:focus-visible {
    outline: 2px solid var(--focus-ring);
    outline-offset: 2px;
  }
  .preview-help summary:active {
    opacity: 0.75;
  }
  .preview-help p {
    position: absolute;
    z-index: 5;
    inset-block-start: 100%;
    inset-inline-end: 0;
    inline-size: min(20rem, 75vw);
    padding: 0.8rem;
    border: 1px solid var(--card-border);
    border-radius: 0.75rem;
    background: var(--card-bg);
    color: var(--text);
    font-size: 0.8rem;
    box-shadow: 0 0.5rem 1.5rem var(--card-shadow);
  }
  .preview-message {
    color: var(--text-muted);
    font-size: 0.75rem;
    line-height: 1.5;
  }
  .screen {
    position: relative;
    overflow: clip;
    inline-size: 100%;
    border: 1px solid var(--card-border);
    border-radius: 1rem;
    background: oklch(0.13 0.02 260);
  }
  canvas {
    position: absolute;
    inset: 0;
    inline-size: 100%;
    block-size: 100%;
    display: block;
  }
  .zones {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }
  .zone {
    position: absolute;
    overflow: clip;
    border: 1px solid;
    border-radius: 0.25rem;
  }
  .zone-name {
    display: block;
    inline-size: fit-content;
    max-inline-size: calc(100% - 0.5rem);
    margin: 0.3rem;
    padding: 0.1rem 0.25rem;
    border-radius: 0.2rem;
    overflow: clip;
    text-overflow: ellipsis;
    white-space: nowrap;
    background: oklch(0 0 none / 0.8);
    color: oklch(1 0 none);
    font-size: 0.58rem;
    line-height: 1.25;
    font-weight: 700;
  }
  .orientation {
    position: absolute;
    inset-inline-end: 0.35rem;
    inset-block-end: 0.35rem;
    display: grid;
    place-items: center;
    inline-size: 44px;
    block-size: 48px;
    max-inline-size: calc(100% - 0.7rem);
    max-block-size: calc(100% - 0.7rem);
    background: oklch(0 0 none / 0.8);
    color: oklch(1 0 none);
    border-radius: 0.3rem;
    pointer-events: auto;
  }
  .orientation svg {
    inline-size: 36px;
    block-size: 40px;
    max-inline-size: 100%;
    max-block-size: 100%;
  }
  .edge {
    position: absolute;
    display: flex;
    pointer-events: none;
  }
  .edge-top,
  .edge-bottom {
    inset-inline: 0;
    block-size: 8px;
  }
  .edge-top {
    inset-block-start: 0;
  }
  .edge-bottom {
    inset-block-end: 0;
  }
  .edge-left,
  .edge-right {
    inset-block: 0;
    inline-size: 8px;
    flex-direction: column;
  }
  .edge-left {
    inset-inline-start: 0;
  }
  .edge-right {
    inset-inline-end: 0;
  }
  .edge span {
    flex: 1;
  }
  .preview-actions {
    flex-wrap: wrap;
  }
  .preview-actions :global(button) {
    flex: 1;
    min-inline-size: 8rem;
  }
</style>
