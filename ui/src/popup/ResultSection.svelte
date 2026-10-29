<script lang="ts">
  import Icon from '../lib/Icon.svelte';
  import { fmtMs, t } from '../lib/i18n.svelte';
  import { htmlLang } from '../lib/langs';
  import Skeleton from './Skeleton.svelte';
  import type { ResultView } from './session.svelte';

  interface Props {
    r: ResultView;
    primary: boolean;
    lang: string;
    oncopy: (text: string) => void;
    onretry: () => void;
    ontoggle: () => void;
  }
  let { r, primary, lang, oncopy, onretry, ontoggle }: Props = $props();

  const hasText = $derived(r.text.length > 0);
  const live = $derived(r.status === 'streaming' || (r.status === 'pending' && hasText));
</script>

<section class="sec result" class:primary class:collapsed={r.collapsed} aria-busy={r.status === 'waiting' || r.status === 'pending' || live}>
  <div class="sec-head">
    <span class="sec-label">{r.engineName}</span>
    {#if r.status === 'done' && r.elapsedMs != null}
      <span class="meta tnum">{fmtMs(r.elapsedMs)}</span>
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
        <span class="err-msg" title={r.message}>{r.message}</span>
        <button class="text-btn" onclick={onretry}>{t('popup.retry')}</button>
      </div>
    {:else if !hasText}
      <Skeleton widths={r.status === 'waiting' ? [56] : primary ? [100, 72] : [88]} />
    {:else}
      <p class="text" lang={htmlLang(lang)}>{r.text}{#if live}<span class="caret" aria-hidden="true"></span>{/if}</p>
    {/if}
  {/if}
</section>
