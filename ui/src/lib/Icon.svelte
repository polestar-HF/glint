<script lang="ts">
  import { icons, type IconName, type IconDef } from './icons';

  let { name, size = 16 }: { name: IconName; size?: number } = $props();
  const def = $derived(icons[name] as IconDef);
</script>

<svg
  class="icon"
  width={size}
  height={size}
  viewBox="0 0 16 16"
  fill="none"
  stroke="currentColor"
  stroke-width="1.5"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
  focusable="false"
>
  <g transform={def.rotate ? `rotate(${def.rotate} 8 8)` : undefined}>
    {#each def.parts as p, i (i)}
      {#if p.f}
        <path d={p.d} fill="currentColor" stroke="none" />
      {:else}
        <path d={p.d} />
      {/if}
    {/each}
  </g>
</svg>

<style>
  .icon {
    display: block;
    flex: none;
  }
</style>
