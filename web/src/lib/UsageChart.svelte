<script>
  import { t } from './i18n.svelte.js';
  import { ENDPOINTS, total, okCount, fmt, dayLabel } from './usage.js';

  /** `daily`: [{day, counts}] oldest first (the last 30 days). `height`: plot px. */
  let { daily = [], height = 140 } = $props();

  const H = 120;
  const W = 600;
  const GAP = 2;
  const values = $derived(daily.map((d) => total(d.counts)));
  const oks = $derived(daily.map((d) => okCount(d.counts)));
  const max = $derived(Math.max(1, ...values));
  // A round axis maximum: 1, 2 or 5 times a power of ten.
  const top = $derived.by(() => {
    const p = 10 ** Math.floor(Math.log10(max));
    return [1, 2, 5, 10].map((m) => m * p).find((v) => v >= max);
  });
  const bw = $derived(daily.length ? W / daily.length : W);
  const empty = $derived(values.every((v) => v === 0));
  const anyFail = $derived(values.some((v, i) => v > oks[i]));

  let hover = $state(null);
  let box = $state(null);

  const px = (v) => (v ? Math.max(2, (v / top) * H) : 0);

  /** Bar from `y0` (bottom) up `h`; rounded top corners when `round`. */
  function path(x, w, y0, h, round) {
    if (h <= 0) return '';
    const y = y0 - h;
    const r = round ? Math.min(4, w / 2, h) : 0;
    return `M${x},${y0}V${y + r}Q${x},${y} ${x + r},${y}H${x + w - r}Q${x + w},${y} ${x + w},${y + r}V${y0}Z`;
  }

  function move(e) {
    const rect = box.getBoundingClientRect();
    const i = Math.floor(((e.clientX - rect.left) / rect.width) * daily.length);
    hover = i >= 0 && i < daily.length ? { i, x: e.clientX - rect.left } : null;
  }
</script>

<div class="chart" bind:this={box} role="img" aria-label={t('usage.chartLabel')} onpointermove={move} onpointerleave={() => (hover = null)}>
  <div class="axis-top small muted">
    <span>{fmt(top)}</span>
    {#if anyFail}
      <span class="legend"><i class="sw ok"></i>{t('usage.ok')}<i class="sw fail"></i>{t('usage.failed')}</span>
    {/if}
  </div>
  <svg viewBox="0 0 {W} {H}" preserveAspectRatio="none" aria-hidden="true" style="height: {height}px">
    <line x1="0" x2={W} y1={H / 2} y2={H / 2} class="grid" />
    <line x1="0" x2={W} y1="0" y2="0" class="grid" />
    {#each daily as d, i (d.day)}
      {@const all = px(values[i])}
      {@const ok = values[i] ? (all * oks[i]) / values[i] : 0}
      {@const x = i * bw + GAP / 2}
      <g class:dim={hover && hover.i !== i}>
        <path d={path(x, bw - GAP, H, ok, ok >= all)} class="bar" />
        <path d={path(x, bw - GAP, H - ok, all - ok, true)} class="bar fail" />
      </g>
    {/each}
    <line x1="0" x2={W} y1={H} y2={H} class="base" />
  </svg>
  <div class="axis-x small muted">
    {#if daily.length}
      <span>{dayLabel(daily[0].day)}</span>
      <span>{dayLabel(daily[Math.floor(daily.length / 2)].day)}</span>
      <span>{t('usage.today')}</span>
    {/if}
  </div>
  {#if empty}
    <div class="empty-note small muted">{t('usage.none')}</div>
  {/if}
  {#if hover}
    {@const d = daily[hover.i]}
    <div class="tip" style="left: {Math.min(Math.max(hover.x, 90), (box?.clientWidth || 0) - 90)}px">
      <div class="tip-head">{dayLabel(d.day, { weekday: 'short', month: 'short', day: 'numeric' })} · {fmt(values[hover.i])}</div>
      {#each ENDPOINTS as ep (ep)}
        {#if d.counts[ep]}
          <div class="tip-row">
            <span class="mono">/{ep}</span>
            <span>{fmt(d.counts[ep].ok)}{#if d.counts[ep].fail} <span class="fail-txt">+{fmt(d.counts[ep].fail)} {t('usage.failed')}</span>{/if}</span>
          </div>
        {/if}
      {:else}
        <div class="muted">{t('usage.noneDay')}</div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .chart {
    position: relative;
    padding-top: 18px;
    touch-action: pan-y;
  }

  svg {
    display: block;
    width: 100%;
    overflow: visible;
  }

  .bar {
    fill: var(--info);
  }

  .bar.fail {
    fill: var(--err);
    opacity: 0.75;
  }

  g {
    transition: opacity 0.12s;
  }

  g.dim {
    opacity: 0.45;
  }

  .grid {
    stroke: var(--border);
    stroke-dasharray: 3 4;
    vector-effect: non-scaling-stroke;
  }

  .base {
    stroke: var(--muted);
    stroke-opacity: 0.5;
    vector-effect: non-scaling-stroke;
  }

  .axis-top {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    display: flex;
    justify-content: space-between;
  }

  .legend {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }

  .sw {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 2px;
    margin-left: 6px;
  }

  .sw.ok {
    background: var(--info);
  }

  .sw.fail {
    background: var(--err);
    opacity: 0.75;
  }

  .axis-x {
    display: flex;
    justify-content: space-between;
    margin-top: 4px;
  }

  .empty-note {
    position: absolute;
    inset: 18px 0 24px;
    display: grid;
    place-items: center;
  }

  .tip {
    position: absolute;
    top: 0;
    transform: translateX(-50%);
    min-width: 170px;
    padding: 8px 10px;
    border-radius: 9px;
    background: var(--surface);
    border: 1px solid var(--border);
    box-shadow: 0 6px 24px rgba(0, 0, 0, 0.14);
    font-size: 12px;
    pointer-events: none;
    z-index: 5;
  }

  .tip-head {
    font-weight: 600;
    margin-bottom: 4px;
  }

  .tip-row {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    font-variant-numeric: tabular-nums;
  }

  .fail-txt {
    color: var(--err);
  }
</style>
