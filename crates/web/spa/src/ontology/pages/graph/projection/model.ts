import type { Entity, GraphData, Relationship } from "../../../types";

export const INITIAL_NODES = 30;
export const INITIAL_EDGES = 60;
export type GraphProjection = { data: GraphData; nodeIds: string[]; edgeIds: string[]; remaining: Record<string, number> };
type Options = { nodeBudget: number; edgeBudget: number; retainedNodes?: string[]; retainedEdges?: string[]; reveal?: string };
const compare = (a: Entity, b: Entity) => a.name.localeCompare(b.name, "zh-CN") || a.id.localeCompare(b.id);

/** Selection only: every node and edge is an original object from the complete query result. */
export function projectGraph(data: GraphData, centers: string[], options: Options): GraphProjection {
  const nodes = new Map(data.nodes.map((node) => [node.id, node]));
  const edges = new Map(data.edges.map((edge) => [edge.id, edge]));
  const adjacency = new Map<string, { id: string; edge: Relationship }[]>();
  for (const edge of data.edges) {
    if (!nodes.has(edge.source_entity_id) || !nodes.has(edge.target_entity_id)) continue;
    for (const [from, to] of [[edge.source_entity_id, edge.target_entity_id], [edge.target_entity_id, edge.source_entity_id]]) {
      adjacency.set(from, [...(adjacency.get(from) ?? []), { id: to, edge }]);
    }
  }
  for (const group of adjacency.values()) group.sort((a, b) => compare(nodes.get(a.id)!, nodes.get(b.id)!) || a.edge.id.localeCompare(b.edge.id));
  const roots = centers.filter((id) => nodes.has(id));
  if (!roots.length && data.nodes.length) roots.push([...data.nodes].sort(compare)[0].id);
  const parent = new Map<string, { node: string; edge: string }>();
  const visited = new Set(roots);
  const ordered = [...roots];
  for (let i = 0; i < ordered.length; i++) {
    for (const neighbor of adjacency.get(ordered[i]) ?? []) {
      if (visited.has(neighbor.id)) continue;
      visited.add(neighbor.id); ordered.push(neighbor.id);
      parent.set(neighbor.id, { node: ordered[i], edge: neighbor.edge.id });
    }
  }
  ordered.push(...data.nodes.filter((node) => !visited.has(node.id)).sort(compare).map((node) => node.id));
  const selected = new Set((options.retainedNodes ?? []).filter((id) => nodes.has(id)));
  const selectedEdges = new Set((options.retainedEdges ?? []).filter((id) => edges.has(id)));
  roots.forEach((id) => selected.add(id));
  if (options.reveal && nodes.has(options.reveal)) {
    let id: string | undefined = options.reveal;
    while (id && !selected.has(id)) {
      selected.add(id);
      const step = parent.get(id);
      if (step) selectedEdges.add(step.edge);
      id = step?.node;
    }
  }
  const prioritized = options.reveal ? adjacency.get(options.reveal) ?? [] : [];
  for (const id of [...prioritized.map((next) => next.id), ...ordered]) {
    if (selected.size >= options.nodeBudget) break;
    selected.add(id);
  }
  for (const id of selected) {
    const adjacent = prioritized.find((next) => next.id === id);
    if (adjacent && options.reveal && selected.has(options.reveal)) selectedEdges.add(adjacent.edge.id);
    const step = parent.get(id);
    if (step && selected.has(step.node)) selectedEdges.add(step.edge);
  }
  const eligible = data.edges.filter((edge) => selected.has(edge.source_entity_id) && selected.has(edge.target_entity_id));
  const eligibleIds = new Set(eligible.map((edge) => edge.id));
  for (const id of selectedEdges) if (!eligibleIds.has(id)) selectedEdges.delete(id);
  for (const edge of [...eligible].sort((a, b) => a.id.localeCompare(b.id))) {
    if (selectedEdges.size >= options.edgeBudget) break;
    selectedEdges.add(edge.id);
  }
  const remaining = Object.fromEntries([...selected].map((id) => [id, new Set((adjacency.get(id) ?? []).filter((next) => !selected.has(next.id)).map((next) => next.id)).size]));
  return { data: { nodes: data.nodes.filter((node) => selected.has(node.id)), edges: eligible.filter((edge) => selectedEdges.has(edge.id)) },
    nodeIds: [...selected], edgeIds: [...selectedEdges], remaining };
}
