<script lang="ts">
  import { t, type MessageKey } from '../../lib/i18n.svelte';
  import Group from '../controls/Group.svelte';
  import Row from '../controls/Row.svelte';
  import Select from '../controls/Select.svelte';
  import Toggle from '../controls/Toggle.svelte';
  import { store } from '../store.svelte';

  const c = $derived(store.config!);
  const engines: Array<{ id: 'paddle' | 'windows'; name: string; tag: MessageKey; desc: MessageKey }> = [
    { id: 'paddle', name: 'PaddleOCR', tag: 'ocr.paddleTag', desc: 'ocr.paddleDesc' },
    { id: 'windows', name: 'Windows OCR', tag: 'ocr.windowsTag', desc: 'ocr.windowsDesc' },
  ];
  const idle = $derived([
    { value: 60, label: t('time.after1m') },
    { value: 300, label: t('time.after5m') },
    { value: 900, label: t('time.after15m') },
    { value: 0, label: t('time.never') },
  ]);
</script>

<Group title={t('ocr.groupEngine')}>
  {#each engines as e (e.id)}
    <label class="choice">
      <input
        class="radio"
        type="radio"
        name="ocr-engine"
        value={e.id}
        checked={c.ocr.engine === e.id}
        onchange={() => (c.ocr.engine = e.id)}
      />
      <span class="choice-text">
        <span class="choice-title">{e.name}<span class="choice-tag">{t(e.tag)}</span></span>
        <span class="choice-desc">{t(e.desc)}</span>
      </span>
    </label>
  {/each}
</Group>

<Group title={t('ocr.groupText')}>
  <Row title={t('ocr.merge')} desc={t('ocr.mergeDesc')} id="o-merge">
    <Toggle labelledby="o-merge" checked={c.ocr.mergeLines} onchange={(v) => (c.ocr.mergeLines = v)} />
  </Row>
</Group>

<Group title={t('ocr.groupMemory')}>
  <Row title={t('ocr.idle')} desc={t('ocr.idleDesc')} id="o-idle">
    <Select
      labelledby="o-idle"
      width={160}
      value={idle.some((o) => o.value === c.ocr.idleUnloadSecs) ? c.ocr.idleUnloadSecs : 300}
      options={idle}
      onchange={(v) => (c.ocr.idleUnloadSecs = v)}
    />
  </Row>
</Group>

<style>
  .choice {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    padding: 12px 16px;
    transition: background-color var(--dur-1) var(--ease-std);
  }
  .choice:hover {
    background: var(--bg-subtle-hover);
  }
  .choice-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .choice-title {
    display: flex;
    align-items: baseline;
    gap: 8px;
    font-size: 14px;
    line-height: 20px;
    color: var(--text-1);
  }
  .choice-tag {
    font-family: var(--font-small);
    font-size: var(--fs-caption);
    color: var(--text-2);
  }
  .choice-desc {
    font-family: var(--font-small);
    font-size: var(--fs-caption);
    line-height: 16px;
    color: var(--text-2);
  }
  /* Fluent RadioButton: 20px ring; checked = accent disc with an 8px center. */
  .radio {
    appearance: none;
    flex: none;
    width: 20px;
    height: 20px;
    margin: 0;
    border-radius: 50%;
    border: 1px solid var(--stroke-strong);
    background: var(--bg-subtle-track);
    display: grid;
    place-items: center;
    transition:
      background-color var(--dur-1) var(--ease-std),
      border-color var(--dur-1) var(--ease-std);
  }
  .radio::after {
    content: '';
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--on-accent);
    transform: scale(0);
    transition: transform var(--dur-2) var(--ease-out);
  }
  .radio:checked {
    border-color: var(--accent);
    background: var(--accent);
  }
  .radio:checked::after {
    transform: scale(1);
  }
  .choice:hover .radio:checked::after {
    transform: scale(1.2);
  }
</style>
