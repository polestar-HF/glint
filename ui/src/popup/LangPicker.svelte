<script lang="ts">
  import Icon from '../lib/Icon.svelte';
  import { t } from '../lib/i18n.svelte';
  import { LANGS, SOURCE_LANGS, langName } from '../lib/langs';

  interface Props {
    source: string;
    target: string;
    detected: string | null;
    /** Where to translate to when a swap would otherwise produce X → X (the configured target). */
    fallback: string;
    onchange: (source: string, target: string) => void;
    /** Reports how tall the window must be while the menu is open (0 when closed). */
    onneed: (px: number) => void;
  }
  let { source, target, detected, fallback, onchange, onneed }: Props = $props();

  let open = $state<'source' | 'target' | null>(null);
  let menuEl = $state<HTMLDivElement>();
  let anchorEls: Record<'source' | 'target', HTMLButtonElement | undefined> = $state({ source: undefined, target: undefined });

  const items = $derived(open === 'source' ? SOURCE_LANGS : LANGS);
  const current = $derived(open === 'source' ? source : target);
  const canSwap = $derived(source !== 'auto' || !!detected);

  function toggle(which: 'source' | 'target') {
    open = open === which ? null : which;
  }

  function close(refocus = true) {
    const which = open;
    open = null;
    onneed(0);
    if (refocus && which) anchorEls[which]?.focus();
  }

  function pick(code: string) {
    const which = open;
    close();
    if (which === 'source' && code !== source) {
      // Picking the target language as source swaps the pair instead of producing en → en.
      const prev = source === 'auto' ? detected : source;
      onchange(code, code === target ? (prev && prev !== code ? prev : fallback) : target);
    } else if (which === 'target' && code !== target) {
      onchange(code === source ? target : source, code);
    }
  }

  function swap() {
    if (!canSwap) return;
    const from = source === 'auto' ? detected! : source;
    // Detected language already equals the target: swapping would give X → X.
    onchange(target, from === target ? fallback : from);
  }

  $effect(() => {
    if (!open || !menuEl) return;
    const el = menuEl;
    onneed(el.offsetTop + el.offsetHeight + 8);
    (el.querySelector('[aria-selected="true"]') as HTMLElement | null)?.focus();
    const away = (e: PointerEvent) => {
      const target = e.target as Node;
      if (!el.contains(target) && !anchorEls.source?.contains(target) && !anchorEls.target?.contains(target)) close(false);
    };
    const blur = () => close(false);
    window.addEventListener('pointerdown', away, true);
    window.addEventListener('blur', blur);
    return () => {
      window.removeEventListener('pointerdown', away, true);
      window.removeEventListener('blur', blur);
    };
  });

  function onMenuKey(e: KeyboardEvent) {
    const cols = 3;
    const list = Array.from(menuEl!.querySelectorAll<HTMLElement>('[role="option"]'));
    const i = list.indexOf(document.activeElement as HTMLElement);
    let next = -1;
    switch (e.key) {
      case 'Escape':
        e.preventDefault();
        e.stopPropagation();
        close();
        return;
      case 'ArrowRight':
        next = i + 1;
        break;
      case 'ArrowLeft':
        next = i - 1;
        break;
      case 'ArrowDown':
        next = i + cols;
        break;
      case 'ArrowUp':
        next = i - cols;
        break;
      case 'Home':
        next = 0;
        break;
      case 'End':
        next = list.length - 1;
        break;
      case 'Tab':
        close(false);
        return;
      default:
        return;
    }
    e.preventDefault();
    list[Math.max(0, Math.min(list.length - 1, next))]?.focus();
  }

  const sourceLabel = $derived(source === 'auto' ? (detected ? langName(detected, true) : langName('auto')) : langName(source, true));
</script>

<div class="langs">
  <button
    class="lang-btn"
    bind:this={anchorEls.source}
    aria-haspopup="listbox"
    aria-expanded={open === 'source'}
    title={t('popup.from')}
    onclick={() => toggle('source')}
  >
    <span class="lang-name">{sourceLabel}</span>
    {#if source === 'auto' && detected}<span class="lang-auto">{t('popup.auto')}</span>{/if}
    <Icon name="chevron-small" />
  </button>
  <button class="icon-btn swap" title={t('popup.swap')} aria-label={t('popup.swap')} disabled={!canSwap} onclick={swap}>
    <Icon name="swap" />
  </button>
  <button
    class="lang-btn"
    bind:this={anchorEls.target}
    aria-haspopup="listbox"
    aria-expanded={open === 'target'}
    title={t('popup.to')}
    onclick={() => toggle('target')}
  >
    <span class="lang-name">{langName(target, true)}</span>
    <Icon name="chevron-small" />
  </button>
</div>

{#if open}
  <!-- svelte-ignore a11y_interactive_supports_focus -->
  <div
    class="lang-menu"
    bind:this={menuEl}
    role="listbox"
    aria-label={open === 'source' ? t('popup.from') : t('popup.to')}
    onkeydown={onMenuKey}
  >
    <div class="lang-menu-title">{open === 'source' ? t('popup.from') : t('popup.to')}</div>
    <div class="lang-grid">
      {#each items as code (code)}
        <button
          class="lang-item"
          role="option"
          aria-selected={code === current}
          tabindex={code === current ? 0 : -1}
          onclick={() => pick(code)}
        >
          {langName(code, true)}
        </button>
      {/each}
    </div>
  </div>
{/if}
