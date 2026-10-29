<script lang="ts">
  import { flip } from 'svelte/animate';
  import { t } from '../../lib/i18n.svelte';
  import { engineFromPreset, isUserAdded, missingFields, type FieldKey, type Preset } from '../../lib/engines';
  import { LANGS, SOURCE_LANGS, langName } from '../../lib/langs';
  import Group from '../controls/Group.svelte';
  import Row from '../controls/Row.svelte';
  import Select from '../controls/Select.svelte';
  import ServicePicker from '../controls/ServicePicker.svelte';
  import { store } from '../store.svelte';
  import EngineRow from './EngineRow.svelte';

  const c = $derived(store.config!);
  const engines = $derived(c.translate.engines);
  const primaryId = $derived(engines.find((e) => e.enabled)?.id);

  const sourceOpts = $derived(SOURCE_LANGS.map((v) => ({ value: v as string, label: langName(v) })));
  const targetOpts = $derived(LANGS.map((v) => ({ value: v as string, label: langName(v) })));

  let dragId = $state<string | null>(null);
  let listEl = $state<HTMLDivElement>();
  let addRow = $state<HTMLDivElement>();
  /** The row just added from a preset, and the field its panel should focus. */
  let justAdded = $state<{ id: string; focus: FieldKey | null } | null>(null);

  function addService(p: Preset) {
    const e = engineFromPreset(p, c.translate.engines.map((x) => x.id));
    c.translate.engines.push(e);
    // "自定义" still needs its endpoint; otherwise the first empty required field (key, model).
    const focus: FieldKey | null = p.custom ? 'baseUrl' : (missingFields(e)[0] ?? null);
    justAdded = { id: e.id, focus };
  }

  function remove(id: string) {
    const i = c.translate.engines.findIndex((e) => e.id === id);
    if (i < 0 || !isUserAdded(c.translate.engines[i])) return;
    c.translate.engines.splice(i, 1);
    requestAnimationFrame(() => addRow?.querySelector<HTMLElement>('.add')?.focus());
  }

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
    (ev.currentTarget as HTMLElement).focus();
    dragId = id;
    const onMove = (m: PointerEvent) => {
      // offsetTop ignores the FLIP transforms, so hit-testing stays right mid-animation.
      const rows = Array.from(listEl!.querySelectorAll<HTMLElement>('.slot'));
      const y = m.clientY - listEl!.getBoundingClientRect().top;
      const from = c.translate.engines.findIndex((e) => e.id === dragId);
      let to = rows.findIndex((r) => y < r.offsetTop + r.offsetHeight / 2);
      if (to === -1) to = rows.length - 1;
      else if (to > from) to -= 1;
      if (to !== from) move(from, to);
    };
    // Listen on window, not with pointer capture on the grip: the keyed list moves the
    // row's DOM node while reordering, which would silently drop the capture.
    const onUp = () => {
      dragId = null;
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
      window.removeEventListener('pointercancel', onUp);
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    window.addEventListener('pointercancel', onUp);
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
          autofocus={justAdded?.id === e.id ? justAdded.focus : undefined}
          onopened={() => (justAdded = null)}
          ondelete={isUserAdded(e) ? () => remove(e.id) : undefined}
        />
      </div>
    {/each}
  </div>
  <div class="add-row" bind:this={addRow}>
    <ServicePicker onpick={addService} />
  </div>
</Group>

<style>
  .engines {
    position: relative;
  }
  .slot + .slot {
    border-top: 1px solid var(--stroke-divider);
  }
  .add-row {
    display: flex;
    align-items: center;
    min-height: 48px;
    padding: 8px 16px 8px 4px;
  }
</style>
