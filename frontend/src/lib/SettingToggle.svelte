<script lang="ts">
  import * as Item from './components/ui/item';
  import { Switch } from './components/ui/switch';

  let {
    title,
    description,
    checked,
    disabled = false,
    compact = false,
    onchange,
  }: {
    title: string;
    description?: string;
    checked: boolean;
    disabled?: boolean;
    compact?: boolean;
    onchange: (checked: boolean) => void;
  } = $props();
  const id = $props.id();
</script>

<Item.Root class="setting-item" data-compact={compact}>
  <Item.Content>
    <Item.Title><label for={id}>{title}</label></Item.Title>
    {#if description}<Item.Description id={`${id}-description`}>{description}</Item.Description
      >{/if}
  </Item.Content>
  <Item.Actions>
    <Switch
      {id}
      {disabled}
      aria-describedby={description ? `${id}-description` : undefined}
      bind:checked={() => checked, onchange}
    />
  </Item.Actions>
</Item.Root>
