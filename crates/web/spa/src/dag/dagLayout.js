// dagLayout.js — dagre auto-layout for the DAG run graph (pure, no React).
// React Flow wants top-left positions; dagre returns node CENTERS, so every
// position is shifted back by half the fixed node box. The box size is a
// constant shared with the CSS (.dag-node) so labels never clip.

import dagre from '@dagrejs/dagre';

export const NODE_W = 176;
export const NODE_H = 52;

/// layoutGraph(nodes, edges, opts) → nodes with `position` assigned.
/// Edges referencing unknown ids are ignored (graphFromSpec already guards
/// cycles upstream, and dagre misbehaves on cyclic input).
export function layoutGraph(nodes, edges, opts = {}) {
  const list = Array.isArray(nodes) ? nodes : [];
  if (!list.length) {
    return list;
  }
  const { direction = 'LR', nodesep = 40, ranksep = 96 } = opts;
  const g = new dagre.graphlib.Graph();
  g.setGraph({ rankdir: direction, nodesep, ranksep, marginx: 16, marginy: 16 });
  g.setDefaultEdgeLabel(() => ({}));
  for (const n of list) {
    g.setNode(n.id, { width: NODE_W, height: NODE_H });
  }
  for (const e of Array.isArray(edges) ? edges : []) {
    if (g.hasNode(e.source) && g.hasNode(e.target) && e.source !== e.target) {
      g.setEdge(e.source, e.target);
    }
  }
  dagre.layout(g);
  return list.map((n) => {
    const p = g.node(n.id) || {};
    return {
      ...n,
      position: {
        x: Math.round((Number.isFinite(p.x) ? p.x : 0) - NODE_W / 2),
        y: Math.round((Number.isFinite(p.y) ? p.y : 0) - NODE_H / 2),
      },
    };
  });
}
