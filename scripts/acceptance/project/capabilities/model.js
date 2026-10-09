// Only the model transport is replaced. Decisions use the actual node context.
const assert = require('node:assert/strict');

function markerFrom(text) {
  const markers = text.match(/pcap-[a-zA-Z0-9-]+-(?:agent|operator|dag|brain)/g) || [];
  assert(markers.length, 'model did not receive the TODO marker');
  return markers.at(-1);
}

function answer(prompt) {
  let context;
  try { context = JSON.parse(prompt); } catch {}
  if (!context?.plan?.layers || !context.run) {
    const output = { marker: markerFrom(prompt), sum: 2 + 3 + 5, passed: true };
    return prompt.includes('上游步骤输出（JSON）') ? '\x60\x60\x60json\n' + JSON.stringify(output) + '\n\x60\x60\x60' : output;
  }
  const layer = context.run.layer || 0;
  const current = context.operations.filter((op) => op.activation === context.run.activation);
  assert(current.every((op) => op.status === 'done'), 'model invoked before the layer barrier');
  const assessments = layer ? { [context.plan.layers[layer - 1].layer_id]:
    { met: true, reason: 'All compute results contain the expected marker and sum.' } } : {};
  const marker = markerFrom(context.root_inputs.todo);
  if (layer === context.plan.layers.length) return {
    decision: 'complete', reason: 'Both layers passed', assessments,
    evidence_execution_ids: context.operations.map((op) => op.execution_id),
    summary: JSON.stringify({ marker, sum: 10, passed: true }),
  };
  const assignments = context.plan.nodes.filter((node) =>
    node.layer_id === context.plan.layers[layer].layer_id).map((node) => {
    const inputs = { todo: { kind: 'root', name: 'todo' } };
    if (layer) {
      for (const kind of ['agent', 'operator']) {
        const upstream = current.find((op) => op.execution_kind === kind);
        assert(upstream, 'upstream execution missing: ' + kind);
        inputs[kind + '_result'] = { kind: 'execution', execution_id: upstream.execution_id, path: '' };
      }
    }
    return { node_id: node.node_id, capability_id: node.capability_id, inputs, reason: 'Execute the assigned bounded task' };
  });
  return { decision: 'dispatch_layer', layer: layer + 1, assignments, assessments,
    reflection: null, reason: 'All nodes of the next layer', evidence_execution_ids: [] };
}
module.exports = { answer };
