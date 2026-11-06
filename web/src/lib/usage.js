// Helpers for the request counters (`stats.json` on the agent).

export const ENDPOINTS = ['m3u8', 'key', 'lyrics', 'webplayback', 'license'];

/** Ranges the tiles and tables can show. */
export const RANGES = ['today', 'week', 'month', 'total'];

/** Requests in an endpoint -> {ok, fail} map (optionally one endpoint). */
export function total(counts, endpoint) {
  if (!counts) return 0;
  const list = endpoint ? [counts[endpoint]] : Object.values(counts);
  return list.reduce((n, c) => n + (c ? c.ok + c.fail : 0), 0);
}

export function okCount(counts, endpoint) {
  if (!counts) return 0;
  const list = endpoint ? [counts[endpoint]] : Object.values(counts);
  return list.reduce((n, c) => n + (c ? c.ok : 0), 0);
}

export function failCount(counts, endpoint) {
  return total(counts, endpoint) - okCount(counts, endpoint);
}

/** Success share in percent, or null without requests. */
export function okRate(counts, endpoint) {
  const n = total(counts, endpoint);
  return n ? Math.round((okCount(counts, endpoint) / n) * 1000) / 10 : null;
}

/** Adds up several endpoint -> {ok, fail} maps. */
export function merge(list) {
  const out = {};
  for (const counts of list) {
    for (const [ep, c] of Object.entries(counts || {})) {
      const o = (out[ep] ??= { ok: 0, fail: 0 });
      o.ok += c.ok;
      o.fail += c.fail;
    }
  }
  return out;
}

/** The counters of the last `n` days of a `daily` series (oldest first). */
export const lastDays = (daily, n) => merge((daily || []).slice(-n).map((d) => d.counts));

/**
 * Counters of `range` from a record with `today`, `total` and `daily`
 * (the overall summary or one account).
 */
export function inRange(rec, range) {
  if (!rec) return {};
  switch (range) {
    case 'today':
      return rec.today || {};
    case 'week':
      return lastDays(rec.daily, 7);
    case 'month':
      return lastDays(rec.daily, 30);
    default:
      return rec.total || {};
  }
}

export const fmt = (n) => Number(n || 0).toLocaleString();

/** Compact count for tight spots: 1234 -> 1.2k. */
export function short(n) {
  n = Number(n || 0);
  if (n < 1000) return String(n);
  if (n < 1e6) return `${Math.round(n / 100) / 10}k`;
  return `${Math.round(n / 1e5) / 10}M`;
}

/** Day number (days since 1970-01-01 UTC) -> short date. */
export function dayLabel(day, opts = { month: 'short', day: 'numeric' }) {
  return new Date(day * 86400000).toLocaleDateString(undefined, { ...opts, timeZone: 'UTC' });
}

/** CSS class for a success rate. */
export const rateClass = (p) => (p === null ? '' : p >= 95 ? 'ok' : p >= 80 ? 'warn' : 'err');
