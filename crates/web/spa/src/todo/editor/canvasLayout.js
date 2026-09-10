// canvasLayout.js — dagre auto-layout for the TODO editor canvas (pure, no
// React). Mirrors dag/editor/canvasLayout.js with TODO-sized boxes and the
// same position priority chain: an explicit opts.positions override wins,
// then a placement the user dragged themselves (node.data.placed), and only
// then the computed dagre center shifted back by half the box (React Flow
// wants top-left corners). The box size is shared with the editor CSS.

import dagre from '@dagrejs/dagre';

export const TODO_NODE_W = 240;
export const TODO_NODE_H = 96;

/// layoutTodoNodes(nodes, edges, opts) → NEW node array with `position`
/// assigned; the input arrays/nodes are never mutated. opts.positions is a
/// plain object keyed by node id. Edges referencing unknown ids or self
/// loops are ignored (dagre misbehaves otherwise).
export function layoutTodoNodes(nodes, edges, opts = {}) {
  const list = Array.isArray(nodes) ? nodes : [];
  if (!list.length) {
    return list;
  }
  const positions = opts.positions && typeof opts.positions === 'object' ? opts.positions : null;
  const g = new dagre.graphlib.Graph();
  g.setGraph({ rankdir: 'LR', nodesep: 36, ranksep: 90, marginx: 24, marginy: 24 });
  g.setDefaultEdgeLabel(() => ({}));
  for (const n of list) {
    g.setNode(n.id, { width: TODO_NODE_W, height: TODO_NODE_H });
  }
  for (const e of Array.isArray(edges) ? edges : []) {
    if (g.hasNode(e.source) && g.hasNode(e.target) && e.source !== e.target) {
      g.setEdge(e.source, e.target);
    }
  }
  dagre.layout(g);
  return list.map((n) => {
    const pin = positions ? positions[n.id] : null;
    if (pin && Number.isFinite(pin.x) && Number.isFinite(pin.y)) {
      return { ...n, position: { x: Math.round(pin.x), y: Math.round(pin.y) } };
    }
    if (n.data && n.data.placed === true) {
      return { ...n };
    }
    const p = g.node(n.id) || {};
    return {
      ...n,
      position: {
        x: Math.round((Number.isFinite(p.x) ? p.x : 0) - TODO_NODE_W / 2),
        y: Math.round((Number.isFinite(p.y) ? p.y : 0) - TODO_NODE_H / 2),
      },
    };
  });
}
