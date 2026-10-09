// Execution selection uses saved operations, including earlier layers and rounds.
export const newestFirst = (operations) => [...operations].sort((a, b) =>
  (b.activation || 0) - (a.activation || 0) || (b.attempt || 0) - (a.attempt || 0));

export function layerExecutions(plan, operations, layerId) {
  const ids = new Set(plan.nodes.filter((node) => node.layer_id === layerId).map((node) => node.node_id));
  return newestFirst(operations.filter((operation) => ids.has(operation.node_id)));
}

export function latestExecutions(operations) {
  const latest = new Map();
  for (const operation of newestFirst(operations)) {
    if (!latest.has(operation.node_id)) latest.set(operation.node_id, operation);
  }
  return [...latest.values()];
}

export const canOpenExecution = (operation) => !!operation?.execution_id && operation.status !== 'creating';

export function selectExecution(plan, operations, selection) {
  const layerId = selection.type === 'layer' ? selection.id
    : plan.nodes.find((node) => node.node_id === selection.id)?.layer_id;
  const rows = layerExecutions(plan, operations, layerId);
  const latest = latestExecutions(rows);
  const operation = selection.type === 'node' ? latest.find((row) => row.node_id === selection.id)
    : latest.length === 1 ? latest[0] : null;
  return { layerId, executionId: canOpenExecution(operation) ? operation.execution_id : null };
}
