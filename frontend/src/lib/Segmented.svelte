<script lang="ts">
  import { Button } from './components/ui/button';
  let {
    label,
    options,
    value,
    disabled = false,
    onchange,
    onhelp,
  }: {
    label: string;
    options: { value: string; label: string; help: string }[];
    value: string;
    disabled?: boolean;
    onchange: (value: string) => void;
    onhelp: (help: string) => void;
  } = $props();
  let selected = $derived(options.findIndex((option) => option.value === value));
</script>

<fieldset class="preset-group">
  <legend>{label}</legend>
  <div
    class="segmented"
    style:--selected={Math.max(0, selected)}
    style:--selected-opacity={selected < 0 ? 0 : 1}
    role="group"
    aria-label={label}
  >
    {#each options as option (option.value)}<Button
        variant="ghost"
        class="segment"
        aria-pressed={option.value === value}
        {disabled}
        onfocus={() => onhelp(option.help)}
        onclick={() => {
          onhelp(option.help);
          onchange(option.value);
        }}>{option.label}</Button
      >{/each}
  </div>
</fieldset>
