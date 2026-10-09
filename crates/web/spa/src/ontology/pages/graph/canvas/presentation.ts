import type { GraphData } from "../../../types";

export const NODE_WIDTH = 220;

export function nodeTitle(name: string): { title: string; detail?: string } {
  const http = name.match(/^HTTP (\w+) (\S+)(.*)$/);
  if (!http) return { title: name };
  let operation = "";
  const selector = http[3].split(" · ")[0].trim();
  if (selector.startsWith("{")) {
    try { const body = JSON.parse(selector); operation = String(body.body_selector?.operation ?? ""); } catch { /* Preserve the original title in the tooltip. */ }
  }
  return { title: `${http[1]}${operation ? ` · ${operation}` : ""}`, detail: http[2] };
}
export function focusNodeId(data: GraphData, centerIds: string[]): string | undefined {
  const visible = new Set(data.nodes.map((node) => node.id));
  const center = centerIds.find((id) => visible.has(id));
  if (center) return center;
  const degrees = new Map(data.nodes.map((node) => [node.id, 0]));
  for (const edge of data.edges) {
    degrees.set(edge.source_entity_id, (degrees.get(edge.source_entity_id) ?? 0) + 1);
    degrees.set(edge.target_entity_id, (degrees.get(edge.target_entity_id) ?? 0) + 1);
  }
  return [...data.nodes].sort((a, b) => (degrees.get(b.id) ?? 0) - (degrees.get(a.id) ?? 0) || a.id.localeCompare(b.id))[0]?.id;
}
export function nodeHeight(name: string): number {
  return name ? 112 : 92;
}
