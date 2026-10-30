<script lang="ts">
  import { nearestStop } from './settings';
  import { vibrate } from './haptics';
  let {
    label,
    value,
    stops,
    format,
    help,
    disabled = false,
    onchange,
    onhelp,
    vertical = true,
  }: {
    label: string;
    value: number;
    stops: number[];
    format: (value: number) => string;
    help: string;
    disabled?: boolean;
    onchange: (value: number) => void;
    onhelp: () => void;
    vertical?: boolean;
  } = $props();
  let index = $derived(nearestStop(value, stops));
</script>

<label class={['fader', !vertical && 'horizontal']}>
  <span class="fader-label">{label}</span><output>{format(value)}</output>
  <input
    type="range"
    aria-label={label}
    aria-valuetext={format(value)}
    aria-orientation={vertical ? 'vertical' : 'horizontal'}
    min="0"
    max={stops.length - 1}
    step="1"
    value={index}
    {disabled}
    title={help}
    onfocus={onhelp}
    onpointerdown={onhelp}
    oninput={(event) => {
      onhelp();
      vibrate();
      onchange(stops[Number(event.currentTarget.value)]);
    }}
  />
</label>
