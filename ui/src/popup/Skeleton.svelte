<script lang="ts">
  let { widths }: { widths: number[] } = $props();
</script>

<div class="sk" aria-hidden="true">
  {#each widths as w, i (i)}
    <span class="sk-line" style:--w="{w}%"></span>
  {/each}
</div>
