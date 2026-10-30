<script lang="ts">
  import { icons, type IconName, type IconDef } from './icons';

  /** `solid` switches to the filled variant: use it for "on" states (pinned, current page). */
  let { name, size = 16, solid = false }: { name: IconName; size?: number; solid?: boolean } = $props();
  const def = $derived(icons[name] as IconDef);
  const parts = $derived(solid ? (def.fill ?? def.duotone) : def.duotone);
  const box = $derived.by(() => {
    const i = def.inset ?? 0;
    return `${-i} ${-i} ${48 + 2 * i} ${48 + 2 * i}`;
  });
</script>

<svg class="icon" width={size} height={size} viewBox={box} fill="currentColor" aria-hidden="true" focusable="false">
  {#each parts as p, i (i)}
    <path
      d={p.d}
      class:tone={p.t}
      fill-rule={p.e ? 'evenodd' : undefined}
      clip-rule={p.e ? 'evenodd' : undefined}
    />
  {/each}
</svg>

<style>
  .icon {
    display: block;
    flex: none;
  }
  /* Duotone back layer: the same color as the glyph, lighter. */
  .tone {
    opacity: var(--icon-tone);
  }
</style>
