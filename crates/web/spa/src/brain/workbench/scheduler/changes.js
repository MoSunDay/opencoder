import { addLayer, bindCapability, executionNode, moveNode } from '../milestone/model.js';

// Every edit is applied to the latest draft; callbacks never replace an old plan.
export function changeDraft(draft, action) {
  const plan = draft.version.plan;
  const withPlan = (next) => ({ ...draft, version: { ...draft.version, plan: next } });
  switch (action.type) {
    case 'add_layer':
      if (plan.layers.length >= 32) throw new Error('最多 32 层');
      return withPlan(addLayer(plan, action.id));
    case 'add_node': {
      if (!plan.layers.some((layer) => layer.layer_id === action.layerId)) throw new Error('目标里程碑不存在');
      if (plan.nodes.length >= 256 || plan.nodes.filter((node) => node.layer_id === action.layerId).length >= 32) throw new Error('最多 256 个执行节点，每层最多 32 个');
      return withPlan({ ...plan, nodes: [...plan.nodes, executionNode(action.id, action.layerId)] });
    }
    case 'layer':
      return withPlan({ ...plan, layers: plan.layers.map((layer) => layer.layer_id === action.id ? { ...layer, ...action.patch } : layer) });
    case 'capability':
      return withPlan({ ...plan, nodes: plan.nodes.map((node) => node.node_id === action.id ? bindCapability(node, action.capability) : node) });
    case 'move':
      return withPlan(moveNode(plan, action.id, action.layerId));
    case 'remove': {
      const layers = new Set(action.items.filter((item) => item.type === 'layer').map((item) => item.id));
      const nodes = new Set(action.items.filter((item) => item.type === 'node').map((item) => item.id));
      const next = withPlan({ ...plan, layers: plan.layers.filter((layer) => !layers.has(layer.layer_id)),
        nodes: plan.nodes.filter((node) => !nodes.has(node.node_id) && !layers.has(node.layer_id)) });
      return { ...next, layout: Object.fromEntries(Object.entries(draft.layout || {}).filter(([id]) => !layers.has(id))) };
    }
    case 'positions': {
      const ids = new Set(plan.layers.map((layer) => layer.layer_id));
      const patch = Object.fromEntries(Object.entries(action.positions).filter(([id, p]) => ids.has(id) && Number.isFinite(p?.x) && Number.isFinite(p?.y)));
      return { ...draft, layout: { ...draft.layout, ...patch } };
    }
    case 'reset_layout': return { ...draft, layout: {} };
    case 'metadata': {
      const { engineering = [], title, objective, max_rounds } = action.values;
      return { ...withPlan({ ...plan, title, objective, max_rounds }), engineering };
    }
    default: throw new Error(`未知的画布操作：${action.type}`);
  }
}
