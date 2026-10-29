<script lang="ts">
  import { onMount, untrack, type Component } from 'svelte';
  import Icon from '../lib/Icon.svelte';
  import { inTauri } from '../lib/ipc';
  import { t, type MessageKey } from '../lib/i18n.svelte';
  import type { IconName } from '../lib/icons';
  import { flushNow, load, onConfigChanged, store } from './store.svelte';
  import General from './sections/General.svelte';
  import Hotkeys from './sections/Hotkeys.svelte';
  import Ocr from './sections/Ocr.svelte';
  import Translation from './sections/Translation.svelte';
  import Appearance from './sections/Appearance.svelte';
  import About from './sections/About.svelte';

  type Id = 'general' | 'hotkeys' | 'ocr' | 'translate' | 'appearance' | 'about';
  const sections: Array<{ id: Id; icon: IconName; label: MessageKey; comp: Component }> = [
    { id: 'general', icon: 'sliders', label: 'nav.general', comp: General },
    { id: 'hotkeys', icon: 'keyboard', label: 'nav.hotkeys', comp: Hotkeys },
    { id: 'ocr', icon: 'scan', label: 'nav.ocr', comp: Ocr },
    { id: 'translate', icon: 'translate', label: 'nav.translate', comp: Translation },
    { id: 'appearance', icon: 'contrast', label: 'nav.appearance', comp: Appearance },
    { id: 'about', icon: 'info', label: 'nav.about', comp: About },
  ];

  function initial(): Id {
    const q = new URLSearchParams(location.search).get('section');
    let saved: string | null = null;
    try {
      saved = sessionStorage.getItem('glint.section');
    } catch {
      /* ignore */
    }
    const pick = q ?? saved;
    return sections.some((s) => s.id === pick) ? (pick as Id) : 'general';
  }

  let current = $state<Id>(initial());
  let pageEl = $state<HTMLElement>();
  const section = $derived(sections.find((s) => s.id === current)!);
  const Current = $derived(section.comp);

  function go(id: Id) {
    current = id;
    pageEl?.scrollTo({ top: 0 });
    try {
      sessionStorage.setItem('glint.section', id);
    } catch {
      /* ignore */
    }
  }

  function navKey(e: KeyboardEvent) {
    const i = sections.findIndex((s) => s.id === current);
    let n = -1;
    if (e.key === 'ArrowDown') n = Math.min(sections.length - 1, i + 1);
    else if (e.key === 'ArrowUp') n = Math.max(0, i - 1);
    else return;
    e.preventDefault();
    go(sections[n].id);
    (document.querySelectorAll<HTMLElement>('.nav-item')[n])?.focus();
  }

  onMount(() => {
    load().catch((e) => console.error(e));
    window.addEventListener('beforeunload', flushNow);
    return () => window.removeEventListener('beforeunload', flushNow);
  });

  // Deep-track the config; every change is applied and saved (debounced).
  $effect(() => {
    if (!store.config) return;
    JSON.stringify(store.config);
    untrack(onConfigChanged);
  });

  $effect(() => {
    document.title = t('settings.title');
  });

  const saveText = $derived(
    store.save === 'saving'
      ? t('save.saving')
      : store.save === 'saved'
        ? t('save.saved')
        : store.save === 'failed'
          ? t('save.failed')
          : t('save.idle'),
  );
</script>

<div class="window">
  {#if !inTauri}
    <!-- Browser preview only: stands in for the native Windows 11 title bar. -->
    <div class="caption" aria-hidden="true">
      <span class="caption-title">{t('settings.title')}</span>
      <span class="caption-btns">
        <svg viewBox="0 0 10 10"><path d="M0 5.5h10" /></svg>
        <svg viewBox="0 0 10 10"><rect x="0.5" y="0.5" width="9" height="9" rx="1.5" /></svg>
        <svg viewBox="0 0 10 10"><path d="M0.5 0.5l9 9M9.5 0.5l-9 9" /></svg>
      </span>
    </div>
  {/if}

  <div class="settings">
    <nav class="nav" aria-label={t('settings.title')}>
      <div class="nav-list" role="tablist" aria-orientation="vertical" tabindex="-1" onkeydown={navKey}>
        {#each sections as s (s.id)}
          <button
            class="nav-item"
            role="tab"
            aria-selected={current === s.id}
            tabindex={current === s.id ? 0 : -1}
            onclick={() => go(s.id)}
          >
            <Icon name={s.icon} />
            <span>{t(s.label)}</span>
          </button>
        {/each}
      </div>
      <div class="nav-foot" class:failed={store.save === 'failed'} title={store.save === 'failed' ? store.error : undefined}>
        {#if store.save === 'saved'}<Icon name="check" />{:else if store.save === 'failed'}<Icon name="alert" />{/if}
        <span>{saveText}</span>
      </div>
    </nav>

    <main class="page" bind:this={pageEl} role="tabpanel">
      <h1 class="page-title">{t(section.label)}</h1>
      {#if store.config}
        <Current />
      {/if}
    </main>
  </div>
</div>
