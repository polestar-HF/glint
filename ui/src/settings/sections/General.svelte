<script lang="ts">
  import { t } from '../../lib/i18n.svelte';
  import type { UiLanguage } from '../../lib/types';
  import Group from '../controls/Group.svelte';
  import Row from '../controls/Row.svelte';
  import Select from '../controls/Select.svelte';
  import Toggle from '../controls/Toggle.svelte';
  import { store } from '../store.svelte';

  const c = $derived(store.config!);
  const languages: Array<{ value: UiLanguage; label: string }> = [
    { value: 'zh-CN', label: '简体中文' },
    { value: 'en-US', label: 'English' },
  ];
  const idle = $derived([
    { value: 60, label: t('time.after1m') },
    { value: 600, label: t('time.after10m') },
    { value: 1800, label: t('time.after30m') },
    { value: 0, label: t('time.never') },
  ]);
</script>

<Group title={t('general.groupStart')}>
  <Row title={t('general.uiLanguage')} id="g-lang">
    <Select labelledby="g-lang" value={c.general.uiLanguage} options={languages} onchange={(v) => (c.general.uiLanguage = v)} />
  </Row>
  <Row title={t('general.autostart')} desc={t('general.autostartDesc')} id="g-auto">
    <Toggle labelledby="g-auto" checked={c.general.autostart} onchange={(v) => (c.general.autostart = v)} />
  </Row>
</Group>

<Group title={t('general.groupCapture')}>
  <Row title={t('general.snap')} desc={t('general.snapDesc')} id="g-snap">
    <Toggle labelledby="g-snap" checked={c.overlay.snapWindows} onchange={(v) => (c.overlay.snapWindows = v)} />
  </Row>
  <Row title={t('general.magnifier')} desc={t('general.magnifierDesc')} id="g-mag">
    <Toggle labelledby="g-mag" checked={c.overlay.magnifier} onchange={(v) => (c.overlay.magnifier = v)} />
  </Row>
</Group>

<Group title={t('general.groupMemory')}>
  <Row title={t('general.popupIdle')} desc={t('general.popupIdleDesc')} id="g-idle">
    <Select
      labelledby="g-idle"
      width={160}
      value={idle.some((o) => o.value === c.ui.popupIdleDestroySecs) ? c.ui.popupIdleDestroySecs : 600}
      options={idle}
      onchange={(v) => (c.ui.popupIdleDestroySecs = v)}
    />
  </Row>
</Group>
