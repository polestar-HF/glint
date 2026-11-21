<script lang="ts">
  import Icon from '../lib/Icon.svelte';
  import { fmtMs, t } from '../lib/i18n.svelte';
  import { htmlLang } from '../lib/langs';
  import { describeError } from '../lib/errors';
  import Skeleton from './Skeleton.svelte';
  import type { ResultView } from './session.svelte';

  interface Props {
    r: ResultView;
    primary: boolean;
    /** True when multiple translation engines are active, so the primary card shows its badge. */
    multi?: boolean;
    /** v0.2: add the first-token time to the meta. */
    showTimings?: boolean;
    lang: string;
    /** v0.2.2: the engine is a keyless free endpoint (rate-limit advice differs). */
    freeEndpoint?: boolean;
    oncopy: (text: string) => void;
    onretry: () => void;
    ontoggle: () => void;
  }
  let { r, primary, multi = false, showTimings = false, lang, freeEndpoint = false, oncopy, onretry, ontoggle }: Props = $props();

  const hasText = $derived(r.text.length > 0);
  const meta = $derived(
    showTimings && r.firstTokenMs != null
      ? `${t('tm.first')} ${fmtMs(r.firstTokenMs)} · ${fmtMs(r.elapsedMs)}`
      : fmtMs(r.elapsedMs),
  );
  const live = $derived(r.status === 'streaming' || (r.status === 'pending' && hasText));
  const err = $derived(r.status === 'error' ? describeError(r, freeEndpoint) : null);
</script>

<section
  class="sec result"
  class:primary
  class:has-error={r.status === 'error'}
  class:collapsed={r.collapsed}
  aria-busy={r.status === 'waiting' || r.status === 'pending' || live}
>
  <div class="sec-head">
    {#if primary && r.status !== 'error'}
      <span class="status-dot" class:live={live || r.status === 'pending'} aria-hidden="true"></span>
    {/if}
    <span class="sec-label">{r.engineName}</span>
    {#if primary && multi && r.status !== 'error'}
      <span class="primary-badge">{t('tr.primary')}</span>
    {/if}
    {#if r.status === 'done' && r.elapsedMs != null}
      <span class="meta tnum">{meta}</span>
    {/if}
    {#if r.collapsed && hasText}
      <button class="sec-peek" lang={htmlLang(lang)} onclick={ontoggle}>{r.text}</button>
    {/if}
    <span class="sec-actions">
      {#if r.status !== 'error'}
        <button class="icon-btn sm" title={t('popup.copy')} aria-label={t('popup.copy')} disabled={!hasText} onclick={() => oncopy(r.text)}>
          <Icon name="copy" />
        </button>
      {/if}
      <button
        class="icon-btn sm"
        title={r.collapsed ? t('popup.expand') : t('popup.collapse')}
        aria-label={r.collapsed ? t('popup.expand') : t('popup.collapse')}
        aria-expanded={!r.collapsed}
        onclick={ontoggle}
      >
        <Icon name={r.collapsed ? 'chevron-down' : 'chevron-up'} />
      </button>
    </span>
  </div>

  {#if !r.collapsed}
    {#if r.status === 'error'}
      <div class="err" role="alert">
        <span class="err-icon"><Icon name="alert" /></span>
        <span class="err-msg" title={err?.text === r.message ? r.message : `${err?.text}\n${r.message}`}>{err?.text}</span>
        <button class="text-btn" onclick={onretry}>{t('popup.retry')}</button>
      </div>
      {#if err?.hint}<p class="err-hint">{err.hint}</p>{/if}
    {:else if !hasText}
      <Skeleton widths={r.status === 'waiting' ? [56] : primary ? [100, 72] : [88]} />
    {:else}
      <p class="text" lang={htmlLang(lang)}>{r.text}{#if live}<span class="caret" aria-hidden="true"></span>{/if}</p>
    {/if}
  {/if}
</section>
