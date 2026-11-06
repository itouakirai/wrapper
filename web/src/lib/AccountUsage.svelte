<script>
  import { t, duration } from './i18n.svelte.js';
  import { ENDPOINTS, inRange, total, okCount, failCount, okRate, rateClass, fmt } from './usage.js';
  import UsageChart from './UsageChart.svelte';

  /** `usage`: one account's {today, total, daily, lastUsed, lastOk}; `since`: added at (ms). */
  let { usage, since = 0, now = Date.now() } = $props();

  const cols = ['today', 'week', 'month', 'total'];
  const rows = $derived(
    [...ENDPOINTS, ...Object.keys(usage?.total || {}).filter((e) => !ENDPOINTS.includes(e))].filter(
      (ep) => total(usage?.total, ep) || total(usage?.today, ep),
    ),
  );
  const ago = (ts) => (now - ts < 60000 ? t('common.justNow') : t('common.ago', { time: duration(now - ts) }));
</script>

<div class="detail">
  <div class="row meta small muted">
    {#if since}<span>{t('usage.addedAt', { date: new Date(since).toLocaleString() })}</span>{/if}
    <span>{t('pub.lastUsed')}: {usage?.lastUsed ? ago(usage.lastUsed) : '—'}</span>
    <span>{t('usage.lastOk')}: {usage?.lastOk ? ago(usage.lastOk) : '—'}</span>
  </div>

  {#if rows.length === 0}
    <div class="muted small">{t('usage.noneAccount')}</div>
  {:else}
    <div class="table-wrap">
      <table>
        <thead>
          <tr>
            <th>{t('usage.endpoint')}</th>
            {#each cols as c (c)}<th class="num">{t(`usage.range.${c}`)}</th>{/each}
            <th class="num">{t('usage.ok')}</th>
            <th class="num">{t('usage.failed')}</th>
            <th class="num">{t('usage.rate')}</th>
          </tr>
        </thead>
        <tbody>
          {#each [...rows, null] as ep (ep ?? '*')}
            {@const rate = okRate(usage.total, ep ?? undefined)}
            <tr class:sum={!ep}>
              <td class:mono={!!ep}>{ep ? `/${ep}` : t('usage.all')}</td>
              {#each cols as c (c)}
                {@const counts = inRange(usage, c)}
                {@const f = failCount(counts, ep ?? undefined)}
                <td class="num">
                  {fmt(total(counts, ep ?? undefined))}{#if f}<span class="fail"> ({fmt(f)})</span>{/if}
                </td>
              {/each}
              <td class="num">{fmt(okCount(usage.total, ep ?? undefined))}</td>
              <td class="num" class:fail={failCount(usage.total, ep ?? undefined) > 0}>{fmt(failCount(usage.total, ep ?? undefined))}</td>
              <td class="num rate {rateClass(rate)}">{rate === null ? '—' : `${rate}%`}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
    <div class="hint small muted">{t('usage.detailHint')}</div>
  {/if}

  <UsageChart daily={usage?.daily || []} height={90} />
</div>

<style>
  .detail {
    display: flex;
    flex-direction: column;
    gap: 10px;
    white-space: normal;
    min-width: 0;
  }

  .meta {
    gap: 4px 16px;
    flex-wrap: wrap;
  }

  .table-wrap {
    overflow-x: auto;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12.5px;
  }

  th,
  td {
    text-align: left;
    padding: 5px 8px;
    border-top: 1px solid var(--border);
    white-space: nowrap;
  }

  th {
    border-top: none;
    font-weight: 500;
    color: var(--muted);
    font-size: 11.5px;
  }

  tr.sum td {
    font-weight: 600;
  }

  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  .fail {
    color: var(--err);
  }

  .rate.ok {
    color: var(--ok);
  }

  .rate.warn {
    color: var(--warn);
  }

  .rate.err {
    color: var(--err);
  }

  .hint {
    margin-top: -4px;
  }
</style>
