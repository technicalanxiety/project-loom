/**
 * Entity list page.
 *
 * The most-visited "browse" page after Pipeline Health. Surfaces:
 *   - KPI strip (total / hot tier / namespaces)
 *   - Filter bar (namespace + search) using shared form controls
 *   - .tbl with tier pills, salience inline-bar, and aliases under
 *     the entity name
 */
import type React from 'react';
import { useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import { getEntities } from '../api/client';
import { useApi } from '../hooks/useApi';
import { useNamespaces } from '../hooks/useNamespaces';
import type { EntitySummary } from '../types';

const TIER_PILL: Record<string, string> = {
  hot: 'pill-tier-hot',
  warm: 'pill-tier-warm',
  cold: 'pill-tier-cold',
};

export const EntitiesPage: React.FC = () => {
  const [namespace, setNamespace] = useState<string>('');
  const [search, setSearch] = useState('');
  const { data: namespaces } = useNamespaces();

  const { data, loading, error } = useApi(
    () =>
      getEntities({
        namespace: namespace || undefined,
        q: search || undefined,
        limit: 50,
      }),
    [namespace, search],
  );

  const stats = useMemo(() => {
    if (!data) return null;
    const types = new Set(data.map((e) => e.entity_type)).size;
    const namespacesInResults = new Set(data.map((e) => e.namespace)).size;
    const hot = data.filter((e) => e.tier === 'hot').length;
    return { total: data.length, types, namespaces: namespacesInResults, hot };
  }, [data]);

  return (
    <>
      <div className="page-header">
        <div className="page-header-titles">
          <div className="page-eyebrow">Knowledge / Entities</div>
          <h2>Entities</h2>
          <p>Browse and search knowledge-graph entities.</p>
        </div>
      </div>

      {data && stats && (
        <div className="kpi-grid">
          <div className="kpi accent">
            <div className="kpi-eyebrow">Showing</div>
            <div className="kpi-value numeric">{stats.total}</div>
            <div className="kpi-sub">limit 50</div>
          </div>
          <div className="kpi">
            <div className="kpi-eyebrow">Hot tier</div>
            <div className="kpi-value numeric" style={{ color: 'var(--madder-700)' }}>
              {stats.hot}
            </div>
          </div>
          <div className="kpi">
            <div className="kpi-eyebrow">Types</div>
            <div className="kpi-value numeric">{stats.types}</div>
          </div>
          <div className="kpi">
            <div className="kpi-eyebrow">Namespaces</div>
            <div className="kpi-value numeric">{stats.namespaces}</div>
          </div>
        </div>
      )}

      <div className="filter-bar">
        <select
          className="form-control"
          value={namespace}
          onChange={(e) => setNamespace(e.target.value)}
        >
          <option value="">All namespaces</option>
          {namespaces?.map((ns) => (
            <option key={ns.namespace} value={ns.namespace}>
              {ns.namespace}
            </option>
          ))}
        </select>
        <input
          className="form-control"
          type="text"
          placeholder="Search entities…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          style={{ minWidth: 220 }}
        />
        {(namespace || search) && (
          <button
            type="button"
            className="btn btn-ghost"
            onClick={() => {
              setNamespace('');
              setSearch('');
            }}
          >
            Reset
          </button>
        )}
        {data && <span className="filter-result-count">{data.length} results</span>}
      </div>

      {loading && <div className="loading">Loading entities…</div>}
      {error && <div className="error">{error}</div>}

      {data &&
        (data.length === 0 ? (
          <div className="empty-state">
            <h3>No entities match</h3>
            <p>
              Try widening the search, switching namespace, or seeding the namespace from the
              ingestion pipeline.
            </p>
          </div>
        ) : (
          <table className="tbl">
            <thead>
              <tr>
                <th>Name</th>
                <th>Type</th>
                <th>Namespace</th>
                <th>Tier</th>
                <th>Salience</th>
              </tr>
            </thead>
            <tbody>
              {data.map((entity) => (
                <EntityRow key={entity.id} entity={entity} />
              ))}
            </tbody>
          </table>
        ))}
    </>
  );
};

function EntityRow({ entity }: { entity: EntitySummary }) {
  const sal = entity.salience_score;
  return (
    <tr>
      <td>
        <Link to={`/entities/${entity.id}`} className="cell-id">
          {entity.name}
        </Link>
        {entity.aliases.length > 0 && (
          <div
            className="cell-muted"
            style={{
              fontFamily: 'var(--font-mono)',
              fontSize: 11,
              marginTop: 2,
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
              maxWidth: 320,
            }}
          >
            aliases: {entity.aliases.join(', ')}
          </div>
        )}
      </td>
      <td className="cell-muted">{entity.entity_type}</td>
      <td>
        <span className="pill pill-vendor">
          <span className="dot" />
          {entity.namespace}
        </span>
      </td>
      <td>
        {entity.tier ? (
          <span className={`pill ${TIER_PILL[entity.tier] ?? 'pill-neutral'}`}>{entity.tier}</span>
        ) : (
          <span className="cell-muted">—</span>
        )}
      </td>
      <td>
        {sal !== null ? (
          <span
            style={{ display: 'inline-flex', alignItems: 'center', gap: 6, whiteSpace: 'nowrap' }}
          >
            <span className="inline-bar tone-ok" style={{ width: 60 }} aria-hidden="true">
              <span style={{ width: `${sal * 100}%` }} />
            </span>
            <span className="cell-num" style={{ minWidth: 32 }}>
              {sal.toFixed(2)}
            </span>
          </span>
        ) : (
          <span className="cell-muted">—</span>
        )}
      </td>
    </tr>
  );
}
