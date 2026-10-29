<script lang="ts">
  import { flip } from 'svelte/animate';
  import { t } from '../../lib/i18n.svelte';
  import { LANGS, SOURCE_LANGS, langName } from '../../lib/langs';
  import Group from '../controls/Group.svelte';
  import Row from '../controls/Row.svelte';
  import Select from '../controls/Select.svelte';
  import { store } from '../store.svelte';
  import EngineRow from './EngineRow.svelte';

  const c = $derived(store.config!);
  const engines = $derived(c.translate.engines);
  const primaryId = $derived(engines.find((e) => e.enabled)?.id);

  const sourceOpts = $derived(SOURCE_LANGS.map((v) => ({ value: v as string, label: langName(v) })));
  const targetOpts = $derived(LANGS.map((v) => ({ value: v as string, label: langName(v) })));

  let dragId = $state<string | null>(null);
  let listEl = $state<HTMLDivElement>();

  function move(from: number, to: number) {
    const list = c.translate.engines;
    if (to < 0 || to >= list.length || from === to) return;
    const [item] = list.splice(from, 1);
    list.splice(to, 0, item);
  }

  function moveBy(id: string, delta: -1 | 1) {
    const i = engines.findIndex((e) => e.id === id);
    move(i, i + delta);
    // Keep keyboard focus on the moved row's grip.
    requestAnimationFrame(() =>
      listEl?.querySelector<HTMLElement>(`[data-engine-row="${CSS.escape(id)}"] .grip`)?.focus(),
    );
  }

  function grab(ev: PointerEvent, id: string) {
    if (ev.button !== 0 || !listEl) return;
    ev.preventDefault();
    const grip = ev.currentTarget as HTMLElement;
    grip.setPointerCapture(ev.pointerId);
    dragId = id;
    const onMove = (m: PointerEvent) => {
      const rows = Array.from(listEl!.querySelectorAll<HTMLElement>('[data-engine-row]'));
      const from = c.translate.engines.findIndex((e) => e.id === dragId);
      let to = rows.findIndex((r) => {
        const b = r.getBoundingClientRect();
        return m.clientY < b.top + b.height / 2;
      });
      if (to === -1) to = rows.length - 1;
      else if (to > from) to -= 1;
      if (to !== from) move(from, to);
    };
    const onUp = () => {
      dragId = null;
      grip.removeEventListener('pointermove', onMove);
      grip.removeEventListener('pointerup', onUp);
      grip.removeEventListener('pointercancel', onUp);
    };
    grip.addEventListener('pointermove', onMove);
    grip.addEventListener('pointerup', onUp);
    grip.addEventListener('pointercancel', onUp);
  }
</script>

<Group title={t('tr.groupLang')}>
  <Row title={t('tr.source')} id="t-src">
    <Select labelledby="t-src" value={c.translate.sourceLang} options={sourceOpts} onchange={(v) => (c.translate.sourceLang = v)} />
  </Row>
  <Row title={t('tr.target')} id="t-tgt">
    <Select labelledby="t-tgt" value={c.translate.targetLang} options={targetOpts} onchange={(v) => (c.translate.targetLang = v)} />
  </Row>
  <Row title={t('tr.secondary')} desc={t('tr.secondaryDesc')} id="t-sec">
    <Select
      labelledby="t-sec"
      value={c.translate.secondaryLang}
      options={targetOpts}
      onchange={(v) => (c.translate.secondaryLang = v)}
    />
  </Row>
</Group>

<Group title={t('tr.groupEngines')} hint={t('tr.enginesHint')}>
  <div class="engines" bind:this={listEl}>
    {#each engines as e, i (e.id)}
      <div class="slot" animate:flip={{ duration: 140 }}>
        <EngineRow
          {e}
          primary={e.id === primaryId}
          dragging={dragId === e.id}
          first={i === 0}
          last={i === engines.length - 1}
          onmove={(d) => moveBy(e.id, d)}
          ongrab={(ev) => grab(ev, e.id)}
        />
      </div>
    {/each}
  </div>
</Group>

<style>
  .slot + .slot {
    border-top: 1px solid var(--stroke-divider);
  }
</style>
