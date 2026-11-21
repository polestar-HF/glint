<script lang="ts">
  import Icon from '../../lib/Icon.svelte';
  import { t, type MessageKey } from '../../lib/i18n.svelte';
  import { ACCENTS, normalizeHex } from '../../lib/theme';
  import type { ThemeChoice } from '../../lib/types';
  import Group from '../controls/Group.svelte';
  import Row from '../controls/Row.svelte';
  import Segmented from '../controls/Segmented.svelte';
  import Select from '../controls/Select.svelte';
  import Toggle from '../controls/Toggle.svelte';
  import { store } from '../store.svelte';

  const c = $derived(store.config!);
  const themes = $derived<Array<{ value: ThemeChoice; label: string }>>([
    { value: 'system', label: t('ap.themeSystem') },
    { value: 'light', label: t('ap.themeLight') },
    { value: 'dark', label: t('ap.themeDark') },
  ]);
  const sizes = [12, 13, 14, 15, 16, 18, 20].map((n) => ({ value: n, label: `${n} px` }));

  const accent = $derived(normalizeHex(c.ui.accent) ?? ACCENTS[0].hex);
  const preset = $derived(ACCENTS.find((a) => a.hex === accent));
  let hexDraft = $state<string | null>(null);
  const hexShown = $derived(hexDraft ?? accent);
  const hexValid = $derived(hexDraft == null || normalizeHex(hexDraft) != null);

  function commitHex() {
    const v = normalizeHex(hexDraft);
    if (v) c.ui.accent = v;
    hexDraft = null;
  }
</script>

<Group>
  <Row title={t('ap.theme')} id="a-theme">
    <Segmented label={t('ap.theme')} value={c.ui.theme} options={themes} onchange={(v) => (c.ui.theme = v)} />
  </Row>
  <Row title={t('ap.fontSize')} desc={t('ap.fontSizeDesc')} id="a-size">
    <Select
      labelledby="a-size"
      width={120}
      value={sizes.some((s) => s.value === c.ui.fontSize) ? c.ui.fontSize : 14}
      options={sizes}
      onchange={(v) => (c.ui.fontSize = v)}
    />
  </Row>
  <div class="accent-row">
    <div class="accent-head">
      <div class="title" id="a-accent">{t('ap.accent')}</div>
      <div class="desc">{t('ap.accentDesc')}</div>
    </div>
    <div class="accent-controls">
      <div class="swatches" role="radiogroup" aria-labelledby="a-accent">
        {#each ACCENTS as a (a.id)}
          <button
            class="swatch"
            role="radio"
            aria-checked={preset?.id === a.id}
            title={t(`accent.${a.id}` as MessageKey)}
            aria-label={t(`accent.${a.id}` as MessageKey)}
            style:--sw={a.hex}
            onclick={() => {
              hexDraft = null;
              c.ui.accent = a.hex;
            }}
          >
            {#if preset?.id === a.id}<Icon name="check" />{/if}
          </button>
        {/each}
      </div>
      <label class="hex" class:invalid={!hexValid} class:custom={!preset}>
        <span class="hex-chip" style:--sw={accent}></span>
        <input
          class="field mono"
          value={hexShown}
          maxlength="7"
          spellcheck="false"
          aria-label={t('accent.custom')}
          oninput={(e) => (hexDraft = e.currentTarget.value)}
          onchange={commitHex}
          onkeydown={(e) => e.key === 'Enter' && commitHex()}
        />
      </label>
    </div>
  </div>
</Group>

<Group title={t('ap.groupPopup')}>
  <Row title={t('ap.closeOnBlur')} desc={t('ap.closeOnBlurDesc')} id="a-blur">
    <Toggle labelledby="a-blur" checked={c.ui.closeOnBlur} onchange={(v) => (c.ui.closeOnBlur = v)} />
  </Row>
</Group>

<Group title={t('ap.preview')}>
  <div class="preview" style:--content-size="{c.ui.fontSize}px">
    <div class="pv-sec pv-source">
      <div class="pv-label">{t('popup.source')}<span class="pv-meta">PaddleOCR · 86 ms</span></div>
      <p class="pv-text quiet" lang="en">{t('ap.previewSource')}</p>
    </div>
    <div class="pv-sec pv-primary">
      <div class="pv-label">
        <span class="pv-dot" aria-hidden="true"></span>
        <span>Google 翻译</span>
        <span class="pv-badge">{t('tr.primary')}</span>
        <span class="pv-meta">412 ms</span>
      </div>
      <p class="pv-text primary" lang="zh-CN">{t('ap.previewTarget')}<span class="pv-caret"></span></p>
    </div>
  </div>
</Group>

<style>
  .accent-row {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 8px 16px 12px;
  }
  .title {
    font-size: 14px;
    line-height: 20px;
  }
  .desc {
    margin-top: 1px;
    font-family: var(--font-small);
    font-size: var(--fs-caption);
    line-height: 16px;
    color: var(--text-2);
  }
  .accent-controls {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 12px;
  }
  .swatches {
    display: flex;
    gap: 8px;
  }
  .swatch {
    --c: light-dark(var(--sw), oklch(from var(--sw) 0.78 calc(c * 0.82) h));
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: var(--r-sm);
    background: var(--c);
    color: var(--on-accent);
    box-shadow: inset 0 0 0 1px light-dark(rgb(0 0 0 / 0.1), rgb(255 255 255 / 0.12));
    transition:
      transform var(--dur-1) var(--ease-std),
      box-shadow var(--dur-1) var(--ease-std);
  }
  .swatch:hover {
    transform: scale(1.08);
  }
  .swatch[aria-checked='true'] {
    box-shadow:
      0 0 0 2px var(--bg-card-primary),
      0 0 0 3.5px var(--c);
  }
  .hex {
    position: relative;
    display: block;
    width: 100px;
  }
  .hex .field {
    padding-left: 32px;
    text-transform: uppercase;
  }
  .hex-chip {
    position: absolute;
    left: 9px;
    top: 9px;
    width: 14px;
    height: 14px;
    border-radius: 4px;
    background: light-dark(var(--sw), oklch(from var(--sw) 0.78 calc(c * 0.82) h));
    box-shadow: inset 0 0 0 1px light-dark(rgb(0 0 0 / 0.12), rgb(255 255 255 / 0.14));
    pointer-events: none;
  }
  .hex.invalid .field {
    border-color: var(--critical);
    box-shadow: 0 0 0 2px var(--bg-error-soft);
  }

  /* A miniature of the popup's Prism Tactile card stack at the chosen size. */
  .preview {
    --lh: round(nearest, calc(var(--content-size) * 1.6), 2px);
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
    background: var(--bg-layer);
  }
  .pv-sec {
    border-radius: var(--r-md);
    border: 1px solid var(--stroke-card);
  }
  .pv-source {
    padding: 8px 12px 10px;
    background: var(--bg-recessed);
    border-color: var(--stroke-divider);
  }
  .pv-primary {
    padding: 10px 12px 12px;
    background: var(--bg-card-primary);
    border-color: var(--accent-border);
    box-shadow: var(--shadow-primary-card), var(--highlight-primary-top);
  }
  .pv-label {
    display: flex;
    gap: 6px;
    height: 22px;
    align-items: center;
    font-family: var(--font-small);
    font-size: var(--fs-caption);
    font-weight: 600;
    color: var(--text-2);
  }
  .pv-primary .pv-label {
    color: var(--text-1);
  }
  .pv-dot {
    width: 6px;
    height: 6px;
    border-radius: var(--r-pill);
    corner-shape: round;
    background: var(--accent);
    box-shadow: 0 0 0 2.5px var(--accent-glow);
  }
  .pv-badge {
    display: inline-flex;
    align-items: center;
    height: 18px;
    padding: 0 6px;
    border-radius: var(--r-pill);
    corner-shape: round;
    background: var(--accent-soft);
    color: var(--accent);
    font-size: 11px;
    line-height: 1;
    font-weight: 600;
  }
  .pv-meta {
    font-weight: 400;
    color: var(--text-3);
  }
  .pv-text {
    margin-top: 4px;
    font-size: var(--content-size);
    line-height: var(--lh);
  }
  .pv-text.quiet {
    color: var(--text-2);
  }
  .pv-text.primary {
    --s: calc(var(--content-size) + 1px);
    font-size: var(--s);
    line-height: round(nearest, calc(var(--s) * 1.6), 2px);
    color: var(--text-1);
  }
  .pv-caret {
    display: inline-block;
    width: 2px;
    height: 1.15em;
    margin-left: 2px;
    vertical-align: -0.2em;
    border-radius: var(--r-pill);
    corner-shape: round;
    background: var(--accent);
  }
</style>
