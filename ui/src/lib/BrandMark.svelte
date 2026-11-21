<script lang="ts">
  let { size = 24 }: { size?: number } = $props();
  const uid = `gm-${Math.random().toString(36).slice(2, 8)}`;
</script>

<svg
  class="brand-mark"
  width={size}
  height={size}
  viewBox="0 0 1024 1024"
  aria-hidden="true"
  focusable="false"
>
  <defs>
    <linearGradient id="{uid}-bg" x1="0" y1="64" x2="0" y2="960" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#35363B" />
      <stop offset=".5" stop-color="#28292D" />
      <stop offset="1" stop-color="#191A1E" />
    </linearGradient>
    <linearGradient id="{uid}-tl" x1="512" y1="317" x2="430" y2="512" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#FFFFFF" />
      <stop offset="1" stop-color="#C5FDF6" />
    </linearGradient>
    <linearGradient id="{uid}-tr" x1="512" y1="317" x2="700" y2="512" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#BFD6DD" />
      <stop offset=".4" stop-color="#8AE6FF" />
      <stop offset="1" stop-color="#F2FCFF" />
    </linearGradient>
    <linearGradient id="{uid}-bl" x1="317" y1="512" x2="512" y2="707" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#FCFEFF" />
      <stop offset="1" stop-color="#95E7FF" />
    </linearGradient>
    <linearGradient id="{uid}-br" x1="580" y1="512" x2="512" y2="707" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#1EAAD4" />
      <stop offset="1" stop-color="#75C5F2" />
    </linearGradient>
  </defs>
  <path
    d="M 324 64 H 700 C 866 64 960 158 960 324 V 700 C 960 866 866 960 700 960 H 324 C 158 960 64 866 64 700 V 324 C 64 158 158 64 324 64 Z"
    fill="url(#{uid}-bg)"
  />
  <g fill="none" stroke="#FFFFFF" stroke-width="62" stroke-linecap="round">
    <path d="M 234 362 V 278 A 44 44 0 0 1 278 234 H 362" />
    <path d="M 662 234 H 746 A 44 44 0 0 1 790 278 V 362" />
    <path d="M 234 662 V 746 A 44 44 0 0 0 278 790 H 362" />
    <path d="M 662 790 H 746 A 44 44 0 0 0 790 746 V 662" />
  </g>
  <path d="M 512 318 L 706 512 L 512 706 L 318 512 Z" fill="#8CE6FF" />
  <path d="M 512 317 L 512 512 L 317 512 Z" fill="url(#{uid}-tl)" />
  <path d="M 512 317 L 707 512 L 512 512 Z" fill="url(#{uid}-tr)" />
  <path d="M 317 512 L 512 512 L 512 707 Z" fill="url(#{uid}-bl)" />
  <path d="M 512 512 L 707 512 L 512 707 Z" fill="url(#{uid}-br)" />
</svg>

<style>
  .brand-mark {
    display: block;
    flex: none;
    filter: drop-shadow(0 1px 2px light-dark(rgb(15 23 42 / 0.14), rgb(0 0 0 / 0.4)));
  }
</style>
