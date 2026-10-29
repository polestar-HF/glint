<script lang="ts" generics="T extends string | number">
  import Icon from '../../lib/Icon.svelte';
  import { viewportRect } from '../../lib/viewport';

  interface Props {
    value: T;
    options: Array<{ value: T; label: string }>;
    onchange: (v: T) => void;
    label?: string;
    labelledby?: string;
    width?: number;
  }
  let { value, options, onchange, label, labelledby, width = 200 }: Props = $props();

  const ITEM = 32;
  const PAD = 4;

  let open = $state(false);
  let active = $state(0);
  let btn = $state<HTMLButtonElement>();
  let list = $state<HTMLDivElement>();
  let pos = $state({ top: 0, left: 0, width: 0, maxH: 360 });
  const uid = `sel-${Math.random().toString(36).slice(2, 8)}`;

  const index = $derived(Math.max(0, options.findIndex((o) => o.value === value)));
  const current = $derived(options[index]);

  function show() {
    if (!btn) return;
    const r = btn.getBoundingClientRect();
    const vp = viewportRect();
    const maxH = Math.min(360, vp.bottom - vp.top - 16);
    const fullH = options.length * ITEM + PAD * 2;
    const h = Math.min(fullH, maxH);
    // Like a Fluent ComboBox: open with the selected item lying over the button.
    const ideal = r.top - (PAD + index * ITEM);
    const top = Math.max(vp.top + 8, Math.min(ideal, vp.bottom - 8 - h));
    pos = { top, left: r.left - 4, width: r.width + 8, maxH };
    active = index;
    open = true;
    requestAnimationFrame(() => {
      if (!list) return;
      list.scrollTop = Math.max(0, PAD + index * ITEM - (r.top - top));
    });
  }

  function close() {
    open = false;
  }

  function choose(i: number) {
    const o = options[i];
    close();
    btn?.focus();
    if (o && o.value !== value) onchange(o.value);
  }

  function onkey(e: KeyboardEvent) {
    const last = options.length - 1;
    if (!open) {
      if (e.key === 'Enter' || e.key === ' ' || (e.altKey && e.key === 'ArrowDown') || e.key === 'F4') {
        e.preventDefault();
        show();
      } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        choose(Math.max(0, Math.min(last, index + (e.key === 'ArrowDown' ? 1 : -1))));
      }
      return;
    }
    switch (e.key) {
      case 'ArrowDown':
        active = Math.min(last, active + 1);
        break;
      case 'ArrowUp':
        active = Math.max(0, active - 1);
        break;
      case 'Home':
        active = 0;
        break;
      case 'End':
        active = last;
        break;
      case 'Enter':
      case ' ':
        choose(active);
        break;
      case 'Escape':
        e.stopPropagation();
        close();
        break;
      case 'Tab':
        close();
        return;
      default:
        return;
    }
    e.preventDefault();
    list?.querySelectorAll('[role="option"]')[active]?.scrollIntoView({ block: 'nearest' });
  }

  $effect(() => {
    if (!open) return;
    const away = (e: Event) => {
      const t = e.target as Node;
      if (!list?.contains(t) && !btn?.contains(t)) close();
    };
    // A click on a half-visible anchor scrolls it into view first; that scroll arrives just
    // after opening and must not close the flyout again.
    const openedAt = performance.now();
    const scroll = (e: Event) => {
      if (performance.now() - openedAt < 200) return;
      if (!list?.contains(e.target as Node)) close();
    };
    window.addEventListener('pointerdown', away, true);
    window.addEventListener('scroll', scroll, true);
    window.addEventListener('resize', close);
    window.addEventListener('blur', close);
    return () => {
      window.removeEventListener('pointerdown', away, true);
      window.removeEventListener('scroll', scroll, true);
      window.removeEventListener('resize', close);
      window.removeEventListener('blur', close);
    };
  });
</script>

<button
  class="select"
  role="combobox"
  bind:this={btn}
  style:width="{width}px"
  aria-haspopup="listbox"
  aria-expanded={open}
  aria-controls={open ? uid : undefined}
  aria-activedescendant={open ? `${uid}-${active}` : undefined}
  aria-label={label}
  aria-labelledby={labelledby}
  onclick={() => (open ? close() : show())}
  onkeydown={onkey}
>
  <span class="value">{current?.label ?? ''}</span>
  <Icon name="chevron-down" />
</button>

{#if open}
  <div
    class="list"
    id={uid}
    role="listbox"
    tabindex="-1"
    bind:this={list}
    style:top="{pos.top}px"
    style:left="{pos.left}px"
    style:min-width="{pos.width}px"
    style:max-height="{pos.maxH}px"
  >
    {#each options as o, i (o.value)}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div
        class="opt"
        id="{uid}-{i}"
        role="option"
        tabindex="-1"
        aria-selected={i === index}
        class:active={i === active}
        onpointermove={() => (active = i)}
        onclick={() => choose(i)}
      >
        {o.label}
      </div>
    {/each}
  </div>
{/if}

<style>
  .select {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    height: 32px;
    padding: 0 8px 0 12px;
    border-radius: var(--r-sm);
    border: 1px solid var(--stroke-control);
    border-bottom-color: var(--stroke-control-bottom);
    background: var(--bg-control) padding-box;
    color: var(--text-1);
    transition: background-color var(--dur-1) var(--ease-std);
  }
  .select:hover {
    background-color: var(--bg-control-hover);
  }
  .select:active,
  .select[aria-expanded='true'] {
    background-color: var(--bg-control-pressed);
  }
  .value {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: left;
  }
  .select :global(.icon) {
    color: var(--text-2);
  }

  .list {
    position: fixed;
    z-index: 50;
    padding: 4px;
    overflow-y: auto;
    border-radius: var(--r-lg);
    border: 1px solid var(--stroke-flyout);
    background: var(--bg-flyout);
    box-shadow: var(--shadow-flyout);
    animation: list-in var(--dur-1) var(--ease-out);
  }
  .opt {
    position: relative;
    display: flex;
    align-items: center;
    height: 32px;
    padding: 0 12px;
    border-radius: var(--r-sm);
    white-space: nowrap;
    color: var(--text-1);
  }
  .opt.active {
    background: var(--bg-subtle-hover);
  }
  .opt[aria-selected='true'] {
    background: var(--bg-subtle-hover);
  }
  .opt[aria-selected='true']::before {
    content: '';
    position: absolute;
    left: 0;
    top: 8px;
    bottom: 8px;
    width: 3px;
    border-radius: 2px;
    background: var(--accent);
  }
  @keyframes list-in {
    from {
      opacity: 0;
    }
  }
</style>
