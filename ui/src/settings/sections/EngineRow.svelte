<script lang="ts">
  import { asErrorInfo, describeError } from '../../lib/errors';
  import { tick, untrack } from 'svelte';
  import Icon from '../../lib/Icon.svelte';
  import { api } from '../../lib/ipc';
  import { fmtMs, t, type MessageKey, i18n } from '../../lib/i18n.svelte';
  import {
    engineName,
    hostOf,
    isUserAdded,
    kindSpec,
    missingFields,
    missingHint,
    requiredFields,
    presetForUrl,
    protocolLabel,
    type FieldKey,
  } from '../../lib/engines';
  import type { EngineConfig } from '../../lib/types';
  import Toggle from '../controls/Toggle.svelte';

  interface Props {
    e: EngineConfig;
    primary: boolean;
    dragging: boolean;
    first: boolean;
    last: boolean;
    onmove: (delta: -1 | 1) => void;
    ongrab: (ev: PointerEvent) => void;
    /** Set right after the row was added: open the panel and focus this field (null: the name). */
    autofocus?: FieldKey | null;
    onopened?: () => void;
    /** User-added rows only. */
    ondelete?: () => void;
  }
  let { e, primary, dragging, first, last, onmove, ongrab, autofocus, onopened, ondelete }: Props = $props();

  const spec = $derived(kindSpec(e.kind));
  const name = $derived(engineName(e));
  const added = $derived(isUserAdded(e));
  const missing = $derived(missingFields(e));
  const required = $derived(requiredFields(e));
  const hint = $derived(spec.free ? '' : missingHint(e));
  const customBase = $derived(e.kind === 'openai' && !!e.baseUrl?.trim() && !e.baseUrl.includes('api.openai.com'));

  let open = $state(false);
  let confirming = $state(false);
  let rowEl = $state<HTMLDivElement>();
  let cancelBtn = $state<HTMLButtonElement>();
  let reveal = $state<Record<string, boolean>>({});
  let test = $state<{ state: 'idle' | 'running' | 'ok' | 'error'; text: string; ms: number }>({
    state: 'idle',
    text: '',
    ms: 0,
  });
  const uid = $derived(`eng-${e.id}`);

  /** Secondary line: endpoint for user-added services, model / host for built-ins. */
  const sub = $derived.by(() => {
    if (spec.free) return t('tr.free');
    if (added) {
      const host = hostOf(e.baseUrl) || (e.kind === 'anthropic' ? 'api.anthropic.com' : e.kind === 'openai' ? 'api.openai.com' : '');
      return [host, e.model].filter(Boolean).join(' · ');
    }
    if (hint) return '';
    return [e.model, e.kind === 'openai' ? hostOf(e.baseUrl) : ''].filter(Boolean).join(' · ') || t('tr.configure');
  });

  function placeholder(key: FieldKey, fallback?: string): string {
    if (key === 'model' && customBase) return t('field.modelPh');
    return fallback ?? '';
  }

  function fieldHint(key: FieldKey, fallback?: MessageKey): MessageKey | null {
    // The "OpenAI-compatible, e.g. Ollama" note only helps while the endpoint is unknown.
    if (key === 'baseUrl' && e.kind === 'openai') return presetForUrl(e.baseUrl ?? '') ? null : (fallback ?? null);
    if (key === 'baseUrl' && e.kind === 'anthropic') return 'hint.anthropicBase';
    return fallback ?? null;
  }

  async function runTest() {
    test = { state: 'running', text: '', ms: 0 };
    const t0 = performance.now();
    try {
      const out = await api.testEngine($state.snapshot(e) as EngineConfig);
      test = { state: 'ok', text: out, ms: performance.now() - t0 };
    } catch (err) {
      const info = asErrorInfo(err);
      const free = (e.kind === 'google' || e.kind === 'bing') && !e.apiKey?.trim();
      const d = describeError(info, free);
      const sep = i18n.locale === 'zh-CN' ? '。' : '. ';
      // Settings is where the fix happens, so only rate limits get advice here.
      const hint = info.code === 'rate_limited' ? d.hint : null;
      test = { state: 'error', text: hint ? `${d.text}${sep}${hint}` : d.text, ms: 0 };
    }
  }

  function gripKey(ev: KeyboardEvent) {
    if (ev.key === 'ArrowUp' && !first) {
      ev.preventDefault();
      onmove(-1);
    } else if (ev.key === 'ArrowDown' && !last) {
      ev.preventDefault();
      onmove(1);
    }
  }

  function setField(key: FieldKey, v: string) {
    // User-added services always keep a name (v0.2 contract); an emptied field is restored on blur.
    if (key === 'name' && added && !v.trim()) return;
    e[key] = v.trim() === '' ? null : v;
  }

  // Freshly added: open the panel and put the caret where the user has to type next.
  $effect(() => {
    if (autofocus === undefined) return;
    const field = autofocus;
    untrack(() => {
      open = true;
      void tick().then(() => {
        const target = document.getElementById(field ? `${uid}-${field}` : `${uid}-name-f`);
        target?.focus({ preventScroll: true });
        rowEl?.scrollIntoView({ block: 'nearest' });
        onopened?.();
      });
    });
  });

  async function askDelete() {
    confirming = true;
    await tick();
    cancelBtn?.focus();
  }
</script>

<div class="engine" class:dragging class:disabled={!e.enabled} data-engine-row={e.id} bind:this={rowEl}>
  <div class="engine-row">
    <button
      class="grip"
      title={t('tr.reorder')}
      aria-label="{name}: {t('tr.reorder')}"
      onpointerdown={ongrab}
      onkeydown={gripKey}
    >
      <Icon name="grip" />
    </button>

    <div class="engine-text">
      <div class="engine-name" id="{uid}-name">
        <span class="engine-title">{name}</span>
        {#if added}<span class="proto">{protocolLabel(e.kind)}</span>{/if}
        {#if primary}<span class="primary-mark">{t('tr.primary')}</span>{/if}
      </div>
      <div class="engine-sub">
        {sub}{#if hint}{#if sub}{' · '}{/if}<span class:warn={e.enabled}>{hint}</span>{/if}
      </div>
      {#if test.state === 'ok' || test.state === 'error'}
        <div class="test-result" class:error={test.state === 'error'} role="status">
          <Icon name={test.state === 'ok' ? 'check' : 'alert'} />
          <span class="test-text" title={test.text}>{test.text}</span>
          {#if test.state === 'ok'}<span class="test-ms tnum">{fmtMs(test.ms)}</span>{/if}
        </div>
      {/if}
    </div>

    <div class="engine-actions">
      <button class="btn sm subtle" disabled={test.state === 'running'} onclick={runTest}>
        {#if test.state === 'running'}<span class="spinner" aria-hidden="true"></span>{t('tr.testing')}{:else}{t('tr.test')}{/if}
      </button>
      <button class="btn sm subtle disclose" aria-expanded={open} aria-controls="{uid}-panel" onclick={() => (open = !open)}>
        {t('tr.configure')}<Icon name={open ? 'chevron-up' : 'chevron-down'} />
      </button>
      <Toggle
        checked={e.enabled}
        label={t('tr.enable', { name })}
        onchange={(v) => (e.enabled = v)}
      />
    </div>
  </div>

  {#if open}
    <div class="panel" id="{uid}-panel">
      {#if spec.free}
        <p class="panel-note">{t('tr.noConfig')}</p>
      {/if}
      <div class="fields">
        <label class="f-label" for="{uid}-name-f">{t('field.name')}</label>
        <div class="f-input">
          <input
            id="{uid}-name-f"
            class="field"
            value={e.name ?? ''}
            placeholder={engineName({ kind: e.kind, name: null })}
            oninput={(ev) => setField('name', ev.currentTarget.value)}
            onblur={(ev) => {
              if (added && !ev.currentTarget.value.trim()) ev.currentTarget.value = e.name ?? '';
            }}
          />
        </div>
        {#each spec.fields as f (f.key)}
          {@const need = missing.includes(f.key)}
          {@const h = fieldHint(f.key, f.hint)}
          <label class="f-label" for="{uid}-{f.key}">
            {t(f.label)}{#if f.optional && !required.includes(f.key)}<span class="f-opt">{t('field.optional')}</span>{/if}
          </label>
          <div class="f-input">
            {#if f.multiline}
              <textarea
                id="{uid}-{f.key}"
                class="field"
                rows="2"
                value={e[f.key] ?? ''}
                placeholder={t('field.promptPh')}
                oninput={(ev) => setField(f.key, ev.currentTarget.value)}
              ></textarea>
            {:else}
              <div class="f-line">
                <input
                  id="{uid}-{f.key}"
                  class="field"
                  class:mono={f.secret || f.key === 'baseUrl' || f.key === 'model'}
                  class:missing={need}
                  aria-invalid={need || undefined}
                  type={f.secret && !reveal[f.key] ? 'password' : 'text'}
                  autocomplete="off"
                  spellcheck="false"
                  value={e[f.key] ?? ''}
                  placeholder={placeholder(f.key, f.placeholder)}
                  oninput={(ev) => setField(f.key, ev.currentTarget.value)}
                />
                {#if f.secret}
                  <button
                    class="icon-btn reveal"
                    title={reveal[f.key] ? t('field.hide') : t('field.show')}
                    aria-label={reveal[f.key] ? t('field.hide') : t('field.show')}
                    aria-pressed={!!reveal[f.key]}
                    onclick={() => (reveal[f.key] = !reveal[f.key])}
                  >
                    <Icon name={reveal[f.key] ? 'eye-off' : 'eye'} />
                  </button>
                {/if}
              </div>
            {/if}
            {#if h}<p class="f-hint">{t(h)}</p>{/if}
          </div>
        {/each}
      </div>

      {#if added && ondelete}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="panel-foot"
          onkeydown={(ev) => {
            if (confirming && ev.key === 'Escape') {
              ev.stopPropagation();
              confirming = false;
            }
          }}
        >
          {#if confirming}
            <span class="confirm-text" role="alert">{t('tr.deleteConfirm', { name })}</span>
            <button class="btn sm danger" onclick={ondelete}>{t('tr.delete')}</button>
            <button class="btn sm subtle" bind:this={cancelBtn} onclick={() => (confirming = false)}>{t('tr.cancel')}</button>
          {:else}
            <button class="btn sm subtle remove" onclick={askDelete}><Icon name="trash" />{t('tr.deleteService')}</button>
          {/if}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .engine {
    position: relative;
    background: transparent;
    transition: background-color var(--dur-1) var(--ease-std);
  }
  .engine.dragging {
    z-index: 1;
    background: var(--bg-flyout);
    box-shadow: var(--shadow-flyout);
  }
  .engine-row {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 60px;
    padding: 8px 16px 8px 4px;
  }
  .grip {
    flex: none;
    display: grid;
    place-items: center;
    width: 24px;
    height: 32px;
    border-radius: var(--r-sm);
    color: var(--text-4);
    cursor: grab;
    touch-action: none;
    transition: color var(--dur-1) var(--ease-std);
  }
  .engine:hover .grip,
  .grip:focus-visible {
    color: var(--text-2);
  }
  .dragging .grip {
    cursor: grabbing;
  }
  .engine-text {
    flex: 1 1 auto;
    min-width: 0;
  }
  .engine-name {
    display: flex;
    align-items: baseline;
    gap: 8px;
    font-size: 14px;
    line-height: 20px;
    color: var(--text-1);
  }
  .disabled .engine-name {
    color: var(--text-2);
  }
  .primary-mark {
    font-family: var(--font-small);
    font-size: var(--fs-caption);
    color: var(--accent);
  }
  .engine-title {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .proto {
    flex: none;
    font-family: var(--font-small);
    font-size: var(--fs-caption);
    color: var(--text-3);
  }
  .primary-mark {
    flex: none;
  }
  .engine-sub {
    overflow: hidden;
    text-overflow: ellipsis;
    font-family: var(--font-small);
    font-size: var(--fs-caption);
    line-height: 16px;
    color: var(--text-3);
    white-space: nowrap;
  }
  .engine-sub .warn {
    color: var(--caution);
  }
  .test-result {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 6px;
    font-family: var(--font-small);
    font-size: var(--fs-caption);
    line-height: 16px;
    color: var(--success);
    animation: fade-in var(--dur-2) var(--ease-out);
  }
  .test-result.error {
    color: var(--critical);
  }
  .test-text {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-1);
  }
  .test-result.error .test-text {
    color: var(--critical);
  }
  .test-ms {
    color: var(--text-3);
  }
  @keyframes fade-in {
    from {
      opacity: 0;
    }
  }
  .engine-actions {
    flex: none;
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .engine-actions :global(.toggle) {
    margin-left: 12px;
  }
  .disclose {
    gap: 4px;
    padding-right: 8px;
  }
  .disclose :global(.icon) {
    color: var(--text-2);
  }

  .panel {
    padding: 4px 16px 16px 36px;
    animation: fade-in var(--dur-2) var(--ease-out);
  }
  .panel-note {
    margin: 0 0 12px;
    font-family: var(--font-small);
    font-size: var(--fs-caption);
    line-height: 16px;
    color: var(--text-2);
  }
  .fields {
    display: grid;
    grid-template-columns: 96px minmax(0, 1fr);
    gap: 8px 12px;
    align-items: start;
  }
  .f-label {
    display: flex;
    flex-direction: column;
    padding-top: 4px;
    font-size: 13px;
    line-height: 24px;
    color: var(--text-2);
  }
  .f-opt {
    font-family: var(--font-small);
    font-size: 11px;
    line-height: 14px;
    color: var(--text-3);
  }
  .f-line {
    position: relative;
  }
  .f-line .field[type='password'],
  .f-line:has(.reveal) .field {
    padding-right: 36px;
  }
  .reveal {
    position: absolute;
    top: 2px;
    right: 2px;
  }
  /* A required field that is still empty: the bottom edge picks up the caution color,
     matching the hint on the row. Focus keeps the accent. */
  .field.missing:not(:focus) {
    border-bottom-color: var(--caution);
  }
  .panel-foot {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    min-height: 28px;
    margin-top: 12px;
  }
  .confirm-text {
    margin-right: 4px;
    font-size: 13px;
    color: var(--text-1);
  }
  .remove {
    gap: 8px;
    color: var(--critical);
  }
  .btn.danger {
    color: var(--critical);
  }
  .f-hint {
    margin: 4px 0 0;
    font-family: var(--font-small);
    font-size: var(--fs-caption);
    line-height: 16px;
    color: var(--text-3);
    overflow-wrap: anywhere;
  }
</style>
