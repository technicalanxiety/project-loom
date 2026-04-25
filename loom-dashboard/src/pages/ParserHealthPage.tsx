/**
 * Parser health page.
 *
 * One row per (parser_version, parser_source_schema) pair across all
 * vendor_import episodes. The page's job: surface stale parsers at a
 * glance. Each row shows a freshness pill (fresh ≤ 1d / aging ≤ 7d /
 * stale > 7d) backed by `freshness()` from `lib/thresholds.ts`. Rows
 * sort stale-first so the operator's eye lands on what needs attention.
 *
 * Schema-assertion failures are not displayed here — bootstrap parsers
 * fail loud at the CLI boundary and never write episodes on failure, so
 * absence of a recent `last_ingested_at` is the primary signal. See
 * `bootstrap/README.md`.
 */
import type React from 'react';
import { getParserHealthMetrics } from '../api/client';
import { useApi } from '../hooks/useApi';
import { type Freshness, freshness, relativeTime } from '../lib/thresholds';
import type { ParserHealthRow } from '../types';

const FRESHNESS_PILL: Record<Freshness, string> = {
  fresh: 'pill-success',
  aging: 'pill-warning',
  stale: 'pill-error',
};
const FRESHNESS_LABEL: Record<Freshness, string> = {
  fresh: 'fresh',
  aging: 'aging',
  stale: 'stale',
};
const FRESHNESS_RANK: Record<Freshness, number> = { stale: 0, aging: 1, fresh: 2 };

export const ParserHealthPage: React.FC = () => {
  const { data, loading, error } = useApi(getParserHealthMetrics);

  if (loading) {
    return (
      <>
        <PageHeader />
        <div className="loading">Loading parser health…</div>
      </>
    );
  }
  if (error) {
    return (
      <>
        <PageHeader />
        <div className="error">Error: {error}</div>
      </>
    );
  }
  if (!data || data.parsers.length === 0) {
    return (
      <>
        <PageHeader />
        <div className="empty-state">
          <h3>No vendor-import episodes yet</h3>
          <p>
            Run a parser under <code>bootstrap/</code> to populate this view. Parsers that fail
            schema assertions exit non-zero before writing episodes, so they never appear here.
          </p>
        </div>
      </>
    );
  }

  // Annotate each parser with its freshness category and sort stale → fresh,
  // then by recency within each tier (newest first).
  const annotated = data.parsers.map((row) => ({
    row,
    fresh: freshness(row.last_ingested_at) ?? 'stale',
    age: row.last_ingested_at ? Date.parse(row.last_ingested_at) : 0,
  }));
  annotated.sort((a, b) => {
    const r = FRESHNESS_RANK[a.fresh] - FRESHNESS_RANK[b.fresh];
    return r !== 0 ? r : b.age - a.age;
  });

  // KPI aggregates.
  const total24h = annotated.reduce((sum, p) => {
    if (p.fresh === 'fresh') return sum + p.row.episode_count;
    return sum;
  }, 0);
  const staleCount = annotated.filter((p) => p.fresh === 'stale').length;
  const medianFreshSec = medianAgeSeconds(annotated.filter((p) => p.fresh !== 'stale'));

  return (
    <>
      <PageHeader />

      <div className="kpi-grid">
        <div className="kpi accent">
          <div className="kpi-eyebrow">Active</div>
          <div className="kpi-value numeric">{annotated.length}</div>
          <div className="kpi-sub">parser pairs</div>
        </div>
        <div className="kpi">
          <div className="kpi-eyebrow">Episodes (24h)</div>
          <div className="kpi-value numeric">{total24h.toLocaleString()}</div>
        </div>
        <div
          className="kpi"
          style={staleCount > 0 ? { borderTop: '2px solid var(--madder-600)' } : undefined}
        >
          <div
            className="kpi-eyebrow"
            style={staleCount > 0 ? { color: 'var(--madder-700)' } : undefined}
          >
            Stale (&gt;7d)
          </div>
          <div
            className="kpi-value numeric"
            style={staleCount > 0 ? { color: 'var(--madder-700)' } : undefined}
          >
            {staleCount}
          </div>
        </div>
        <div className="kpi">
          <div className="kpi-eyebrow">Median freshness</div>
          <div className="kpi-value numeric">
            {medianFreshSec === null ? '—' : formatAge(medianFreshSec)}
          </div>
        </div>
      </div>

      <div className="section-head">
        <h3>
          Parsers <span className="count-pill">{annotated.length}</span>
        </h3>
      </div>
      <table className="tbl">
        <thead>
          <tr>
            <th>Parser</th>
            <th>Source schema</th>
            <th className="cell-num">Episodes</th>
            <th>Last ingested</th>
            <th>Health</th>
          </tr>
        </thead>
        <tbody>
          {annotated.map(({ row, fresh }) => (
            <ParserRow
              key={`${row.parser_version}|${row.parser_source_schema}`}
              row={row}
              fresh={fresh}
            />
          ))}
        </tbody>
      </table>
    </>
  );
};

function ParserRow({ row, fresh }: { row: ParserHealthRow; fresh: Freshness }) {
  return (
    <tr>
      <td className="cell-id">{row.parser_version}</td>
      <td className="cell-muted">{row.parser_source_schema}</td>
      <td className="cell-num">{row.episode_count.toLocaleString()}</td>
      <td
        className="cell-muted"
        title={row.last_ingested_at ?? undefined}
        style={{ whiteSpace: 'nowrap' }}
      >
        {relativeTime(row.last_ingested_at)}
      </td>
      <td>
        <span className={`pill ${FRESHNESS_PILL[fresh]}`}>
          <span className="dot" />
          {FRESHNESS_LABEL[fresh]}
        </span>
      </td>
    </tr>
  );
}

function PageHeader() {
  return (
    <div className="page-header">
      <div className="page-header-titles">
        <div className="page-eyebrow">Ingestion / Parser health</div>
        <h2>Parser health</h2>
        <p>
          When did each bootstrap parser last write an episode? Parsers that fail schema assertions
          exit before writing, so their absence from this list is the failure signal.
        </p>
      </div>
    </div>
  );
}

function medianAgeSeconds(rows: { age: number }[]): number | null {
  if (rows.length === 0) return null;
  const ages = rows.map((r) => Math.max(0, (Date.now() - r.age) / 1000)).sort((a, b) => a - b);
  const mid = Math.floor(ages.length / 2);
  return ages.length % 2 === 0 ? (ages[mid - 1] + ages[mid]) / 2 : ages[mid];
}

function formatAge(seconds: number): React.ReactNode {
  if (seconds < 60)
    return (
      <>
        {Math.round(seconds)}
        <span className="kpi-unit">s</span>
      </>
    );
  if (seconds < 3600)
    return (
      <>
        {Math.round(seconds / 60)}
        <span className="kpi-unit">m</span>
      </>
    );
  if (seconds < 86400)
    return (
      <>
        {Math.round(seconds / 3600)}
        <span className="kpi-unit">h</span>
      </>
    );
  return (
    <>
      {Math.round(seconds / 86400)}
      <span className="kpi-unit">d</span>
    </>
  );
}
