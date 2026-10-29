<script lang="ts">
  import { t, type MessageKey } from '../../lib/i18n.svelte';
  import { defaultConfig } from '../../lib/engines';
  import Group from '../controls/Group.svelte';
  import KeyRecorder from '../controls/KeyRecorder.svelte';
  import Row from '../controls/Row.svelte';
  import { store } from '../store.svelte';

  type Key = 'translate' | 'ocr' | 'silentOcr';
  type Msg = { text: string; tone: 'error' | 'info' } | null;

  const c = $derived(store.config!);
  const defaults = defaultConfig().hotkeys;
  const rows: Array<{ key: Key; title: MessageKey; desc: MessageKey }> = [
    { key: 'translate', title: 'hotkeys.translate', desc: 'hotkeys.translateDesc' },
    { key: 'ocr', title: 'hotkeys.ocr', desc: 'hotkeys.ocrDesc' },
    { key: 'silentOcr', title: 'hotkeys.silentOcr', desc: 'hotkeys.silentOcrDesc' },
  ];
  let messages = $state<Record<Key, Msg>>({ translate: null, ocr: null, silentOcr: null });

  const others = (key: Key) =>
    rows.filter((r) => r.key !== key).map((r) => ({ name: t(r.title), acc: c.hotkeys[r.key] }));

  const overlayKeys: Array<{ title: MessageKey; keys: string[] }> = [
    { title: 'hotkeys.confirm', keys: ['Enter'] },
    { title: 'hotkeys.copyOnly', keys: ['Ctrl', 'C'] },
    { title: 'hotkeys.cancel', keys: ['Esc'] },
  ];
</script>

<Group>
  {#each rows as r (r.key)}
    <Row title={t(r.title)} desc={t(r.desc)} id="hk-{r.key}" message={messages[r.key]}>
      <KeyRecorder
        labelledby="hk-{r.key}"
        value={c.hotkeys[r.key]}
        defaultValue={defaults[r.key]}
        others={others(r.key)}
        onchange={(v) => (c.hotkeys[r.key] = v)}
        onmessage={(m) => (messages[r.key] = m)}
      />
    </Row>
  {/each}
</Group>

<Group title={t('hotkeys.groupOverlay')}>
  {#each overlayKeys as k (k.title)}
    <Row title={t(k.title)}>
      <span class="keys">{#each k.keys as key (key)}<kbd class="kbd">{key}</kbd>{/each}</span>
    </Row>
  {/each}
</Group>
