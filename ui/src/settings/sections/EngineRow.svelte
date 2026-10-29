<script lang="ts">
  import Icon from '../../lib/Icon.svelte';
  import { api } from '../../lib/ipc';
  import { fmtMs, t } from '../../lib/i18n.svelte';
  import { engineName, isConfigured, kindSpec } from '../../lib/engines';
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
  }
  let { e, primary, dragging, first, last, onmove, ongrab }: Props = $props();

  const spec = $derived(kindSpec(e.kind));
  const name = $derived(engineName(e));
  const configured = $derived(isConfigured(e));
  let open = $state(false);
  let reveal = $state<Record<string, boolean>>({});
  let test = $state<{ state: 'idle' | 'running' | 'ok' | 'error'; text: string; ms: number }>({
    state: 'idle',
    text: '',
    ms: 0,
  });
  const uid = $derived(`eng-${e.id}`);

  const sub = $derived.by(() => {
    if (spec.free) return t('tr.free');
    if (!configured) return t('tr.needsKey');
    const host = e.baseUrl ? e.baseUrl.replace(/^https?:\/\//, '').replace(/\/.*$/, '') : '';
    return [e.model, e.kind === 'openai' ? host : ''].filter(Boolean).join(' · ') || t('tr.configure');
  });

  async function runTest() {
    test = { state: 'running', text: '', ms: 0 };
    const t0 = performance.now();
    try {
      const out = await api.testEngine($state.snapshot(e) as EngineConfig);
      test = { state: 'ok', text: out, ms: performance.now() - t0 };
    } catch (err) {
      test = { state: 'error', text: String(err), ms: 0 };
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

  function setField(key: 'name' | 'apiKey' | 'apiSecret' | 'baseUrl' | 'model' | 'prompt', v: string) {
    e[key] = v.trim() === '' ? null : v;
  }
</script>

<div class="engine" class:dragging class:disabled={!e.enabled} data-engine-row={e.id}>
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
        <span>{name}</span>
        {#if primary}<span class="primary-mark">{t('tr.primary')}</span>{/if}
      </div>
      <div class="engine-sub" class:warn={e.enabled && !configured}>{sub}</div>
      {#if test.state === 'ok' || test.state === 'error'}
        <div class="test-result" class:error={test.state === 'error'} role="status">
          <Icon name={test.state === 'ok' ? 'check' : 'alert'} />
          <span class="test-text" title={test.text}>{test.text}</span>
          {#if test.state === 'ok'}<span class="test-ms tnum">{fmtMs(test.ms)}</span>{/if}
        </div>
      {/if}
    </div>

    <div class="engine-actions">
      <button class="btn sm" disabled={test.state === 'running'} onclick={runTest}>
        {#if test.state === 'running'}<span class="spinner" aria-hidden="true"></span>{t('tr.testing')}{:else}{t('tr.test')}{/if}
      </button>
      <button class="btn sm disclose" aria-expanded={open} aria-controls="{uid}-panel" onclick={() => (open = !open)}>
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
          />
        </div>
        {#each spec.fields as f (f.key)}
          <label class="f-label" for="{uid}-{f.key}">
            {t(f.label)}{#if f.optional}<span class="f-opt">{t('field.optional')}</span>{/if}
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
                  type={f.secret && !reveal[f.key] ? 'password' : 'text'}
                  autocomplete="off"
                  spellcheck="false"
                  value={e[f.key] ?? ''}
                  placeholder={f.placeholder ?? ''}
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
            {#if f.hint}<p class="f-hint">{t(f.hint)}</p>{/if}
          </div>
        {/each}
      </div>
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
    padding: 10px 16px 10px 4px;
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
  .engine-sub {
    font-family: var(--font-small);
    font-size: var(--fs-caption);
    line-height: 16px;
    color: var(--text-3);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .engine-sub.warn {
    color: var(--caution);
  }
  .test-result {
    display: flex;
    align-items: center;
    gap: 6px;
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
    gap: 8px;
  }
  .engine-actions :global(.toggle) {
    margin-left: 8px;
  }
  .disclose {
    gap: 4px;
    padding-right: 6px;
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
    padding-top: 6px;
    font-size: 13px;
    line-height: 20px;
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
  .f-hint {
    margin: 4px 0 0;
    font-family: var(--font-small);
    font-size: var(--fs-caption);
    line-height: 16px;
    color: var(--text-3);
    overflow-wrap: anywhere;
  }
</style>
