<script>
  import { total, okCount } from './usage.js';

  /** `daily`: [{day, counts}] oldest first; the last `days` are drawn. */
  let { daily = [], days = 14 } = $props();

  const W = 70;
  const H = 18;
  const pts = $derived((daily || []).slice(-days));
  const max = $derived(Math.max(1, ...pts.map((d) => total(d.counts))));
  const bw = $derived(W / Math.max(1, pts.length));
</script>

<svg class="spark" viewBox="0 0 {W} {H}" width={W} height={H} aria-hidden="true">
  {#each pts as d, i (d.day)}
    {@const n = total(d.counts)}
    {@const h = n ? Math.max(1.5, (n / max) * H) : 0}
    {@const ok = n ? (h * okCount(d.counts)) / n : 0}
    <rect x={i * bw + 0.5} y={H - h} width={Math.max(1, bw - 1)} height={h - ok} class="fail" />
    <rect x={i * bw + 0.5} y={H - ok} width={Math.max(1, bw - 1)} height={ok} class="ok" />
  {/each}
  <line x1="0" x2={W} y1={H - 0.5} y2={H - 0.5} class="base" />
</svg>

<style>
  .spark {
    display: inline-block;
    vertical-align: middle;
  }

  .ok {
    fill: var(--info);
  }

  .fail {
    fill: var(--err);
    opacity: 0.75;
  }

  .base {
    stroke: var(--border);
  }
</style>
