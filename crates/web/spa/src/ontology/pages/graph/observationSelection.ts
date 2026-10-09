import type { Entity, GraphResponse, Relationship, RelationshipType } from "../../types";

export type ObservationSelection = {
  entityTypeIds: string[];
  relationshipTypeIds: string[];
  centerIds: string[];
  upstreamDepth: number;
  downstreamDepth: number;
};

export const initialSelection = (): ObservationSelection => ({
  entityTypeIds: [], relationshipTypeIds: [], centerIds: [], upstreamDepth: 3, downstreamDepth: 3,
});

/** Preserve the reference when no selection has become invalid. */
export function retainAvailableIds(selected: string[], available: Iterable<string>): string[] {
  const ids = new Set(available);
  const retained = selected.filter((id) => ids.has(id));
  return retained.length === selected.length ? selected : retained;
}

/** 关系类型候选：source_entity_type_id ∈ 所选实体类型 ∪ 未声明；target_entity_type_ids ∩ 所选 ≠ ∅ ∪ 未声明。 */
export function relationshipTypeCandidates(relationshipTypes: RelationshipType[], entityTypeIds: string[], expandNeighbors = false): RelationshipType[] {
  const selected = new Set(entityTypeIds);
  return relationshipTypes.filter((item) => {
    const sourceOk = !item.source_entity_type_id || selected.has(item.source_entity_type_id);
    const targets = item.target_entity_type_ids ?? [];
    const targetOk = !targets.length || targets.some((id) => selected.has(id));
    return expandNeighbors
      ? (!item.source_entity_type_id && !targets.length)
        || Boolean(item.source_entity_type_id && selected.has(item.source_entity_type_id))
        || targets.some((id) => selected.has(id))
      : sourceOk && targetOk;
  });
}

export function entitiesOfTypes(entities: Entity[], entityTypeIds: string[]): Entity[] {
  const types = new Set(entityTypeIds);
  return entities.filter((item) => types.has(item.entity_type_id));
}

export function incidentRelationshipTypeIds(entities: Entity[], relationships: Relationship[], entityTypeIds: string[]): Set<string> {
  const selected = new Set(entityTypeIds);
  const entityTypes = new Map(entities.map((item) => [item.id, item.entity_type_id]));
  return new Set(relationships
    .filter((item) => selected.has(entityTypes.get(item.source_entity_id) ?? "")
      || selected.has(entityTypes.get(item.target_entity_id) ?? ""))
    .map((item) => item.relationship_type_id));
}

/** Only describe connectivity in the returned scope; hidden hops may connect the centers elsewhere. */
export function centersDisconnected(graph: GraphResponse, centerIds: string[], excludedRelationshipTypeIds: ReadonlySet<string> = new Set()): boolean {
  const centers = [...new Set(centerIds)];
  if (centers.length < 2) return false;
  const visible = new Set(graph.nodes.map((node) => node.id));
  if (centers.some((id) => !visible.has(id))) return false;
  const neighbors = new Map([...visible].map((id) => [id, new Set<string>()]));
  for (const edge of graph.edges) {
    if (excludedRelationshipTypeIds.has(edge.relationship_type_id)) continue;
    if (!visible.has(edge.source_entity_id) || !visible.has(edge.target_entity_id)) continue;
    neighbors.get(edge.source_entity_id)?.add(edge.target_entity_id);
    neighbors.get(edge.target_entity_id)?.add(edge.source_entity_id);
  }
  const reached = new Set([centers[0]]);
  const pending = [centers[0]];
  while (pending.length) {
    for (const neighbor of neighbors.get(pending.pop()!) ?? []) {
      if (reached.has(neighbor)) continue;
      reached.add(neighbor);
      pending.push(neighbor);
    }
  }
  return centers.some((id) => !reached.has(id));
}

/** 关系类型选项标注「源类型→目标类型」，未声明端显示「不限」。 */
export function relationshipTypeLabel(item: RelationshipType, typeNames: Record<string, string>): string {
  const source = item.source_entity_type_id ? typeNames[item.source_entity_type_id] ?? item.source_entity_type_id : "不限";
  const targets = (item.target_entity_type_ids ?? []).map((id) => typeNames[id] ?? id);
  if (!item.source_entity_type_id && !targets.length) return `${item.name}（不限端点类型）`;
  return `${item.name}（${source}→${targets.length ? targets.join("、") : "不限"}）`;
}
