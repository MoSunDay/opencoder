// model.js — pure helpers for the schema_version 4 layered capability canvas.
//
// Every row is derived from the two LOCKED control payloads
// (GET /api/brain/runs/:id/layered, GET /api/brain/runs/:id/layered/rounds/:n)
// and degrades to empty/pending rows when `operations` or `events` are missing
// or a node has no attempt yet. Layer numbers are 1-based on the wire: run.layer
// counts the layers already dispatched and the layer being decided is always
// run.layer + 1, so layers[i] is layer i + 1.

export const LAYERED_SCHEMA_VERSION = 4;
export const PENDING_STATUS = 'pending';
/// LayeredRetry::default (retry.max_attempts is 1..=5).
export const DEFAULT_MAX_ATTEMPTS = 2;
export const LAYERED_TERMINAL = ['completed', 'failed', 'cancelled'];

export const LAYERED_PHASES = {
  ready: '等待决策', deciding: '判断中', waiting: '等待执行', paused: '已暂停',
  blocked: '已阻塞', completed: '已完成', failed: '失败', cancelled: '已取消',
};

export const LAYERED_STATUS = {
  pending: '待派发', creating: '创建中', running: '执行中', done: '执行结束',
  error: '失败', cancelled: '已取消',
};

export const LAYERED_COLORS = {
  ready: 'cyan', deciding: 'purple', waiting: 'blue', paused: 'gold', blocked: 'red',
  completed: 'green', failed: 'red', cancelled: 'default', pending: 'default',
  creating: 'gold', running: 'blue', done: 'green', error: 'red',
};

export const LAYERED_EVENTS = {
  run_created: '运行创建', layer_started: '层开始', node_dispatched: '节点派发',
  operation_admitted: '执行准入', operation_terminal: '执行结束',
  operation_retry_scheduled: '重试排期', layer_barrier_reached: '层屏障达成',
  run_completed: '运行完成', run_failed: '运行失败', run_paused: '已暂停',
  run_resumed: '已恢复', run_cancelled: '已取消', decision_blocked: '决策阻塞',
};

export const LAYERED_DECISIONS = {
  dispatch_layer: '派发本层', complete: '完成判断', done: '执行结束', error: '执行失败', cancelled: '已取消',
};

const asArray = (value) => (Array.isArray(value) ? value : []);
const asText = (value) => (value === null || value === undefined ? '' : String(value));
const asCount = (value) => {
  const number = Math.trunc(Number(value));
  return Number.isFinite(number) && number > 0 ? number : 0;
};

/// schemaVersionOf(body) — the first usable schema_version of a control body.
/// Accepts the run snapshot (top level), an embedded run or a plan, so a v4
/// snapshot is detectable no matter which of the three carries the field.
export function schemaVersionOf(body) {
  for (const value of [body?.schema_version, body?.run?.schema_version, body?.plan?.schema_version]) {
    if (typeof value === 'number' && Number.isInteger(value)) return Number(value);
  }
  return null;
}

export const isLayeredSnapshot = (body) => schemaVersionOf(body) === LAYERED_SCHEMA_VERSION;
export const isLayeredView = (body) => isLayeredSnapshot(body) && !!body?.run;
export const layeredPhase = (view) => asText(view?.run?.phase) || 'ready';
export const terminalPhase = (phase) => LAYERED_TERMINAL.includes(phase);

export function planOf(view) {
  const plan = view?.plan || {};
  return {
    title: asText(plan.title), objective: asText(plan.objective),
    todoId: asText(plan.todo?.id) || null, maxRounds: asCount(plan.max_rounds),
    nodes: planNodes(view), edges: planEdges(view),
  };
}

export function planNodes(view) {
  return asArray(view?.plan?.nodes).filter((node) => node && asText(node.node_id));
}

export function planEdges(view) {
  return asArray(view?.plan?.edges).filter((edge) => edge && asText(edge.from) && asText(edge.to));
}

export function nodeOf(view, nodeId) {
  return planNodes(view).find((node) => asText(node.node_id) === nodeId) || null;
}

export function nodeTitle(view, nodeId) {
  return asText(nodeOf(view, nodeId)?.title) || asText(nodeId);
}

export function maxAttemptsOf(node) {
  const max = Math.trunc(Number(node?.retry?.max_attempts));
  return Number.isFinite(max) && max >= 1 ? max : DEFAULT_MAX_ATTEMPTS;
}

/// layerNodeIds(view) — the plan DAG levels, recomputed by the control plane.
export function layerNodeIds(view) {
  return asArray(view?.layers).map((level) => asArray(level).map(asText).filter(Boolean));
}

export function totalLayers(view) {
  const total = asCount(view?.run?.total_layers);
  return total || layerNodeIds(view).length;
}

export function operationsOf(view) {
  return asArray(view?.operations).filter((op) => op && asText(op.node_id)).map((op) => ({
    operationId: asText(op.operation_id),
    layer: asCount(op.layer),
    nodeId: asText(op.node_id),
    attempt: Math.max(1, Math.trunc(Number(op.attempt)) || 1),
    capabilityId: asText(op.capability_id),
    executionKind: asText(op.execution_kind),
    executionId: asText(op.execution_id),
    status: asText(op.status) || 'creating',
    cancelRequested: !!op.cancel_requested,
  }));
}

/// attemptsOf — every attempt of one node, oldest first (attempt is monotonic).
export function attemptsOf(operations, nodeId) {
  return operations.filter((op) => op.nodeId === nodeId).sort((a, b) => a.attempt - b.attempt);
}

/// latestAttempt — the live attempt; a superseded one never wins (mirrors
/// opencoder_brain::layered::latest_attempt).
export function latestAttempt(operations, nodeId) {
  return attemptsOf(operations, nodeId).at(-1) || null;
}

/// retryBadge(row) — the `attempt n/max_attempts` badge of one node row.
export function retryBadge(row) {
  const attempt = Math.max(0, Math.trunc(Number(row?.attempt)) || 0);
  const max = Math.max(1, Math.trunc(Number(row?.maxAttempts)) || DEFAULT_MAX_ATTEMPTS);
  return { attempt, max, retrying: attempt > 1, label: `${attempt}/${max}` };
}

/// nodeRow — one plan node rolled up with its latest attempt.
export function nodeRow(view, nodeId, operations = operationsOf(view)) {
  const node = nodeOf(view, nodeId);
  const attempts = attemptsOf(operations, nodeId);
  const latest = attempts.at(-1) || null;
  const max = maxAttemptsOf(node);
  return {
    nodeId, title: asText(node?.title) || nodeId, capabilityId: asText(node?.capability_id),
    instructions: asText(node?.instructions), status: latest ? latest.status : PENDING_STATUS,
    attempt: latest ? latest.attempt : 0, maxAttempts: max, attempts, latest,
    operationId: latest?.operationId || '', executionId: latest?.executionId || '',
    executionKind: latest?.executionKind || '', cancelRequested: !!latest?.cancelRequested,
    started: !!latest,
  };
}

/// rollupStatus — one status for a whole layer: completed only when every node
/// has a successful latest attempt (the v4 layer barrier).
function rollupStatus(rows) {
  const statuses = rows.map((row) => row.status);
  if (statuses.length && statuses.every((status) => status === 'done')) return 'done';
  if (statuses.some((status) => status === 'running' || status === 'creating')) return 'running';
  if (statuses.some((status) => status === 'error')) return 'error';
  if (statuses.some((status) => status === 'cancelled')) return 'cancelled';
  return PENDING_STATUS;
}

/// layerRows — the plan DAG grouped into layers, each with node rollups.
export function layerRows(view) {
  const operations = operationsOf(view);
  const active = activeLayer(view);
  return layerNodeIds(view).map((nodeIds, index) => {
    const layer = index + 1;
    const nodes = nodeIds.map((nodeId) => nodeRow(view, nodeId, operations));
    return {
      layer, index, active: layer === active, nodes, status: rollupStatus(nodes),
      complete: nodes.length > 0 && nodes.every((row) => row.status === 'done'),
      attempts: nodes.reduce((sum, row) => sum + row.attempts.length, 0),
    };
  });
}

/// activeLayer — the layer being decided (run.layer + 1), null once every
/// layer has been dispatched.
export function activeLayer(view) {
  const total = totalLayers(view);
  if (!total || terminalPhase(layeredPhase(view))) return null;
  const dispatched = asCount(view?.run?.layer);
  if (dispatched > 0) {
    const ids = layerNodeIds(view)[dispatched - 1] || [];
    const ops = operationsOf(view);
    if (ids.some((id) => latestAttempt(ops, id)?.status !== 'done')) return dispatched;
  }
  return dispatched < total ? dispatched + 1 : null;
}

/// barrier — layer-barrier progress (completed layers / total layers).
export function barrier(view) {
  const rows = layerRows(view);
  const total = totalLayers(view) || rows.length;
  const completed = rows.filter((row) => row.complete).length;
  const dispatched = Math.min(asCount(view?.run?.layer), total);
  return {
    total, completed, dispatched, remaining: Math.max(0, total - completed),
    percent: total ? Math.round((completed * 100) / total) : 0, label: `${completed}/${total}`,
  };
}

export function upstreams(view, nodeId) {
  return planEdges(view).filter((edge) => asText(edge.to) === nodeId).map((edge) => asText(edge.from));
}

export function downstreams(view, nodeId) {
  return planEdges(view).filter((edge) => asText(edge.from) === nodeId).map((edge) => asText(edge.to));
}

export const LAYER_NODE_W = 216;
export const LAYER_NODE_H = 108;
export const LAYER_GAP_X = 88;
export const LAYER_GAP_Y = 26;
/// React Flow's own handle box (style.css does not resize .react-flow__handle);
/// the declared box must match the JSX <Handle> so an edge starts on the dot.
const LAYER_HANDLE = 6;

/// layerNodeBox() -> {width, handles} declaring the fixed card box. React Flow
/// gates every edge on both endpoints being "initialized" (handle bounds + a
/// width), so declaring them makes an edge render on the first frame instead
/// of after ResizeObserver measurement — the editor canvas does the same.
/// Height stays undeclared on purpose (mirrors editNodeBox): the CSS box, not
/// an inline style, keeps owning the card height. A FRESH object per call is
/// required because React Flow mutates declared handle entries in place.
function layerNodeBox() {
  const x = (LAYER_NODE_W - LAYER_HANDLE) / 2;
  return {
    width: LAYER_NODE_W,
    handles: [
      { type: 'target', position: 'top', x, y: -LAYER_HANDLE / 2, width: LAYER_HANDLE, height: LAYER_HANDLE },
      { type: 'source', position: 'bottom', x, y: LAYER_NODE_H - LAYER_HANDLE / 2, width: LAYER_HANDLE, height: LAYER_HANDLE },
    ],
  };
}

/// layerGraph — React Flow nodes/edges: one column per layer (x = layer), the
/// nodes of a column centred vertically, edges carrying their upstream binding.
export function layerGraph(view) {
  const rows = layerRows(view);
  const active = activeLayer(view);
  const heights = rows.map((row) => Math.max(0, row.nodes.length * (LAYER_NODE_W + LAYER_GAP_X) - LAYER_GAP_X));
  const height = heights.reduce((max, value) => Math.max(max, value), 0);
  const nodes = [];
  rows.forEach((row, index) => {
    const top = (height - heights[index]) / 2;
    row.nodes.forEach((data, position) => {
      nodes.push({
        id: data.nodeId,
        type: 'layeredNode',
        ...layerNodeBox(),
        position: {
          x: Math.round(top + position * (LAYER_NODE_W + LAYER_GAP_X)),
          y: index * (LAYER_NODE_H + 100),
        },
        data: {
          ...data, layer: row.layer, active: row.active, layerStatus: row.status,
          upstreamTitles: upstreams(view, data.nodeId).map((id) => nodeTitle(view, id)),
          downstreamTitles: downstreams(view, data.nodeId).map((id) => nodeTitle(view, id)),
        },
      });
    });
  });
  const known = new Set(nodes.map((node) => node.id));
  const edges = planEdges(view)
    .map((edge) => ({ from: asText(edge.from), to: asText(edge.to) }))
    .filter((edge) => known.has(edge.from) && known.has(edge.to))
    .map((edge) => ({
      id: `${edge.from}->${edge.to}`, source: edge.from, target: edge.to,
      // No edge label: the node cards already name their upstreams and a
      // React Flow label needs SVG text measurement (absent in jsdom, and the
      // binding itself is shown per node in the round detail).
      animated: layerOfEdge(view, edge.to) === active,
      data: { binding: edge, upstream: nodeTitle(view, edge.from), downstream: nodeTitle(view, edge.to) },
    }));
  return { nodes, edges };
}

function layerOfEdge(view, nodeId) {
  return layerRows(view).find((row) => row.nodes.some((node) => node.nodeId === nodeId))?.layer || null;
}

/// eventRows — the run event journal, oldest first.
export function eventRows(view) {
  return asArray(view?.events).filter((event) => event && (event.event_type || event.seq !== undefined)).map((event) => ({
    seq: Math.trunc(Number(event.seq)) || 0,
    layer: asCount(event.layer),
    eventType: asText(event.event_type) || 'layered_event',
    nodeId: asText(event.node_id),
    attempt: event.attempt === null || event.attempt === undefined ? null : Math.trunc(Number(event.attempt)),
    executionId: asText(event.execution_id),
    decisionSummary: asText(event.decision_summary),
    reasonSummary: asText(event.reason_summary),
    evidence: asArray(event.evidence_execution_ids).map(asText),
    atMs: Math.trunc(Number(event.at_ms)) || 0,
  })).sort((a, b) => a.seq - b.seq);
}

export const eventLabel = (eventType) => LAYERED_EVENTS[eventType] || eventType;
/// decisionLabel — a decision name (`dispatch_layer` / `complete`) or, on a v4
/// round detail, the phase that decision moved the run into.
export const decisionLabel = (value) => LAYERED_DECISIONS[value] || LAYERED_PHASES[value] || value || '';
export const layerLabel = (layer) => (layer > 0 ? `层 ${layer}` : '初始');

/// eventSummary — the one-line journal text: the model's own reason first,
/// then the (localized) decision name, then the execution it happened on.
export function eventSummary(row) {
  return row.reasonSummary || decisionLabel(row.decisionSummary) || row.executionId || '';
}

/// roundDetail — GET /api/brain/runs/:id/layered/rounds/:round.
export function roundDetail(payload) {
  return {
    schemaVersion: Number(payload?.schema_version) || LAYERED_SCHEMA_VERSION,
    layer: asCount(payload?.layer),
    phase: asText(payload?.phase) || 'ready',
    reason: asText(payload?.reason),
    evidence: asArray(payload?.evidence_execution_ids).map(asText),
    nodes: asArray(payload?.nodes).filter((node) => node && asText(node.node_id)).map((node) => ({
      nodeId: asText(node.node_id), title: asText(node.title) || asText(node.node_id),
      capabilityId: asText(node.capability_id), status: asText(node.status) || PENDING_STATUS,
      attempt: Math.max(1, Math.trunc(Number(node.attempt)) || 1),
      executionId: asText(node.execution_id), executionKind: asText(node.execution_kind),
      cancelRequested: !!node.cancel_requested,
    })),
  };
}
