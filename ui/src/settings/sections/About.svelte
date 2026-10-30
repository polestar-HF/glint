<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '../../lib/Icon.svelte';
  import { api } from '../../lib/ipc';
  import { t, type MessageKey } from '../../lib/i18n.svelte';
  import Group from '../controls/Group.svelte';
  import PerfCard from './PerfCard.svelte';

  const REPO = 'https://github.com/polestar-HF/glint';
  const links: Array<{ label: MessageKey; url: string }> = [
    { label: 'about.home', url: REPO },
    { label: 'about.issues', url: `${REPO}/issues` },
    { label: 'about.changelog', url: `${REPO}/releases` },
  ];
  const oss = [
    { name: 'PaddleOCR', license: 'Apache-2.0' },
    { name: 'ONNX Runtime', license: 'MIT' },
    { name: 'Tauri', license: 'MIT / Apache-2.0' },
    { name: 'Svelte', license: 'MIT' },
    { name: 'qingicon', license: 'Apache-2.0' },
  ];

  let version = $state('');
  onMount(() => {
    api.appVersion().then((v) => (version = v), () => {});
  });
</script>

<div class="hero">
  <h1 class="wordmark">Glint</h1>
  <div class="tagline">{t('about.tagline')}</div>
  {#if version}<div class="version tnum">{t('about.version', { v: version })}</div>{/if}
</div>

<Group>
  {#each links as l (l.label)}
    <button class="link-row" onclick={() => api.openUrl(l.url)}>
      <span>{t(l.label)}</span>
      <Icon name="external" />
    </button>
  {/each}
</Group>

<PerfCard />

<Group title={t('about.groupOss')}>
  {#each oss as o (o.name)}
    <div class="kv">
      <span>{o.name}</span>
      <span class="v">{o.license}</span>
    </div>
  {/each}
</Group>

<Group title={t('about.groupData')}>
  <div class="kv">
    <span>{t('about.configFile')}</span>
    <span class="v path">%APPDATA%\Glint\config.json</span>
  </div>
</Group>

<style>
  .hero {
    margin: -2px 2px 24px;
  }
  .wordmark {
    font-family: var(--font-display);
    font-size: var(--fs-title);
    line-height: 36px;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--text-1);
  }
  .tagline {
    margin-top: 2px;
    color: var(--text-2);
  }
  .version {
    margin-top: 2px;
    font-family: var(--font-small);
    font-size: var(--fs-caption);
    line-height: 16px;
    color: var(--text-3);
  }
  .link-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    min-height: 44px;
    padding: 0 16px;
    color: var(--text-1);
    transition: background-color var(--dur-1) var(--ease-std);
  }
  .link-row:hover {
    background: var(--bg-subtle-hover);
  }
  .link-row:active {
    background: var(--bg-subtle-pressed);
  }
  .link-row :global(.icon) {
    color: var(--text-2);
  }
  .link-row:focus-visible {
    outline-offset: -2px;
  }
  .kv {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    min-height: 44px;
    padding: 0 16px;
  }
  .v {
    color: var(--text-2);
    font-size: 13px;
  }
  .path {
    font-family: var(--font-mono);
    font-size: 12px;
    user-select: text;
  }
</style>
