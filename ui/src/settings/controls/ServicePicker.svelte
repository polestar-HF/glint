<script lang="ts">
  import Icon from '../../lib/Icon.svelte';
  import { t } from '../../lib/i18n.svelte';
  import { PRESETS, hostOf, presetLabel, protocolLabel, type Preset } from '../../lib/engines';

  let { onpick }: { onpick: (p: Preset) => void } = $props();

  const groups = [
    { kind: 'openai', items: PRESETS.filter((p) => p.kind === 'openai') },
    { kind: 'anthropic', items: PRESETS.filter((p) => p.kind === 'anthropic') },
  ];

  let open = $state(false);
  let btn = $state<HTMLButtonElement>();
  let menu = $state<HTMLDivElement>();
  let pos = $state<{ left: number; top?: number; bottom?: number; maxH: number }>({ left: 0, maxH: 400 });

  function desc(p: Preset): string {
    if (p.key === 'none') return t('preset.local');
    if (p.custom) return p.kind === 'openai' ? t('preset.customDesc') : '';
    return hostOf(p.baseUrl) || (p.kind === 'anthropic' ? 'api.anthropic.com' : '');
  }

  function show() {
    if (!btn) return;
    const r = btn.getBoundingClientRect();
    const below = window.innerHeight - r.bottom - 12;
    const above = r.top - 12;
    // Prefer opening downward; the button usually sits low on the page, so flip when cramped.
    pos =
      below >= 320 || below >= above
        ? { left: r.left, top: r.bottom + 4, maxH: below }
        : { left: r.left, bottom: window.innerHeight - r.top + 4, maxH: above };
    open = true;
    requestAnimationFrame(() => menu?.querySelector<HTMLElement>('[role="menuitem"]')?.focus());
  }

  function close(refocus = true) {
    open = false;
    if (refocus) btn?.focus();
  }

  function pick(p: Preset) {
    close(false);
    onpick(p);
  }

  function onkey(e: KeyboardEvent) {
    const items = Array.from(menu?.querySelectorAll<HTMLElement>('[role="menuitem"]') ?? []);
    const i = items.indexOf(document.activeElement as HTMLElement);
    let n = -1;
    if (e.key === 'ArrowDown') n = Math.min(items.length - 1, i + 1);
    else if (e.key === 'ArrowUp') n = Math.max(0, i - 1);
    else if (e.key === 'Home') n = 0;
    else if (e.key === 'End') n = items.length - 1;
    else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      return close();
    } else if (e.key === 'Tab') return close(false);
    else return;
    e.preventDefault();
    items[n]?.focus();
  }

  $effect(() => {
    if (!open) return;
    const away = (e: Event) => {
      const n = e.target as Node;
      if (!menu?.contains(n) && !btn?.contains(n)) close(false);
    };
    const scroll = (e: Event) => {
      if (!menu?.contains(e.target as Node)) close(false);
    };
    const shut = () => close(false);
    window.addEventListener('pointerdown', away, true);
    window.addEventListener('scroll', scroll, true);
    window.addEventListener('resize', shut);
    window.addEventListener('blur', shut);
    return () => {
      window.removeEventListener('pointerdown', away, true);
      window.removeEventListener('scroll', scroll, true);
      window.removeEventListener('resize', shut);
      window.removeEventListener('blur', shut);
    };
  });
</script>

<button
  class="add"
  bind:this={btn}
  aria-haspopup="menu"
  aria-expanded={open}
  onclick={() => (open ? close() : show())}
>
  <span class="add-icon"><Icon name="plus" /></span>
  <span>{t('tr.addService')}</span>
</button>

{#if open}
  <!-- svelte-ignore a11y_interactive_supports_focus -->
  <div
    class="menu"
    class:up={pos.bottom != null}
    role="menu"
    aria-label={t('tr.addService')}
    bind:this={menu}
    style:left="{pos.left}px"
    style:top={pos.top != null ? `${pos.top}px` : undefined}
    style:bottom={pos.bottom != null ? `${pos.bottom}px` : undefined}
    style:max-height="{pos.maxH}px"
    onkeydown={onkey}
  >
    {#each groups as g, gi (g.kind)}
      {#if gi > 0}<div class="sep" role="separator"></div>{/if}
      <div class="cap" role="presentation">{protocolLabel(g.kind)}</div>
      {#each g.items as p (p.id)}
        <button class="item" role="menuitem" tabindex="-1" onclick={() => pick(p)}>
          <span class="name">{presetLabel(p)}</span>
          <span class="desc">{desc(p)}</span>
        </button>
      {/each}
    {/each}
  </div>
{/if}

<style>
  /* The plus sits in the grip column and the label on the engine-name edge, so the
     button reads as "a new row goes here". */
  .add {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 32px;
    padding: 0 12px 0 0;
    border-radius: var(--r-sm);
    color: var(--accent);
    transition: background-color var(--dur-1) var(--ease-std);
  }
  .add:hover,
  .add[aria-expanded='true'] {
    background: var(--bg-subtle-hover);
  }
  .add:active {
    background: var(--bg-subtle-pressed);
  }
  .add-icon {
    display: grid;
    place-items: center;
    width: 24px;
  }

  .menu {
    position: fixed;
    z-index: 50;
    width: 320px;
    padding: 4px;
    overflow-y: auto;
    border-radius: var(--r-lg);
    border: 1px solid var(--stroke-flyout);
    background: var(--bg-flyout);
    box-shadow: var(--shadow-flyout);
    animation: menu-in var(--dur-2) var(--ease-out);
  }
  .cap {
    padding: 8px 12px 4px;
    font-family: var(--font-small);
    font-size: var(--fs-caption);
    line-height: 16px;
    color: var(--text-3);
  }
  .sep {
    height: 1px;
    margin: 4px -4px;
    background: var(--stroke-divider);
  }
  .item {
    display: flex;
    align-items: baseline;
    gap: 12px;
    width: 100%;
    height: 32px;
    padding: 6px 12px;
    border-radius: var(--r-sm);
    color: var(--text-1);
    transition: background-color var(--dur-1) var(--ease-std);
  }
  .item:hover {
    background: var(--bg-subtle-hover);
  }
  .item:focus-visible {
    outline-offset: -2px;
    background: var(--bg-subtle-hover);
  }
  .name {
    flex: none;
    white-space: nowrap;
  }
  .desc {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: right;
    font-family: var(--font-small);
    font-size: var(--fs-caption);
    color: var(--text-3);
  }
  .menu.up {
    animation-name: menu-up;
  }
  @keyframes menu-in {
    from {
      opacity: 0;
      transform: translateY(-4px);
    }
  }
  @keyframes menu-up {
    from {
      opacity: 0;
      transform: translateY(4px);
    }
  }
</style>
