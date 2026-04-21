/**
 * Entity detail page.
 *
 * Shows full entity properties, aliases, source episodes,
 * related facts with temporal ranges, and a graph neighborhood
 * visualization using the loom_traverse data.
 */
import type React from 'react';
import { Link, useParams } from 'react-router-dom';
import { getEntityDetail, getEntityGraph } from '../api/client';
import { useApi } from '../hooks/useApi';
import type { GraphEdge, GraphNode } from '../types';

// ---------------------------------------------------------------------------
// Graph neighborhood sub-component
// ---------------------------------------------------------------------------

/** Props for the graph neighborhood renderer. */
interface GraphViewProps {
  nodes: GraphNode[];
  edges: GraphEdge[];
  rootId: string;
}

/** Color map for entity types. */
const TYPE_COLORS: Record<string, string> = {
  person: '#3498db',
  organization: '#2ecc71',
  project: '#e74c3c',
  service: '#9b59b6',
  technology: '#f39c12',
  pattern: '#1abc9c',
  environment: '#e67e22',
  document: '#95a5a6',
  metric: '#34495e',
  decision: '#d35400',
};

/**
 * Renders a simple SVG-based graph neighborhood.
 *
 * Nodes are positioned in concentric rings by hop depth.
 * Edges are drawn as lines between connected nodes.
 */
const GraphView: React.FC<GraphViewProps> = ({ nodes, edges, rootId }) => {
  if (nodes.length === 0) {
    return <p className="placeholder">No graph neighbors found.</p>;
  }

  const width = 600;
  const height = 400;
  const cx = width / 2;
  const cy = height / 2;

  // Group nodes by hop depth for ring layout
  const byDepth: Record<number, GraphNode[]> = {};
  for (const node of nodes) {
    const d = node.hop_depth;
    if (!byDepth[d]) byDepth[d] = [];
    byDepth[d].push(node);
  }

  // Compute positions: root at center, each depth in a ring
  const positions = new Map<string, { x: number; y: number }>();
  for (const [depthStr, group] of Object.entries(byDepth)) {
    const depth = Number(depthStr);
    if (depth === 0) {
      for (const n of group) {
        positions.set(n.entity_id, { x: cx, y: cy });
      }
    } else {
      const radius = depth * 140;
      for (let i = 0; i < group.length; i++) {
        const angle = (2 * Math.PI * i) / group.length - Math.PI / 2;
        positions.set(group[i].entity_id, {
          x: cx + radius * Math.cos(angle),
          y: cy + radius * Math.sin(angle),
        });
      }
    }
  }

  // Build a simple edge-to-node mapping using fact subject/object
  // Since GraphEdge only has fact_id + predicate, we draw edges between
  // sequential node pairs as a best-effort visualization

  return (
    <svg
      viewBox={`0 0 ${width} ${height}`}
      style={{ width: '100%', maxWidth: '600px', background: '#fafbfc', borderRadius: '8px' }}
      role="img"
      aria-label="Entity graph neighborhood visualization"
    >
      {/* Draw edges as lines between root and hop-1, hop-1 and hop-2 */}
      {nodes
        .filter((n) => n.hop_depth > 0)
        .map((n) => {
          const pos = positions.get(n.entity_id);
          // Connect to root for hop 1, or to nearest hop-1 node for hop 2
          const parentDepth = n.hop_depth - 1;
          const parents = byDepth[parentDepth] ?? [];
          const parent = parents[0]; // simplified: connect to first parent
          const parentPos = parent ? positions.get(parent.entity_id) : undefined;
          if (!(pos && parentPos)) return null;
          return (
            <line
              key={`edge-${n.entity_id}`}
              x1={parentPos.x}
              y1={parentPos.y}
              x2={pos.x}
              y2={pos.y}
              stroke="#ccc"
              strokeWidth={1.5}
            />
          );
        })}

      {/* Draw edge labels */}
      {edges.slice(0, 20).map((e, i) => {
        // Position labels along edges
        const sourceNode = nodes.find((n) => n.hop_depth === 0) ?? nodes[0];
        const targetIdx = Math.min(i + 1, nodes.length - 1);
        const targetNode = nodes[targetIdx];
        if (!(sourceNode && targetNode)) return null;
        const sp = positions.get(sourceNode.entity_id);
        const tp = positions.get(targetNode.entity_id);
        if (!(sp && tp)) return null;
        const mx = (sp.x + tp.x) / 2;
        const my = (sp.y + tp.y) / 2;
        return (
          <text
            key={`label-${e.fact_id}`}
            x={mx}
            y={my - 4}
            textAnchor="middle"
            fontSize={8}
            fill="#888"
          >
            {e.predicate}
          </text>
        );
      })}

      {/* Draw nodes */}
      {nodes.map((n) => {
        const pos = positions.get(n.entity_id);
        if (!pos) return null;
        const isRoot = n.entity_id === rootId;
        const color = TYPE_COLORS[n.entity_type] ?? '#888';
        return (
          <g key={n.entity_id}>
            <circle
              cx={pos.x}
              cy={pos.y}
              r={isRoot ? 20 : 14}
              fill={color}
              opacity={0.85}
              stroke={isRoot ? '#1a1a2e' : 'none'}
              strokeWidth={isRoot ? 2 : 0}
            />
            <text
              x={pos.x}
              y={pos.y + (isRoot ? 30 : 24)}
              textAnchor="middle"
              fontSize={isRoot ? 11 : 9}
              fontWeight={isRoot ? 700 : 400}
              fill="#333"
            >
              {n.entity_name.length > 18 ? `${n.entity_name.slice(0, 16)}…` : n.entity_name}
            </text>
            <text
              x={pos.x}
              y={pos.y + 4}
              textAnchor="middle"
              fontSize={8}
              fill="#fff"
              fontWeight={600}
            >
              {n.entity_type.slice(0, 3).toUpperCase()}
            </text>
          </g>
        );
      })}
    </svg>
  );
};

// ---------------------------------------------------------------------------
// Main page component
// ---------------------------------------------------------------------------

/** Detail view for a single entity with graph neighborhood. */
export const EntityDetailPage: React.FC = () => {
  const { id } = useParams<{ id: string }>();
  // biome-ignore lint/style/noNonNullAssertion: id guaranteed by route param
  const entityId = id!;
  const { data, loading, error } = useApi(() => getEntityDetail(entityId), [entityId]);
  const {
    data: graph,
    loading: graphLoading,
    error: graphError,
  } = useApi(() => getEntityGraph(entityId), [entityId]);

  return (
    <div>
      <div className="page-header">
        <h2>Entity Detail</h2>
        <p>
          <Link to="/entities" style={{ color: '#3a3a6a' }}>
            ← Back to entities
          </Link>
        </p>
      </div>

      {loading && <p className="loading">Loading…</p>}
      {error && <p className="error">{error}</p>}

      {data && (
        <>
          <div className="card">
            <dl
              style={{
                display: 'grid',
                gridTemplateColumns: 'max-content 1fr',
                gap: '0.5rem 1rem',
                fontSize: '0.85rem',
              }}
            >
              <dt style={{ fontWeight: 600 }}>Name</dt>
              <dd>{data.name}</dd>
              <dt style={{ fontWeight: 600 }}>Type</dt>
              <dd>{data.entity_type}</dd>
              <dt style={{ fontWeight: 600 }}>Namespace</dt>
              <dd>{data.namespace}</dd>
              <dt style={{ fontWeight: 600 }}>Tier</dt>
              <dd>{data.tier ?? '—'}</dd>
              <dt style={{ fontWeight: 600 }}>Salience</dt>
              <dd>{data.salience_score != null ? data.salience_score.toFixed(3) : '—'}</dd>
              <dt style={{ fontWeight: 600 }}>Aliases</dt>
              <dd>{data.aliases.length > 0 ? data.aliases.join(', ') : '—'}</dd>
              <dt style={{ fontWeight: 600 }}>Source Episodes</dt>
              <dd>
                {data.source_episodes && data.source_episodes.length > 0
                  ? `${data.source_episodes.length} episode(s)`
                  : '—'}
              </dd>
              <dt style={{ fontWeight: 600 }}>Created</dt>
              <dd>{new Date(data.created_at).toLocaleString()}</dd>
            </dl>
          </div>

          {/* Graph Neighborhood */}
          <div className="card">
            <h3 style={{ fontSize: '0.95rem', marginBottom: '0.75rem' }}>
              Graph Neighborhood (1-2 hops)
            </h3>
            {graphLoading && <p className="loading">Loading graph…</p>}
            {graphError && <p className="error">{graphError}</p>}
            {graph && (
              <GraphView nodes={graph.nodes} edges={graph.edges} rootId={graph.root_entity_id} />
            )}
          </div>

          {/* Facts with temporal details */}
          {data.facts.length > 0 && (
            <div className="card">
              <h3 style={{ fontSize: '0.95rem', marginBottom: '0.5rem' }}>
                Facts ({data.facts.length})
              </h3>
              <div style={{ overflowX: 'auto' }}>
                <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: '0.85rem' }}>
                  <thead>
                    <tr style={{ borderBottom: '2px solid #eee', textAlign: 'left' }}>
                      <th style={{ padding: '0.5rem' }}>Subject</th>
                      <th style={{ padding: '0.5rem' }}>Predicate</th>
                      <th style={{ padding: '0.5rem' }}>Object</th>
                      <th style={{ padding: '0.5rem' }}>Status</th>
                      <th style={{ padding: '0.5rem' }}>Valid From</th>
                      <th style={{ padding: '0.5rem' }}>Valid Until</th>
                      <th style={{ padding: '0.5rem' }}>Tier</th>
                    </tr>
                  </thead>
                  <tbody>
                    {data.facts.map((f) => (
                      <tr
                        key={f.id}
                        style={{
                          borderBottom: '1px solid #f0f0f0',
                          opacity: f.valid_until ? 0.6 : 1,
                        }}
                      >
                        <td style={{ padding: '0.5rem' }}>{f.subject_name}</td>
                        <td
                          style={{
                            padding: '0.5rem',
                            fontFamily: 'monospace',
                            fontSize: '0.8rem',
                          }}
                        >
                          {f.predicate}
                        </td>
                        <td style={{ padding: '0.5rem' }}>{f.object_name}</td>
                        <td style={{ padding: '0.5rem' }}>
                          <span
                            style={{
                              color: f.evidence_status === 'superseded' ? '#e74c3c' : '#333',
                              fontWeight: f.evidence_status === 'superseded' ? 600 : 400,
                            }}
                          >
                            {f.evidence_status}
                          </span>
                        </td>
                        <td style={{ padding: '0.5rem' }}>
                          {new Date(f.valid_from).toLocaleDateString()}
                        </td>
                        <td style={{ padding: '0.5rem' }}>
                          {f.valid_until ? new Date(f.valid_until).toLocaleDateString() : '—'}
                        </td>
                        <td style={{ padding: '0.5rem' }}>{f.tier ?? '—'}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </div>
          )}
        </>
      )}
    </div>
  );
};
