<script lang="ts">
  import { Switch as SwitchPrimitive } from 'bits-ui';
  import { createAttachmentKey } from 'svelte/attachments';
  import { buttonHaptics } from '$lib/haptics';
  const haptics = { [createAttachmentKey()]: buttonHaptics };
  import { cn, type WithoutChildrenOrChild } from '$lib/utils.js';

  let {
    ref = $bindable(null),
    class: className,
    checked = $bindable(false),
    size = 'default',
    ...restProps
  }: WithoutChildrenOrChild<SwitchPrimitive.RootProps> & {
    size?: 'sm' | 'default';
  } = $props();
</script>

<SwitchPrimitive.Root
  bind:ref
  {...haptics}
  bind:checked
  data-slot="switch"
  data-size={size}
  class={cn(
    'data-[state=checked]:bg-primary data-[state=unchecked]:bg-input focus-visible:border-ring focus-visible:ring-ring/50 aria-invalid:ring-destructive/20 dark:aria-invalid:ring-destructive/40 aria-invalid:border-destructive dark:aria-invalid:border-destructive/50 dark:data-[state=unchecked]:bg-input/80 shrink-0 rounded-full border border-transparent focus-visible:ring-3 aria-invalid:ring-3 group-has-[:focus-visible]/field-label:ring-0 group-has-[:focus-visible]/field-label:border-transparent data-[size=default]:h-7 data-[size=default]:w-12 data-[size=sm]:h-[14px] data-[size=sm]:w-[24px] peer group/switch relative inline-flex items-center transition-colors outline-none after:absolute after:inset-x-0 after:-inset-y-2 data-disabled:cursor-not-allowed data-disabled:opacity-50',
    className,
  )}
  {...restProps}
>
  <SwitchPrimitive.Thumb
    data-slot="switch-thumb"
    class={cn(
      'bg-primary-foreground rounded-full group-data-[size=default]/switch:size-6 group-data-[size=sm]/switch:size-3 group-data-[size=default]/switch:data-[state=checked]:translate-x-5 group-data-[size=sm]/switch:data-[state=checked]:translate-x-[calc(100%_-_2px)] group-data-[size=default]/switch:data-[state=unchecked]:translate-x-0 group-data-[size=sm]/switch:data-[state=unchecked]:translate-x-0 pointer-events-none block ring-0 motion-safe:transition-transform',
      'rtl:data-[state=checked]:translate-x-[calc(-100%)]',
    )}
  />
</SwitchPrimitive.Root>
