<script lang="ts">
  import { onMount } from 'svelte';
  import BrandMark from '../../lib/BrandMark.svelte';
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
  <div class="hero-mark">
    <BrandMark size={52} />
  </div>
  <div class="hero-text">
    <div class="hero-head">
      <h1 class="wordmark">Glint</h1>
      {#if version}<span class="version tnum">{t('about.version', { v: version })}</span>{/if}
    </div>
    <div class="tagline">{t('about.tagline')}</div>
  </div>
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
    display: flex;
    align-items: center;
    gap: 16px;
    margin: 0 2px 24px;
  }
  .hero-mark {
    flex: none;
    display: grid;
    place-items: center;
    filter: drop-shadow(0 6px 14px light-dark(rgb(15 23 42 / 0.14), rgb(0 0 0 / 0.36)));
  }
  .hero-text {
    min-width: 0;
  }
  .hero-head {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .wordmark {
    font-family: var(--font-display);
    font-size: var(--fs-title);
    line-height: 34px;
    font-weight: 600;
    letter-spacing: -0.02em;
    color: var(--text-1);
  }
  .tagline {
    margin-top: 2px;
    color: var(--text-2);
  }
  .version {
    display: inline-flex;
    align-items: center;
    height: 20px;
    padding: 0 8px;
    border-radius: var(--r-pill);
    corner-shape: round;
    background: var(--accent-soft);
    border: 1px solid var(--accent-border);
    font-family: var(--font-small);
    font-size: 11px;
    line-height: 1;
    font-weight: 600;
    color: var(--accent);
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
