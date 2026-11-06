<script>
  import { pub } from '../lib/api.js';
  import { t, locale, setLang, duration } from '../lib/i18n.svelte.js';
  import { theme, toggleTheme } from '../lib/theme.svelte.js';
  import { owned, forget } from '../lib/owned.svelte.js';
  import { total, failCount, okRate, rateClass, inRange, merge, fmt } from '../lib/usage.js';
  import Icon from '../lib/Icon.svelte';
  import Sparkline from '../lib/Sparkline.svelte';
  import AccountUsage from '../lib/AccountUsage.svelte';
  import CopyField from '../lib/CopyField.svelte';
  import UsageTiles from '../lib/UsageTiles.svelte';
  import UsageChart from '../lib/UsageChart.svelte';
  import AccountBadge from '../lib/AccountBadge.svelte';
  import PublicLogin from '../lib/PublicLogin.svelte';

  /** `admin`: the visitor may open the panel; `onadmin`: sign in instead. */
  let { admin = false, onadmin } = $props();

  let ov = $state(null);
  let error = $state('');
  let loading = $state(true);
  let now = $state(Date.now());

  async function load() {
    try {
      ov = await pub.overview();
      error = '';
    } catch (e) {
      error = e.message;
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    load();
    const a = setInterval(load, 15000);
    const b = setInterval(() => (now = Date.now()), 30000);
    return () => {
      clearInterval(a);
      clearInterval(b);
    };
  });

  const serving = $derived(ov?.accounts?.filter((a) => a.state === 'running').length ?? 0);
  const online = $derived(!!ov && serving > 0);
  const mine = $derived(new Set(owned.list.map((o) => o.ref)));
  const ago = (ts) => (now - ts < 60000 ? t('common.justNow') : t('common.ago', { time: duration(now - ts) }));

  // Log out: with a remembered key, or with an Apple ID + key typed in.
  let busy = $state('');
  let manual = $state({ username: '', key: '' });
  let manualOpen = $state(false);

  async function logout(username, key) {
    if (!confirm(t('pub.confirmLogout'))) return;
    busy = username;
    try {
      await pub.logout(username, key);
      forget(username);
      manual = { username: '', key: '' };
      manualOpen = false;
      await load();
    } catch (e) {
      alert(e.message);
    } finally {
      busy = '';
    }
  }

  const accountFor = (ref) => ov?.accounts?.find((a) => a.ref === ref);

  // Range shared by the tiles and the account table.
  let range = $state('today');

  // Expanded rows (per-account detail).
  let open = $state(new Set());
  function toggle(ref) {
    const next = new Set(open);
    next.has(ref) ? next.delete(ref) : next.add(ref);
    open = next;
  }
  function reveal(ref) {
    if (!open.has(ref)) toggle(ref);
    requestAnimationFrame(() => document.getElementById(`acc-${ref}`)?.scrollIntoView({ behavior: 'smooth', block: 'center' }));
  }

  // Sorting: the column and direction (desc for numbers by default).
  let sort = $state({ key: 'requests', desc: true });
  function sortBy(key) {
    sort = sort.key === key ? { key, desc: !sort.desc } : { key, desc: key !== 'name' };
  }
  const arrow = (key) => (sort.key === key ? (sort.desc ? ' ↓' : ' ↑') : '');
  const STATE_ORDER = { running: 0, starting: 1, backoff: 2, loggingIn: 3, stopped: 4, needsLogin: 5 };
  function sortValue(a, key) {
    const c = inRange(a.usage, range);
    switch (key) {
      case 'name':
        return a.name;
      case 'fail':
        return failCount(c);
      case 'rate':
        return okRate(c) ?? -1;
      case 'total':
        return total(a.usage.total);
      case 'lastUsed':
        return a.usage.lastUsed ?? 0;
      default:
        return total(c);
    }
  }
  const sorted = $derived.by(() => {
    const list = [...(ov?.accounts ?? [])];
    const dir = sort.desc ? -1 : 1;
    return list.sort((x, y) => {
      const a = sortValue(x, sort.key);
      const b = sortValue(y, sort.key);
      const d = typeof a === 'string' ? a.localeCompare(b) : a - b;
      return d * dir || (STATE_ORDER[x.state] ?? 9) - (STATE_ORDER[y.state] ?? 9) || x.name.localeCompare(y.name);
    });
  });
  const sumRange = $derived(merge((ov?.accounts ?? []).map((a) => inRange(a.usage, range))));
  const sumTotal = $derived(merge((ov?.accounts ?? []).map((a) => a.usage.total)));
</script>

<div class="page">
  <header class="row top">
    <img src="/favicon.svg" alt="" width="28" height="28" />
    <div>
      <div class="name">wrapper</div>
      <div class="small muted">{t('pub.subtitle')}</div>
    </div>
    <span class="spacer"></span>
    <button class="ghost icon" title="Theme" onclick={toggleTheme}><Icon name={theme.name === 'dark' ? 'sun' : 'moon'} /></button>
    <button class="ghost small" onclick={() => setLang(locale.lang === 'zh' ? 'en' : 'zh')}>
      <Icon name="globe" size={16} />{locale.lang === 'zh' ? 'English' : '中文'}
    </button>
    {#if admin}
      <a class="btn small" href="#/dashboard">{t('pub.toPanel')}</a>
    {:else}
      <button class="small" onclick={onadmin}><Icon name="settings" size={14} />{t('pub.admin')}</button>
    {/if}
  </header>

  {#if loading}
    <div class="card empty"><span class="spinner"></span></div>
  {:else if error}
    <div class="card empty">
      <h3>{t('pub.offline')}</h3>
      <p class="muted">{error}</p>
    </div>
  {:else if ov && !ov.publicMode}
    <div class="card empty">
      <h3>{t('pub.closed')}</h3>
      <p class="muted">{t('pub.closedHint')}</p>
    </div>
  {:else if ov}
    {#if ov.notice}<div class="callout info notice">{ov.notice}</div>{/if}

    <div class="grid hero">
      <section class="card">
        <div class="muted small">{t('pub.status')}</div>
        <div class="row status">
          <span class="badge {online ? 'ok' : ov.ready ? 'err' : 'info pulse'}">
            {online ? t('pub.online') : ov.ready ? t('pub.noAccounts') : t('pub.starting')}
          </span>
        </div>
        <div class="small muted">{t('pub.serving', { n: serving })}</div>
      </section>
      <section class="card">
        <div class="muted small">{t('pub.slots')}</div>
        <div class="big">
          {ov.activeAccounts}<span class="muted"> / {ov.maxAccounts || '∞'}</span>
        </div>
        <div class="small muted">{ov.accepting ? t('pub.accepting') : t('pub.full')}</div>
      </section>
      <section class="card">
        <div class="muted small">{t('dash.regions')}</div>
        <div class="row regions">
          {#each ov.regions as r (r)}<span class="badge plain info">{r.toUpperCase()}</span>{:else}<span class="muted">{t('dash.noRegions')}</span>{/each}
        </div>
      </section>
    </div>

    <section class="card">
      <div class="card-title"><h2>{t('dash.endpoint')}</h2></div>
      <CopyField value={location.origin} />
      <div class="hint">{t('pub.endpointHint')}</div>
    </section>

    <section class="card">
      <div class="card-title">
        <h2>{t('usage.title')}</h2>
        <span class="small muted">{t('usage.since', { date: new Date(ov.stats.since).toLocaleDateString() })}</span>
      </div>
      <UsageTiles summary={ov.stats} bind:range />
      <div class="chart-gap"></div>
      <UsageChart daily={ov.stats.daily} />
    </section>

    <section class="card" id="accounts">
      <div class="card-title">
        <h2>{t('nav.accounts')}</h2>
        <span class="small muted">{t('pub.accountsCount', { n: ov.accounts.length, running: serving })}</span>
      </div>
      {#if ov.accounts.length === 0}
        <div class="muted small">{t('pub.noneYet')}</div>
      {:else}
        <div class="small muted table-note">{t('pub.tableNote', { range: t(`usage.range.${range}`) })}</div>
        <div class="table-wrap">
          <table>
            <thead>
              <tr>
                <th class="sortable" onclick={() => sortBy('name')}>{t('pub.account')}{arrow('name')}</th>
                <th>{t('acc.region')}</th>
                <th>{t('pub.state')}</th>
                <th class="num sortable" onclick={() => sortBy('requests')}>{t('usage.requests')}{arrow('requests')}</th>
                <th class="num sortable" onclick={() => sortBy('fail')}>{t('usage.failed')}{arrow('fail')}</th>
                <th class="num sortable" onclick={() => sortBy('rate')}>{t('usage.rate')}{arrow('rate')}</th>
                <th>{t('usage.trend')}</th>
                <th class="num sortable" onclick={() => sortBy('total')}>{t('usage.total')}{arrow('total')}</th>
                <th class="sortable" onclick={() => sortBy('lastUsed')}>{t('pub.lastUsed')}{arrow('lastUsed')}</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {#each sorted as a (a.ref)}
                {@const c = inRange(a.usage, range)}
                {@const rate = okRate(c)}
                {@const f = failCount(c)}
                <tr class="acc" class:mine={mine.has(a.ref)} class:open={open.has(a.ref)} id="acc-{a.ref}" onclick={() => toggle(a.ref)}>
                  <td class="mono">{a.name}{#if mine.has(a.ref)} <span class="badge plain info">{t('pub.yours')}</span>{/if}</td>
                  <td>{(a.region || '—').toUpperCase()}</td>
                  <td><AccountBadge account={{ state: a.state, degraded: a.degraded, health: { verdict: a.verdict } }} /></td>
                  <td class="num strong">{fmt(total(c))}</td>
                  <td class="num" class:fail={f > 0}>{fmt(f)}</td>
                  <td class="num rate {rateClass(rate)}">{rate === null ? '—' : `${rate}%`}</td>
                  <td><Sparkline daily={a.usage.daily} /></td>
                  <td class="num">{fmt(total(a.usage.total))}</td>
                  <td class="muted small">{a.usage.lastUsed ? ago(a.usage.lastUsed) : '—'}</td>
                  <td class="chev">{open.has(a.ref) ? '▾' : '▸'}</td>
                </tr>
                {#if open.has(a.ref)}
                  <tr class="detail-row">
                    <td colspan="10"><AccountUsage usage={a.usage} since={a.since} {now} /></td>
                  </tr>
                {/if}
              {/each}
            </tbody>
            {#if ov.accounts.length > 1}
              {@const c = sumRange}
              {@const rate = okRate(c)}
              <tfoot>
                <tr>
                  <td colspan="3">{t('pub.sumAccounts')}</td>
                  <td class="num strong">{fmt(total(c))}</td>
                  <td class="num" class:fail={failCount(c) > 0}>{fmt(failCount(c))}</td>
                  <td class="num rate {rateClass(rate)}">{rate === null ? '—' : `${rate}%`}</td>
                  <td></td>
                  <td class="num">{fmt(total(sumTotal))}</td>
                  <td colspan="2"></td>
                </tr>
              </tfoot>
            {/if}
          </table>
        </div>
      {/if}
    </section>

    <div class="grid two">
      <section class="card">
        <div class="card-title"><h2>{t('pub.addTitle')}</h2></div>
        <!-- Stays mounted when the server fills up, so a login waiting for
             its code is not lost. -->
        {#if !ov.accepting}
          <div class="callout warn full">{t('pub.fullHint', { n: ov.maxAccounts })}</div>
        {/if}
        <PublicLogin ondone={load} />
      </section>

      <section class="card">
        <div class="card-title"><h2>{t('pub.mineTitle')}</h2></div>
        {#if owned.list.length === 0}
          <div class="muted small">{t('pub.mineEmpty')}</div>
        {:else}
          <ul class="mine-list">
            {#each owned.list as o (o.username)}
              {@const acc = accountFor(o.ref)}
              <li>
                <div class="grow">
                  <div class="mono">{o.name}</div>
                  <div class="small muted">
                    {#if acc}
                      {@const rate = okRate(acc.usage.total)}
                      {t(`acc.state.${acc.state}`)} · {t('usage.range.today')} {fmt(total(acc.usage.today))} · {t('usage.totalN', { n: fmt(total(acc.usage.total)) })}{#if rate !== null}{' · '}<span class="rate {rateClass(rate)}">{t('usage.okRate', { p: rate })}</span>{/if}
                    {:else}{t('pub.gone')}{/if}
                  </div>
                </div>
                {#if acc}
                  <button class="small ghost" onclick={() => reveal(o.ref)}>{t('pub.details')}</button>
                  <button class="small danger" disabled={busy === o.username} onclick={() => logout(o.username, o.key)}>
                    <Icon name="trash" size={14} />{t('pub.logout')}
                  </button>
                {:else}
                  <button class="small ghost" onclick={() => forget(o.username)}>{t('pub.forget')}</button>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
        <button class="ghost small manual-toggle" onclick={() => (manualOpen = !manualOpen)}>
          {manualOpen ? '▾' : '▸'} {t('pub.logoutWithKey')}
        </button>
        {#if manualOpen}
          <form class="stack" onsubmit={(e) => { e.preventDefault(); logout(manual.username.trim(), manual.key.trim()); }}>
            <input placeholder={t('login.username')} bind:value={manual.username} autocomplete="username" required />
            <input placeholder={t('pub.key')} bind:value={manual.key} class="mono" required />
            <div class="row end">
              <button class="danger" type="submit" disabled={!!busy || !manual.username.trim() || !manual.key.trim()}>{t('pub.logout')}</button>
            </div>
          </form>
        {/if}
      </section>
    </div>

    <p class="foot small muted">wrapper {ov.version}</p>
  {/if}
</div>

<style>
  .page {
    max-width: 1080px;
    margin: 0 auto;
    padding: 20px 16px 48px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  /* Wide content (the table) scrolls inside its card instead of widening the page. */
  .page > * {
    min-width: 0;
  }

  .top {
    gap: 6px 10px;
  }

  .top .name {
    font-weight: 700;
    font-size: 16px;
  }

  .notice {
    white-space: pre-wrap;
  }

  .hero {
    grid-template-columns: repeat(3, 1fr);
  }

  .status {
    margin: 6px 0;
  }

  .big {
    font-size: 28px;
    font-weight: 650;
    letter-spacing: -0.02em;
  }

  .regions {
    margin-top: 10px;
  }

  .chart-gap {
    height: 18px;
  }

  .table-wrap {
    overflow-x: auto;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }

  th,
  td {
    text-align: left;
    padding: 8px 10px;
    border-top: 1px solid var(--border);
    white-space: nowrap;
  }

  th {
    border-top: none;
    font-weight: 500;
    color: var(--muted);
    font-size: 12px;
  }

  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  tr.mine td {
    background: var(--info-soft);
  }

  tr.acc {
    cursor: pointer;
  }

  tr.acc:hover td {
    background: var(--surface-2);
  }

  tr.acc.open td {
    border-bottom: none;
  }

  tr.detail-row td {
    border-top: none;
    padding: 4px 10px 16px;
    white-space: normal;
  }

  tfoot td {
    font-weight: 600;
    border-top: 2px solid var(--border);
  }

  th.sortable {
    cursor: pointer;
    user-select: none;
  }

  th.sortable:hover {
    color: var(--text);
  }

  .strong {
    font-weight: 600;
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

  .chev {
    color: var(--muted);
    width: 1%;
  }

  .table-note {
    margin: -4px 0 8px;
  }

  .two {
    grid-template-columns: 1fr 1fr;
  }

  .mine-list {
    list-style: none;
    margin: 0 0 8px;
    padding: 0;
  }

  .mine-list li {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 0;
    border-top: 1px solid var(--border);
  }

  .mine-list li:first-child {
    border-top: none;
  }

  .grow {
    flex: 1;
    min-width: 0;
  }

  .manual-toggle {
    padding: 0 4px;
  }

  .end {
    justify-content: flex-end;
  }

  .full {
    margin-bottom: 12px;
  }

  .foot {
    text-align: center;
  }

  a.btn {
    height: 28px;
    font-size: 12.5px;
    padding: 0 10px;
  }

  .empty h3 {
    color: var(--text);
    margin-bottom: 6px;
  }

  @media (max-width: 760px) {
    .hero,
    .two {
      grid-template-columns: 1fr;
    }

    .top .small.muted {
      display: none;
    }
  }
</style>
