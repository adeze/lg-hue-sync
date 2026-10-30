<script lang="ts">
  import { captureLightingState, rateScale } from './capture-lighting';
  import { diagnosticValue, type DiagnosticStatus } from './diagnostics-types';

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
  let reading = $derived(captureLightingState(status, receivedAt, now, online));
  const number = (value: number | null) => (value === null ? 'Unavailable' : String(value));
</script>

<section class="card capture-lighting" aria-labelledby="captureLightingTitle">
  <div class="gauge-heading">
    <h2 id="captureLightingTitle">Capture &amp; lights</h2>
    <span class="phase" data-active={reading.running}>{reading.phase}</span>
  </div>
  <div class="gauge-body">
    <div class="rates">
      <svg viewBox="0 0 264 148" aria-hidden="true">
        <path class="track" d="M 26 132 A 106 106 0 0 1 238 132" />
        <path class="track" d="M 52 132 A 80 80 0 0 1 212 132" />
        {#if reading.captureArc > 0}<path
            class="capture-arc"
            d="M 26 132 A 106 106 0 0 1 238 132"
            pathLength="100"
            stroke-dasharray={`${reading.captureArc} 100`}
          />{/if}
        {#if reading.lightsArc > 0}<path
            class="lights-arc"
            d="M 52 132 A 80 80 0 0 1 212 132"
            pathLength="100"
            stroke-dasharray={`${reading.lightsArc} 100`}
          />{/if}
        <text x="132" y="107" text-anchor="middle">0–{rateScale} /s</text>
      </svg>
      <dl class="rate-labels">
        <div class={{ 'live-capture': reading.running && reading.captureCeiling !== null }}>
          <dt><span class="key capture-key" aria-hidden="true"></span>Capture ceiling</dt>
          <dd>
            <strong>{number(reading.captureCeiling)}</strong
            >{#if reading.captureCeiling !== null}<span>Hz max</span>{/if}
          </dd>
        </div>
        <div class={{ 'live-lights': reading.running && reading.lightUpdates !== null }}>
          <dt><span class="key lights-key" aria-hidden="true"></span>Light updates</dt>
          <dd>
            <strong>{number(reading.lightUpdates)}</strong>{#if reading.lightUpdates !== null}<span
                >updates/s</span
              >{/if}
          </dd>
        </div>
      </dl>
    </div>
    <div class="engine-details">
      <div class="senders" aria-label="Lighting sender connections">
        <p>
          <span
            class="dot"
            data-connected={reading.running && reading.hue === 'Connected'}
            aria-hidden="true"
          ></span>Hue sender <strong>{reading.hue}</strong>
        </p>
        <p>
          <span
            class="dot"
            data-connected={reading.running && reading.nanoleaf === 'Connected'}
            aria-hidden="true"
          ></span>Nanoleaf sender <strong>{reading.nanoleaf}</strong>
        </p>
      </div>
      <dl class="engine-metadata">
        <div>
          <dt>TV power</dt>
          <dd>{diagnosticValue(reading.fresh ? status?.tv_power_state : null)}</dd>
        </div>
        <div>
          <dt>Capture size</dt>
          <dd>{diagnosticValue(reading.fresh ? status?.capture_resolution : null)}</dd>
        </div>
        <div>
          <dt>Hardware capture</dt>
          <dd>{diagnosticValue(reading.fresh ? status?.capture_hardware : null)}</dd>
        </div>
      </dl>
      {#if reading.fresh && status?.last_error}<p class="engine-error" role="status">
          {status.last_error}
        </p>{/if}
      {#if !reading.fresh}<p class="engine-note">
          Waiting for fresh daemon status.
        </p>{:else if !reading.running}<p class="engine-note">
          Capture ceiling is configured; sync is not running.
        </p>{/if}
    </div>
  </div>
  <p class="gauge-note">
    Capture ceiling is a polling limit, not video frame rate. Light updates are measured separately.
    Arcs share a 0–{rateScale} /s scale; labels retain exact values. Sender connections do not confirm
    physical light output.
  </p>
</section>

<style>
  .capture-lighting {
    padding: clamp(1.1rem, 3vw, 1.75rem);
  }
  .gauge-heading {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 0.5rem 1rem;
  }
  h2 {
    font-size: 1rem;
  }
  .phase {
    color: var(--text-muted);
    font-size: 0.8rem;
    text-transform: capitalize;
  }
  .phase[data-active='true'] {
    color: var(--success);
  }
  .gauge-body {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 16rem), 1fr));
    align-items: center;
    gap: 1.25rem;
    padding-block: 0.75rem 1rem;
  }
  .rates {
    min-inline-size: 0;
  }
  svg {
    display: block;
    inline-size: min(100%, 20rem);
    margin-inline: auto;
    overflow: visible;
  }
  path {
    fill: none;
    stroke-width: 10;
    stroke-linecap: round;
  }
  .track {
    stroke: var(--card-border);
  }
  .capture-arc {
    stroke: var(--focus-ring);
  }
  .lights-arc {
    stroke: var(--success);
  }
  svg text {
    fill: var(--text-muted);
    font:
      11px ui-sans-serif,
      system-ui,
      sans-serif;
  }
  .rate-labels {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 0.75rem;
  }
  .rate-labels > div {
    color: var(--text-muted);
  }
  .rate-labels dt {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.75rem;
  }
  .key {
    display: inline-block;
    inline-size: 0.6rem;
    block-size: 0.6rem;
    border-radius: 50%;
    background: var(--text-muted);
    flex: none;
  }
  .live-capture .capture-key {
    background: var(--focus-ring);
  }
  .live-lights .lights-key {
    background: var(--success);
  }
  dd {
    margin: 0;
    font-variant-numeric: tabular-nums;
  }
  .rate-labels dd {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 0.35rem;
    margin-block-start: 0.3rem;
  }
  .rate-labels strong {
    font-size: clamp(1.1rem, 3vw, 1.7rem);
    font-weight: 650;
    overflow-wrap: anywhere;
  }
  .rate-labels dd span {
    font-size: 0.72rem;
  }
  .live-capture strong {
    color: var(--accent);
  }
  .live-lights strong {
    color: var(--success);
  }
  .engine-details {
    display: grid;
    gap: 1rem;
    min-inline-size: 0;
  }
  .senders {
    display: grid;
    gap: 0.6rem;
  }
  .senders p {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.4rem;
    font-size: 0.78rem;
  }
  .senders strong {
    font-weight: 500;
    margin-inline-start: auto;
    color: var(--text-muted);
  }
  .dot {
    inline-size: 0.5rem;
    block-size: 0.5rem;
    border-radius: 50%;
    background: var(--text-muted);
    flex: none;
  }
  .dot[data-connected='true'] {
    background: var(--success);
  }
  .engine-metadata {
    display: grid;
    gap: 0.4rem;
  }
  .engine-metadata > div {
    display: flex;
    justify-content: space-between;
    gap: 1rem;
    font-size: 0.78rem;
  }
  .engine-metadata dt {
    color: var(--text-muted);
  }
  .engine-metadata dd {
    text-align: end;
    overflow-wrap: anywhere;
  }
  .gauge-note,
  .engine-note {
    color: var(--text-muted);
    font-size: 0.72rem;
    line-height: 1.5;
  }
  .engine-error {
    font-size: 0.78rem;
    overflow-wrap: anywhere;
  }
  @media (forced-colors: active) {
    path {
      stroke: CanvasText;
    }
  }
</style>
