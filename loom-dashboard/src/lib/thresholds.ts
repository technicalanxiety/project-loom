/**
 * Shared threshold helpers for the dashboard.
 *
 * Three-tone health classification — `ok`, `warn`, `crit` — backed by the
 * design system's `--signal-*` token family (moss / saffron / madder).
 * The thresholds are deliberately hardcoded and conservative; if they
 * become wrong in production more than twice, promote them to a config.
 */

export type Tone = 'ok' | 'warn' | 'crit';

/**
 * Tone class names that match the App.css convention. Use these on
 * `.gauge`, `.stage-row`, `.queue-stat`, or `.inline-bar` to colour
 * the element by health.
 */
export const TONE_CLASS: Record<Tone, string> = {
  ok: 'tone-ok',
  warn: 'tone-warn',
  crit: 'tone-crit',
};

/** CPU% — warn ≥ 70, crit ≥ 90. */
export function cpuTone(pct: number): Tone {
  if (pct >= 90) return 'crit';
  if (pct >= 70) return 'warn';
  return 'ok';
}

/** Memory% (used / total) — warn ≥ 75, crit ≥ 90. */
export function memTone(usedMib: number, totalMib: number): Tone {
  if (totalMib === 0) return 'ok';
  const pct = (usedMib / totalMib) * 100;
  if (pct >= 90) return 'crit';
  if (pct >= 75) return 'warn';
  return 'ok';
}

/** Total p50 compilation latency (ms) — warn ≥ 500, crit ≥ 1000. */
export function latencyTone(ms: number | null | undefined): Tone {
  if (ms === null || ms === undefined) return 'ok';
  if (ms >= 1000) return 'crit';
  if (ms >= 500) return 'warn';
  return 'ok';
}

/** Pending-queue depth — warn ≥ 100, crit ≥ 500. */
export function queueTone(depth: number): Tone {
  if (depth >= 500) return 'crit';
  if (depth >= 100) return 'warn';
  return 'ok';
}

/** Failed-episode count — warn at 1, crit at 10. */
export function failedTone(count: number): Tone {
  if (count >= 10) return 'crit';
  if (count >= 1) return 'warn';
  return 'ok';
}

/**
 * Confidence (0–1) — inverse of latency: high is good.
 * warn ≤ 0.7, crit ≤ 0.5.
 */
export function confidenceTone(score: number): Tone {
  if (score <= 0.5) return 'crit';
  if (score <= 0.7) return 'warn';
  return 'ok';
}

/** Hot-tier utilisation% — warn ≥ 80, crit ≥ 95. */
export function utilizationTone(pct: number): Tone {
  if (pct >= 95) return 'crit';
  if (pct >= 80) return 'warn';
  return 'ok';
}

/**
 * Parser freshness, in milliseconds since the last ingestion.
 * fresh ≤ 24 h, aging ≤ 7 d, stale otherwise.
 */
export type Freshness = 'fresh' | 'aging' | 'stale';

const ONE_DAY_MS = 24 * 60 * 60 * 1000;
const ONE_WEEK_MS = 7 * ONE_DAY_MS;

export function freshness(lastIngestedAt: string | null): Freshness | null {
  if (!lastIngestedAt) return 'stale';
  const ts = Date.parse(lastIngestedAt);
  if (Number.isNaN(ts)) return null;
  const age = Date.now() - ts;
  if (age <= ONE_DAY_MS) return 'fresh';
  if (age <= ONE_WEEK_MS) return 'aging';
  return 'stale';
}

/**
 * Render an absolute timestamp as a relative label
 * ("3m ago", "2h ago", "4d ago", "3 weeks ago").
 * Falls back to a locale-formatted absolute string for ages > ~6 months.
 */
export function relativeTime(iso: string | null | undefined): string {
  if (!iso) return '—';
  const ts = Date.parse(iso);
  if (Number.isNaN(ts)) return iso;
  const diffSec = Math.round((Date.now() - ts) / 1000);
  if (diffSec < 60) return 'just now';
  if (diffSec < 3600) return `${Math.floor(diffSec / 60)}m ago`;
  if (diffSec < 86400) return `${Math.floor(diffSec / 3600)}h ago`;
  if (diffSec < 86400 * 7) return `${Math.floor(diffSec / 86400)}d ago`;
  if (diffSec < 86400 * 60) return `${Math.floor(diffSec / 86400 / 7)} weeks ago`;
  if (diffSec < 86400 * 365) return `${Math.floor(diffSec / 86400 / 30)} months ago`;
  return new Date(ts).toLocaleDateString();
}
