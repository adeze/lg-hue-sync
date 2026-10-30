<script lang="ts">
  import CaptureLightingGauge from './CaptureLightingGauge.svelte';
  import {
    diagnosticRate,
    diagnosticValue,
    snapshotState,
    type DiagnosticStatus,
  } from './diagnostics-types';

  let {
    status,
    receivedAt = 0,
    now = Date.now(),
    online = true,
  }: {
    status: DiagnosticStatus | null;
    receivedAt?: number;
    now?: number;
    online?: boolean;
  } = $props();

  const unavailable = 'Unavailable';
  const size = (width?: number | null, height?: number | null) =>
    width != null && height != null ? `${width} × ${height}` : unavailable;
  const dynamicRange = (hdr?: string | null) => (hdr === 'none' ? 'SDR' : diagnosticValue(hdr));

  let snapshot = $derived(status?.diagnostics);
  let freshness = $derived(snapshotState(snapshot, now));
  let snapshotFresh = $derived(freshness.available && !freshness.stale);
  let display = $derived(
    snapshotFresh && snapshot?.display?.available ? snapshot.display : undefined,
  );
  let video = $derived(snapshotFresh && snapshot?.video?.available ? snapshot.video : undefined);
  let player = $derived(snapshotFresh && snapshot?.player?.available ? snapshot.player : undefined);
  let output = $derived(snapshotFresh && snapshot?.output?.available ? snapshot.output : undefined);
  let freshnessMessage = $derived(
    !freshness.available
      ? 'Signal information unavailable.'
      : freshness.stale
        ? 'Signal snapshot is stale. Waiting for fresh readings.'
        : `Signal snapshot updated ${Math.floor(freshness.age / 1000)} seconds ago.`,
  );

  const sectionState = (available: boolean) =>
    freshness.stale ? 'Stale' : available ? 'Available' : unavailable;
  let cards = $derived([
    {
      id: 'display',
      title: 'Display output',
      state: sectionState(!!display),
      rows: [
        ['Output refresh rate', diagnosticRate(display?.refresh_rate)],
        ['Video plane size', size(display?.width, display?.height)],
        ['Video source', diagnosticValue(display?.source)],
        ['Display enabled', diagnosticValue(display?.enabled)],
        ['Display muted', diagnosticValue(display?.muted)],
        ['Timing locked', diagnosticValue(display?.locked)],
      ],
      note: 'Refresh rate is reported by the display driver, independently of content frame rate. Video plane size describes placement after scaling, not measured panel resolution.',
    },
    {
      id: 'video',
      title: 'HDMI video',
      state: sectionState(!!video),
      rows: [
        ['Input', diagnosticValue(video?.input)],
        ['Resolution', size(video?.width, video?.height)],
        ['Frame rate', diagnosticRate(video?.frame_rate)],
        ['Scan', diagnosticValue(video?.scan_type)],
        ['Dynamic range', dynamicRange(video?.hdr)],
        ['Pixel format', diagnosticValue(video?.pixel_encoding)],
        ['Signal range', diagnosticValue(video?.range)],
        ['Colourimetry', diagnosticValue(video?.colourimetry)],
        ['Bit depth', diagnosticValue(video?.bit_depth, ' bit')],
      ],
    },
    {
      id: 'player',
      title: 'Internal video player',
      state: sectionState(!!player),
      rows: [
        ['Decoded video size', size(player?.width, player?.height)],
        ['Content frame rate', diagnosticRate(player?.frame_rate)],
        ['Scan', diagnosticValue(player?.scan_type)],
        ['Dynamic range', dynamicRange(player?.hdr)],
      ],
      note: 'Shown when the main video plane uses the TV decoder. Other app rendering paths may not expose these details.',
    },
    {
      id: 'output',
      title: 'eARC / audio output',
      state: sectionState(!!output),
      rows: [
        ['Return channel', diagnosticValue(output?.arc_state)],
        ['Passthrough', diagnosticValue(output?.passthrough)],
      ],
      note: 'Output status is separate from incoming audio and receiver capabilities.',
    },
  ]);
</script>

<CaptureLightingGauge {status} {receivedAt} {now} {online} />
<div class="diagnostics-heading">
  <h2>Input &amp; output diagnostics</h2>
  <p>{freshnessMessage}</p>
</div>
<div class="diagnostics-grid">
  {#each cards as card (card.id)}
    <section class="card diagnostic-card" aria-labelledby={`diagnostic-${card.id}`}>
      <div class="diagnostic-card-header">
        <h3 id={`diagnostic-${card.id}`}>{card.title}</h3>
        <span
          class="diagnostic-state"
          data-state={card.state === 'Available' || card.state === 'Live'
            ? 'available'
            : card.state.toLowerCase()}>{card.state}</span
        >
      </div>
      <dl class="diagnostic-list">
        {#each card.rows as [label, value] (label)}
          <div>
            <dt>{label}</dt>
            <dd>{value}</dd>
          </div>
        {/each}
      </dl>
      {#if card.note}<p class="diagnostic-note">{card.note}</p>{/if}
    </section>
  {/each}
</div>

<style>
  .diagnostic-list dd {
    font-variant-numeric: tabular-nums;
  }
</style>
