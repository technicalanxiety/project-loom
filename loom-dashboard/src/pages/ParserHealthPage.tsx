/**
 * Parser health page.
 *
 * Lists every bootstrap parser (parser_version + parser_source_schema) that
 * has produced episodes, with episode counts and the last-ingested-at
 * timestamp. Surfaces per-parser activity so the operator can tell at a
 * glance which vendor parsers have run recently.
 *
 * Schema-assertion failures are not displayed here — bootstrap parsers
 * fail loud at the CLI boundary and never write episodes on failure, so
 * absence of a recent `last_ingested_at` is the primary signal. That is
 * intentional per the degraded-mode contract in bootstrap/README.md.
 */
import type React from 'react';
import { getParserHealthMetrics } from '../api/client';
import { useApi } from '../hooks/useApi';

/** Parser health page component. */
export const ParserHealthPage: React.FC = () => {
  const { data, loading, error } = useApi(getParserHealthMetrics);

  if (loading) {
    return <p>Loading parser health…</p>;
  }
  if (error) {
    return <p style={{ color: '#c44' }}>Error: {error}</p>;
  }
  if (!data || data.parsers.length === 0) {
    return (
      <div>
        <h2>Parser Health</h2>
        <p style={{ color: '#888' }}>
          No vendor-import episodes yet. Run a parser under <code>bootstrap/</code> to
          populate this view.
        </p>
      </div>
    );
  }

  return (
    <div>
      <h2>Parser Health</h2>
      <p style={{ color: '#888', fontSize: '0.85rem', maxWidth: 640 }}>
        One row per <code>(parser_version, parser_source_schema)</code> pair
        across all <code>vendor_import</code> episodes. Parsers that fail
        schema assertions do not appear here — they exit non-zero before
        writing.
      </p>
      <table
        style={{
          width: '100%',
          maxWidth: 900,
          borderCollapse: 'collapse',
          marginTop: '1rem',
          fontSize: '0.85rem',
        }}
      >
        <thead>
          <tr style={{ textAlign: 'left', borderBottom: '1px solid #333' }}>
            <th style={{ padding: '0.5rem' }}>Parser</th>
            <th style={{ padding: '0.5rem' }}>Source schema</th>
            <th style={{ padding: '0.5rem', textAlign: 'right' }}>Episodes</th>
            <th style={{ padding: '0.5rem' }}>Last ingested</th>
          </tr>
        </thead>
        <tbody>
          {data.parsers.map((row) => (
            <tr
              key={`${row.parser_version}|${row.parser_source_schema}`}
              style={{ borderBottom: '1px solid #222' }}
            >
              <td style={{ padding: '0.5rem' }}>
                <code>{row.parser_version}</code>
              </td>
              <td style={{ padding: '0.5rem' }}>
                <code>{row.parser_source_schema}</code>
              </td>
              <td style={{ padding: '0.5rem', textAlign: 'right', fontWeight: 600 }}>
                {row.episode_count.toLocaleString()}
              </td>
              <td style={{ padding: '0.5rem', color: '#aaa' }}>
                {row.last_ingested_at ?? '—'}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
};
