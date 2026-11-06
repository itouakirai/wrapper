<script>
  import { t } from './i18n.svelte.js';
  import { ENDPOINTS, RANGES, inRange, total, failCount, okRate, rateClass, fmt } from './usage.js';

  /**
   * `summary`: {today, total, daily, unserved?} from /api/stats, the public
   * overview or one account. `range`: one of RANGES (bindable).
   */
  let { summary, range = $bindable('today') } = $props();

  const counts = $derived(inRange(summary, range));
  const unserved = $derived(total(summary?.unserved));
</script>

<div class="seg" role="tablist">
  {#each RANGES as r (r)}
    <button role="tab" class:on={range === r} aria-selected={range === r} onclick={() => (range = r)}>{t(`usage.range.${r}`)}</button>
  {/each}
</div>

<div class="tiles">
  {#each [null, ...ENDPOINTS] as ep (ep ?? '*')}
    {@const n = total(counts, ep ?? undefined)}
    {@const fail = failCount(counts, ep ?? undefined)}
    {@const rate = okRate(counts, ep ?? undefined)}
    <div class="tile" class:all={!ep} class:idle={!n}>
      <div class="name" class:mono={!!ep}>{ep ? `/${ep}` : t('usage.all')}</div>
      <div class="big">{fmt(n)}</div>
      <div class="sub">
        {#if rate === null}
          —
        {:else}
          <span class="rate {rateClass(rate)}">{t('usage.okRate', { p: rate })}</span>{#if fail}{' · '}<span class="fail">{t('usage.failN', { n: fmt(fail) })}</span>{/if}
        {/if}
      </div>
    </div>
  {/each}
</div>
{#if unserved && range === 'total'}
  <div class="small muted unserved">{t('usage.unserved', { n: fmt(unserved) })}</div>
{/if}

<style>
  .seg {
    display: inline-flex;
    background: var(--surface-2);
    border-radius: 9px;
    padding: 3px;
    gap: 2px;
    margin-bottom: 12px;
    max-width: 100%;
    overflow-x: auto;
  }

  .seg button {
    border: none;
    background: transparent;
    color: var(--muted);
    height: 26px;
    padding: 0 12px;
    font-size: 12.5px;
    border-radius: 7px;
    box-shadow: none;
    white-space: nowrap;
  }

  .seg button.on {
    background: var(--surface);
    color: var(--text);
    font-weight: 600;
    box-shadow: var(--shadow);
  }

  .tiles {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
    gap: 10px;
  }

  .tile {
    background: var(--surface-2);
    border-radius: 10px;
    padding: 10px 12px;
    min-width: 0;
  }

  .tile.all {
    background: var(--info-soft);
  }

  .tile.idle .big {
    color: var(--muted);
  }

  .name {
    font-size: 12px;
    color: var(--muted);
  }

  .big {
    font-size: 22px;
    font-weight: 650;
    letter-spacing: -0.02em;
    font-variant-numeric: tabular-nums;
  }

  .sub {
    font-size: 11.5px;
    color: var(--muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .rate.ok {
    color: var(--ok);
  }

  .rate.warn {
    color: var(--warn);
  }

  .rate.err,
  .fail {
    color: var(--err);
  }

  .unserved {
    margin-top: 8px;
  }
</style>
