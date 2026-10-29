<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { api } from '../../lib/ipc';
  import { i18n, t, type MessageKey } from '../../lib/i18n.svelte';
  import type { PerfMetric, PerfMetricKey, PerfStats } from '../../lib/types';
  import Group from '../controls/Group.svelte';
  import Row from '../controls/Row.svelte';
  import Toggle from '../controls/Toggle.svelte';
  import Icon from '../../lib/Icon.svelte';
  import { store } from '../store.svelte';

  const c = $derived(store.config!);

  // Pipeline order, not alphabetical: a row reads like the capture it describes.
  const ORDER: PerfMetricKey[] = ['hotkeyToOverlay', 'capture', 'overlay', 'ocr', 'ocrCold', 'ocrLoad', 'det', 'rec'];

  let stats = $state<PerfStats | null>(null);
  let failed = $state('');
  let confirming = $state(false);
  let busy = $state(false);
  let cancelBtn = $state<HTMLButtonElement>();
  let toast = $state({ text: '', shown: false });
  let toastTimer: ReturnType<typeof setTimeout> | undefined;

  const metrics = $derived(
    stats ? ORDER.map((k) => stats!.metrics.find((m) => m.key === k && m.n > 0)).filter((m): m is PerfMetric => !!m) : [],
  );
  const engines = $derived(stats?.engines.filter((e) => e.n > 0 || e.errors > 0) ?? []);
  const empty = $derived(!!stats && metrics.length === 0 && engines.length === 0);
  const sinceText = $derived.by(() => {
    if (!stats || empty) return '';
    const d = new Date(stats.since);
    const date = isNaN(d.getTime())
      ? stats.since
      : new Intl.DateTimeFormat(i18n.locale, { month: 'short', day: 'numeric' }).format(d);
    return t('perf.since', { n: stats.sessions, date });
  });
  const monitors = $derived(
    stats?.system.monitors.map((m) => `${m.width} × ${m.height} · ${Math.round(m.scale * 100)}%`).join(i18n.locale === 'zh-CN' ? '，' : ', ') ?? '',
  );

  const num = (v: number | null | undefined) => (v == null || !isFinite(v) ? '—' : Math.round(v).toLocaleString('en-US'));

  async function refresh() {
    try {
      stats = await api.getPerfStats();
      failed = '';
    } catch (e) {
      failed = String(e);
    }
  }
  onMount(refresh);

  function showToast(text: string) {
    toast = { text, shown: true };
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast.shown = false), 1400);
  }

  async function copyDiagnostics() {
    busy = true;
    try {
      await api.copyText(await api.getDiagnostics());
      showToast(t('perf.copied'));
    } catch (e) {
      showToast(String(e));
    } finally {
      busy = false;
    }
  }

  async function askClear() {
    confirming = true;
    await tick();
    cancelBtn?.focus();
  }

  async function clear() {
    confirming = false;
    try {
      await api.resetPerfStats();
    } catch (e) {
      showToast(String(e));
    }
    await refresh();
  }
</script>

<Group title={t('perf.title')} hint={sinceText}>
  <Row title={t('perf.showTimings')} desc={t('perf.showTimingsDesc')} id="p-show">
    <Toggle labelledby="p-show" checked={!!c.ui.showTimings} onchange={(v) => (c.ui.showTimings = v)} />
  </Row>

  {#if failed}
    <p class="note error">{failed}</p>
  {:else if empty}
    <p class="note">{t('perf.empty')}</p>
  {:else if stats}
    {#if metrics.length}
      <div class="table metrics" role="table" aria-label={t('perf.colMetric')}>
        <div class="tr head" role="row">
          <span class="tcap" role="columnheader">{t('perf.colMetric')}</span>
          <span role="columnheader">{t('perf.colN')}</span>
          <span role="columnheader">{t('perf.colP50')}</span>
          <span role="columnheader">{t('perf.colP90')}</span>
          <span role="columnheader">{t('perf.colMax')}</span>
        </div>
        {#each metrics as m (m.key)}
          <div class="tr" role="row">
            <span class="label" role="cell">{t(`perf.m.${m.key}` as MessageKey)}</span>
            <span class="n" role="cell">{num(m.n)}</span>
            <span class="p50" role="cell">{num(m.p50)}</span>
            <span role="cell">{num(m.p90)}</span>
            <span class="max" role="cell">{num(m.max)}</span>
          </div>
        {/each}
      </div>
    {/if}
    {#if engines.length}
      <div class="table engines" role="table" aria-label={t('perf.colEngine')}>
        <div class="tr head" role="row">
          <span class="tcap" role="columnheader">{t('perf.colEngine')}</span>
          <span role="columnheader">{t('perf.colN')}</span>
          <span role="columnheader">{t('perf.colErrors')}</span>
          <span role="columnheader">{t('perf.colP50')}</span>
          <span role="columnheader">{t('perf.colP90')}</span>
          <span role="columnheader">{t('perf.colFirst')}</span>
        </div>
        {#each engines as e (e.engineId)}
          <div class="tr" role="row">
            <span class="label" role="cell" title={e.engineName}>{e.engineName}</span>
            <span class="n" role="cell">{num(e.n)}</span>
            <span class="n" class:bad={e.errors > 0} role="cell">{num(e.errors)}</span>
            <span class="p50" role="cell">{e.n ? num(e.p50) : '—'}</span>
            <span role="cell">{e.n ? num(e.p90) : '—'}</span>
            <span role="cell">{num(e.firstTokenP50)}</span>
          </div>
        {/each}
      </div>
    {/if}
  {/if}

  {#if stats}
    <div class="system">
      <div class="tcap">{t('perf.system')}</div>
      <dl class="kv">
        <dt>{t('perf.app')}</dt>
        <dd>{stats.system.app}</dd>
        <dt>{t('perf.os')}</dt>
        <dd>{stats.system.os}</dd>
        <dt>{t('perf.cpu')}</dt>
        <dd class="tnum">{t('perf.cpuThreads', { n: stats.system.cpuThreads })}</dd>
        <dt>{t('perf.monitors')}</dt>
        <dd class="tnum">{monitors}</dd>
        <dt>{t('perf.webview')}</dt>
        <dd class="tnum">{stats.system.webview}</dd>
      </dl>
    </div>
  {/if}

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="actions"
    onkeydown={(e) => {
      if (confirming && e.key === 'Escape') {
        e.stopPropagation();
        confirming = false;
      }
    }}
  >
    {#if confirming}
      <span class="confirm-text" role="alert">{t('perf.clearConfirm')}</span>
      <button class="btn sm danger" onclick={clear}>{t('perf.clear')}</button>
      <button class="btn sm subtle" bind:this={cancelBtn} onclick={() => (confirming = false)}>{t('tr.cancel')}</button>
    {:else}
      <button class="btn sm subtle" disabled={!stats || empty} onclick={askClear}>{t('perf.clear')}</button>
      <button class="btn sm" disabled={busy} onclick={copyDiagnostics}>
        <Icon name="copy" />{t('perf.copy')}
      </button>
    {/if}
  </div>
</Group>

<div class="toast" class:shown={toast.shown} role="status" aria-live="polite">
  <Icon name="check" />{toast.text}
</div>

<style>
  .note {
    margin: 0;
    padding: 14px 16px;
    font-size: 13px;
    line-height: 20px;
    color: var(--text-2);
  }
  .note.error {
    color: var(--critical);
  }

  /* Tables: a caption-like first header cell, right-aligned tabular numbers, no row rules.
     The median carries the weight; P90 and max step back. */
  .table {
    padding: 10px 16px 12px;
  }
  .tr {
    display: grid;
    grid-template-columns: minmax(0, 1fr) repeat(4, 64px);
    align-items: center;
    min-height: 28px;
    font-size: 13px;
    line-height: 20px;
    font-variant-numeric: tabular-nums;
    color: var(--text-2);
  }
  .engines .tr {
    grid-template-columns: minmax(0, 1fr) repeat(5, 56px);
  }
  .tr > :not(:first-child) {
    text-align: right;
  }
  .tr.head {
    min-height: 24px;
    font-family: var(--font-small);
    font-size: var(--fs-caption);
    line-height: 16px;
    color: var(--text-3);
  }
  .tcap {
    font-family: var(--font-small);
    font-size: var(--fs-caption);
    line-height: 16px;
    font-weight: 600;
    color: var(--text-2);
  }
  .label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-1);
  }
  .p50 {
    color: var(--text-1);
  }
  .max {
    color: var(--text-3);
  }
  .bad {
    color: var(--critical);
  }

  .system {
    padding: 10px 16px 12px;
  }
  .system .tcap {
    display: block;
    min-height: 24px;
  }
  .kv {
    display: grid;
    grid-template-columns: 96px minmax(0, 1fr);
    gap: 4px 12px;
    margin: 2px 0 0;
    font-size: 13px;
    line-height: 20px;
  }
  dt {
    color: var(--text-2);
  }
  dd {
    margin: 0;
    min-width: 0;
    overflow-wrap: anywhere;
    color: var(--text-1);
    user-select: text;
  }

  .actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    min-height: 48px;
    padding: 8px 16px;
  }
  .actions .btn:not(.subtle) {
    gap: 6px;
  }
  .confirm-text {
    margin-right: 4px;
    font-size: 13px;
    color: var(--text-1);
  }
  .btn.danger {
    color: var(--critical);
  }

  /* Same toast as the popup: inverse chip, bottom-center of the window. */
  .toast {
    position: fixed;
    z-index: 60;
    left: 50%;
    bottom: 20px;
    display: flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 12px 0 10px;
    border-radius: var(--r-sm);
    background: var(--bg-inverse);
    color: var(--text-inverse);
    font-family: var(--font-small);
    font-size: var(--fs-caption);
    line-height: 16px;
    white-space: nowrap;
    box-shadow: 0 2px 8px rgb(0 0 0 / 0.18);
    pointer-events: none;
    opacity: 0;
    transform: translate(-50%, 4px);
    transition:
      opacity var(--dur-1) var(--ease-in),
      transform var(--dur-1) var(--ease-in);
  }
  .toast.shown {
    opacity: 1;
    transform: translate(-50%, 0);
    transition-duration: var(--dur-2);
    transition-timing-function: var(--ease-out);
  }
</style>
