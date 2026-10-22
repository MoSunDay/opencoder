import type { GraphData } from "../../../types";

export type Point = { x: number; y: number; z: number };
export type SceneNode = Point & { id: string; name: string; typeId: string; vx?: number; vy?: number; vz?: number };
export type SceneLink = { id: string; source: string | SceneNode; target: string | SceneNode; typeId: string; curvature: number; rotation: number };
export type SceneData = { nodes: SceneNode[]; links: SceneLink[] };

const hash = (value: string) => Array.from(value).reduce((n, c) => ((n * 31 + c.charCodeAt(0)) >>> 0), 7);
export function typeColor(id: string) { return ["#1677ff", "#13a8a8", "#9254de", "#d48806", "#c41d7f", "#389e0d"][hash(id) % 6]; }

/** The simulation owns these objects; API records must never receive coordinates or object endpoints. */
export function sceneData(data: GraphData, previous?: SceneData): SceneData {
  const endpoint = (value: string | SceneNode) => typeof value === "string" ? value : value.id;
  if (previous && data.nodes.length === previous.nodes.length && data.edges.length === previous.links.length
    && data.nodes.every((node, index) => {
      const old = previous.nodes[index]; return old.id === node.id && old.name === node.name && old.typeId === node.entity_type_id;
    }) && data.edges.every((edge) => previous.links.some((link) => link.id === edge.id && link.typeId === edge.relationship_type_id
      && endpoint(link.source) === edge.source_entity_id && endpoint(link.target) === edge.target_entity_id))) return previous;
  const old = new Map(previous?.nodes.map((node) => [node.id, node]));
  const nodes = data.nodes.map((node) => {
    const angle = hash(node.id) * 2.399963;
    const item = old.get(node.id) ?? { id: node.id, x: Math.cos(angle) * 140, y: Math.sin(angle) * 140, z: (hash(node.id) % 11 - 5) * 28 };
    return Object.assign(item, { name: node.name, typeId: node.entity_type_id });
  });
  const ids = new Set(nodes.map((node) => node.id));
  const groups = new Map<string, typeof data.edges>();
  for (const edge of data.edges) {
    if (!ids.has(edge.source_entity_id) || !ids.has(edge.target_entity_id)) continue;
    const key = JSON.stringify([edge.source_entity_id, edge.target_entity_id].sort());
    groups.set(key, [...(groups.get(key) ?? []), edge]);
  }
  const links: SceneLink[] = [];
  for (const group of groups.values()) {
    group.sort((a, b) => a.id.localeCompare(b.id));
    group.forEach((edge, index) => links.push({ id: edge.id, source: edge.source_entity_id, target: edge.target_entity_id,
      typeId: edge.relationship_type_id, curvature: edge.source_entity_id === edge.target_entity_id ? 0.7 + index * 0.3 : group.length > 1 ? 0.22 : 0,
      rotation: index * 2 * Math.PI / group.length }));
  }
  return { nodes, links };
}
