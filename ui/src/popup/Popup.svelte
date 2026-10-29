<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '../lib/Icon.svelte';
  import { api, on, type Unlisten } from '../lib/ipc';
  import { fmtMs, i18n, setLocale, t } from '../lib/i18n.svelte';
  import { htmlLang, langName } from '../lib/langs';
  import { engineName, normalizeConfig } from '../lib/engines';
  import { applyUi } from '../lib/theme';
  import type { Config, EventMap, EventName, SessionSnapshot } from '../lib/types';
  import LangPicker from './LangPicker.svelte';
  import ResultSection from './ResultSection.svelte';
  import Skeleton from './Skeleton.svelte';
  import { Session, type ResultView } from './session.svelte';

  let config = $state<Config | null>(null);
  const s = new Session(() =>
    (config?.translate.engines ?? []).filter((e) => e.enabled).map((e) => ({ id: e.id, name: engineName(e) })),
  );

  let pinned = $state(false);
  let toast = $state({ text: '', shown: false });
  let toastTimer: ReturnType<typeof setTimeout> | undefined;
  let menuNeed = $state(0);

  let rootEl = $state<HTMLElement>();
  let stripEl = $state<HTMLElement>();
  let contentEl = $state<HTMLElement>();
  let footerEl = $state<HTMLElement>();

  // Target to fall back to when a swap would translate a language into itself.
  const langFallback = $derived.by(() => {
    const tr = config?.translate;
    if (!tr) return s.targetLang === 'en' ? 'zh-Hans' : 'en';
    return tr.targetLang !== s.targetLang ? tr.targetLang : tr.secondaryLang;
  });
  const contentLang = $derived(htmlLang(s.sourceLang === 'auto' ? s.detectedLang : s.sourceLang));
  const sourceMeta = $derived.by(() => {
    if (s.ocrStatus === 'pending') return t('popup.recognizing');
    if (s.ocrStatus !== 'done') return '';
    if (s.edited) return t('popup.edited');
    return [s.ocrEngine, fmtMs(s.ocrElapsed)].filter(Boolean).join(' · ');
  });
  // v0.2: one quiet line of timings, e.g.
  // 截图 21 ms · 选区 34 ms · 识别 142 ms（检测 48 / 识别 81）· 首字 610 ms. Absent parts are left out.
  const showTimings = $derived(!!config?.ui.showTimings);
  const timingParts = $derived.by(() => {
    if (!showTimings) return [];
    const out: string[] = [];
    const ct = s.captureTimings;
    if (ct?.captureMs != null) out.push(`${t('tm.capture')} ${fmtMs(ct.captureMs)}`);
    if (ct?.overlayMs != null) out.push(`${t('tm.overlay')} ${fmtMs(ct.overlayMs)}`);
    if (s.ocrStatus === 'done' && s.ocrElapsed != null) {
      const ot = s.ocrTimings;
      const inner: string[] = [];
      if (ot?.loadMs != null) inner.push(`${t('tm.load')} ${Math.round(ot.loadMs)}`);
      if (ot?.detMs != null) inner.push(`${t('tm.det')} ${Math.round(ot.detMs)}`);
      if (ot?.recMs != null) inner.push(`${t('tm.rec')} ${Math.round(ot.recMs)}`);
      const detail = inner.length ? (i18n.locale === 'zh-CN' ? `（${inner.join(' / ')}）` : ` (${inner.join(' / ')})`) : '';
      out.push(`${t('tm.ocr')} ${fmtMs(s.ocrElapsed)}${detail}`);
    }
    const first = s.mode === 'translate' ? s.results[0]?.firstTokenMs : null;
    if (first != null) out.push(`${t('tm.first')} ${fmtMs(first)}`);
    return out;
  });
  const ocrMeta = $derived.by(() => {
    if (s.ocrStatus === 'pending') return t('popup.recognizing');
    if (s.ocrStatus !== 'done' || !s.text) return '';
    const n = s.text.split('\n').filter((l) => l.trim()).length;
    return [s.ocrEngine, fmtMs(s.ocrElapsed), t('popup.lines', { n })].filter(Boolean).join(' · ');
  });

  // ── Config ────────────────────────────────────────────────────────────────
  function setConfig(c: Config) {
    config = normalizeConfig(c);
    setLocale(config.general.uiLanguage);
    applyUi(config);
  }

  // ── Events ────────────────────────────────────────────────────────────────
  let pruneTimer: ReturnType<typeof setTimeout> | undefined;

  function dispatch<K extends EventName>(name: K, payload: EventMap[K], replayOf?: SessionSnapshot | null) {
    const p = payload as never;
    switch (name) {
      case 'config:changed':
        return setConfig(p);
      case 'session:start':
        return s.start(p);
      case 'ocr:result':
        s.ocrResult(p);
        if (s.langChangedEarly && s.text.trim()) {
          s.langChangedEarly = false;
          void retranslate();
        }
        return;
      case 'ocr:error':
        return s.ocrError(p);
      case 'tr:start':
        s.trStart(p);
        clearTimeout(pruneTimer);
        pruneTimer = setTimeout(() => s.prune(), 600);
        return;
      case 'tr:delta': {
        const d = payload as EventMap['tr:delta'];
        // Replayed after a snapshot without sequence numbers: skip what the snapshot already holds.
        if (replayOf && d.seq == null && d.sessionId === replayOf.sessionId) {
          const r = s.results.find((x) => x.engineId === d.engineId);
          if (r && r.text.endsWith(d.delta)) return;
        }
        return s.trDelta(p);
      }
      case 'tr:done':
        return s.trDone(p);
      case 'tr:error':
        return s.trError(p);
    }
  }

  onMount(() => {
    let disposed = false;
    const subs: Unlisten[] = [];
    let buffer: Array<[EventName, unknown]> | null = [];
    const names: EventName[] = ['session:start', 'ocr:result', 'ocr:error', 'tr:start', 'tr:delta', 'tr:done', 'tr:error', 'config:changed'];

    (async () => {
      // Subscribe first, then catch up: nothing emitted in between is lost.
      const list = await Promise.all(
        names.map((n) => on(n, (p) => (buffer ? buffer.push([n, p]) : dispatch(n, p as never)))),
      );
      if (disposed) return list.forEach((u) => u());
      subs.push(...list);
      const [cfg, snap] = await Promise.all([
        api.getConfig().catch(() => null),
        api.getSessionSnapshot().catch(() => null),
      ]);
      if (cfg) setConfig(cfg);
      if (snap) s.fromSnapshot(snap);
      const queued = buffer ?? [];
      buffer = null;
      for (const [n, p] of queued) dispatch(n, p as never, snap);
    })();

    const ro = new ResizeObserver(() => scheduleReport());
    if (rootEl) ro.observe(rootEl);
    if (contentEl) ro.observe(contentEl);

    return () => {
      disposed = true;
      subs.forEach((u) => u());
      ro.disconnect();
    };
  });

  // ── Window sizing ─────────────────────────────────────────────────────────
  // Report the natural content height so the native window fits it, capped at 70 % of
  // the screen; beyond that the body scrolls. Coalesced to one call per frame, ≥ 50 ms apart.
  let lastH = 0;
  let lastSent = 0;
  let raf = 0;
  let trailing: ReturnType<typeof setTimeout> | undefined;

  function measure(): number {
    const natural =
      (stripEl?.offsetHeight ?? 36) + 1 + (contentEl?.offsetHeight ?? 0) + (footerEl?.offsetHeight ?? 0);
    const cap = Math.floor((screen.availHeight || 900) * 0.7);
    return Math.min(Math.ceil(Math.max(natural, menuNeed)), cap);
  }

  function scheduleReport() {
    if (raf) return;
    raf = requestAnimationFrame(() => {
      raf = 0;
      const wait = 50 - (performance.now() - lastSent);
      if (wait > 0) {
        clearTimeout(trailing);
        trailing = setTimeout(scheduleReport, wait);
        return;
      }
      const h = measure();
      if (h !== lastH) {
        lastH = h;
        lastSent = performance.now();
        api.popupResize(h).catch(() => {});
      }
    });
  }

  $effect(() => {
    void menuNeed;
    void s.mode;
    scheduleReport();
  });

  // ── Actions ───────────────────────────────────────────────────────────────
  function showToast(text: string) {
    toast.text = text;
    toast.shown = true;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast.shown = false), 1400);
  }

  async function copy(text: string) {
    if (!text.trim()) return;
    try {
      await api.copyText(text);
      showToast(t('popup.copied'));
    } catch (e) {
      showToast(String(e));
    }
  }

  function copyPrimary() {
    void copy(s.mode === 'ocr' ? s.text : (s.primary?.text ?? s.text));
  }

  function copyAll() {
    if (s.mode === 'ocr') return void copy(s.text);
    const parts = [s.text.trim(), ...s.results.filter((r) => r.text && r.status !== 'error').map((r) => r.text.trim())];
    void copy(parts.filter(Boolean).join('\n\n'));
  }

  function togglePin() {
    pinned = !pinned;
    api.popupSetPinned(pinned).catch(() => {});
  }

  async function retranslate() {
    if (s.ocrStatus !== 'done' || !s.text.trim()) return;
    s.expectRetranslate = { text: s.text };
    try {
      const id = await api.retranslate(s.text, s.sourceLang, s.targetLang);
      // If session:start was not emitted (or not yet), adopt the new id ourselves.
      if (id > s.id) s.start({ sessionId: id, mode: 'translate', sourceLang: s.sourceLang, targetLang: s.targetLang });
    } catch (e) {
      s.expectRetranslate = null;
      showToast(String(e));
    }
  }

  async function retry(r: ResultView) {
    r.status = 'pending';
    r.text = '';
    r.message = '';
    r.deltas = 0;
    r.firstTokenMs = null;
    try {
      await api.retranslateEngine(s.id, r.engineId, s.submittedText || s.text, s.sourceLang, s.targetLang);
    } catch {
      // Backend without per-engine retry: translate everything again.
      void retranslate();
    }
  }

  function changeLangs(source: string, target: string) {
    s.sourceLang = source;
    s.targetLang = target;
    if (s.ocrStatus === 'pending') s.langChangedEarly = true;
    else void retranslate();
  }

  function hasSelection(): boolean {
    const a = document.activeElement;
    if (a instanceof HTMLTextAreaElement || a instanceof HTMLInputElement) return a.selectionStart !== a.selectionEnd;
    return !!window.getSelection()?.toString();
  }

  function onKey(e: KeyboardEvent) {
    if (e.defaultPrevented || e.isComposing) return;
    const ctrl = e.ctrlKey && !e.altKey && !e.metaKey && !e.shiftKey;
    if (e.key === 'Escape') {
      e.preventDefault();
      api.popupHide().catch(() => {});
    } else if (ctrl && e.code === 'KeyP') {
      e.preventDefault();
      togglePin();
    } else if (ctrl && e.key === 'Enter') {
      e.preventDefault();
      void retranslate();
    } else if (ctrl && e.code === 'KeyC' && !hasSelection()) {
      e.preventDefault();
      copyPrimary();
    }
  }

  function recapture() {
    api.startCapture(s.mode).catch(() => {});
  }
</script>

<svelte:window onkeydown={onKey} />

{#snippet timingLine()}
  {#if timingParts.length}
    <div class="timings tnum">
      {#each timingParts as part, i (i)}
        {#if i}<span class="timings-sep" aria-hidden="true">·</span>{/if}<span>{part}</span>
      {/each}
    </div>
  {/if}
{/snippet}

{#snippet sourceBody(lines: number[])}
  {#if s.ocrStatus === 'pending'}
    <Skeleton widths={lines} />
  {:else if s.ocrStatus === 'error'}
    <div class="err" role="alert">
      <span class="err-icon"><Icon name="alert" /></span>
      <span class="err-msg" title={s.ocrMessage}>{t('popup.ocrFailed')}{s.ocrMessage ? `：${s.ocrMessage}` : ''}</span>
      <button class="text-btn" onclick={recapture}>{t('popup.recapture')}</button>
    </div>
  {:else if !s.ocrText.trim() && !s.text}
    <div class="err empty">
      <span class="err-msg">{t('popup.noText')}</span>
      <button class="text-btn" onclick={recapture}>{t('popup.recapture')}</button>
    </div>
  {:else}
    <textarea
      class="text src-text"
      bind:value={s.text}
      lang={contentLang}
      spellcheck="false"
      rows="1"
      aria-label={t('popup.source')}
    ></textarea>
  {/if}
{/snippet}

<div class="popup" class:ocr={s.mode === 'ocr'} class:stale={s.dirty} bind:this={rootEl}>
  <header class="strip" data-tauri-drag-region bind:this={stripEl}>
    {#if s.mode === 'translate'}
      <LangPicker
        source={s.sourceLang}
        target={s.targetLang}
        detected={s.detectedLang}
        fallback={langFallback}
        onchange={changeLangs}
        onneed={(px) => (menuNeed = px)}
      />
    {:else}
      <span class="strip-title" data-tauri-drag-region>
        {t('popup.ocrTitle')}{#if s.detectedLang}<span class="strip-sub" data-tauri-drag-region>{langName(s.detectedLang)}</span>{/if}
      </span>
    {/if}
    <span class="grow" data-tauri-drag-region></span>
    <button
      class="icon-btn"
      aria-pressed={pinned}
      title={pinned ? t('popup.unpin') : t('popup.pin')}
      aria-label={pinned ? t('popup.unpin') : t('popup.pin')}
      onclick={togglePin}
    >
      <Icon name={pinned ? 'pin-filled' : 'pin'} />
    </button>
    <button
      class="icon-btn"
      title={s.mode === 'ocr' ? t('popup.copyText') : t('popup.copyAll')}
      aria-label={s.mode === 'ocr' ? t('popup.copyText') : t('popup.copyAll')}
      disabled={!s.text.trim()}
      onclick={copyAll}
    >
      <Icon name="copy" />
    </button>
    <button class="icon-btn" title={t('popup.close')} aria-label={t('popup.close')} onclick={() => api.popupHide()}>
      <Icon name="close" />
    </button>
  </header>

  <div class="body">
    <div class="content" bind:this={contentEl}>
      {#if s.mode === 'ocr'}
        <section class="sec ocr-sec">
          {@render sourceBody([100, 92, 54])}
          {@render timingLine()}
        </section>
      {:else}
        <section class="sec source" class:collapsed={s.sourceCollapsed}>
          <div class="sec-head">
            <span class="sec-label">{t('popup.source')}</span>
            <span class="meta tnum">{sourceMeta}</span>
            {#if s.sourceCollapsed && s.text}
              <button class="sec-peek" lang={contentLang} onclick={() => (s.sourceCollapsed = false)}>{s.text}</button>
            {:else}
              <span class="grow"></span>
            {/if}
            {#if s.dirty}
              <button class="text-btn" title="Ctrl+Enter" onclick={retranslate}>
                <Icon name="enter" />{t('popup.retranslate')}
              </button>
            {/if}
            <span class="sec-actions">
              <button
                class="icon-btn sm"
                title={s.sourceCollapsed ? t('popup.expand') : t('popup.collapse')}
                aria-label={s.sourceCollapsed ? t('popup.expand') : t('popup.collapse')}
                aria-expanded={!s.sourceCollapsed}
                onclick={() => (s.sourceCollapsed = !s.sourceCollapsed)}
              >
                <Icon name={s.sourceCollapsed ? 'chevron-down' : 'chevron-up'} />
              </button>
            </span>
          </div>
          {@render timingLine()}
          {#if !s.sourceCollapsed}
            {@render sourceBody([100, 64])}
          {/if}
        </section>

        {#each s.results as r, i (r.engineId)}
          <ResultSection
            {r}
            primary={i === 0}
            {showTimings}
            lang={s.targetLang}
            oncopy={copy}
            onretry={() => retry(r)}
            ontoggle={() => (r.collapsed = !r.collapsed)}
          />
        {/each}
      {/if}
    </div>
  </div>

  {#if s.mode === 'ocr'}
    <footer class="foot" bind:this={footerEl}>
      <span class="meta tnum">{ocrMeta}</span>
      <span class="grow"></span>
      <button class="text-btn" disabled={s.ocrStatus !== 'done' || !s.text.trim()} onclick={retranslate}>
        <Icon name="translate" />{t('popup.translateThis')}
      </button>
    </footer>
  {/if}

  <div class="toast" class:shown={toast.shown} role="status" aria-live="polite">
    <Icon name="check" />{toast.text}
  </div>
</div>
