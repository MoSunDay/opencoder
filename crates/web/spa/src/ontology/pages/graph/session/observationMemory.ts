import type { Entity, EntityType, GraphAspect, RelationshipType } from "../../../types";
import type { ObservationSelection } from "../observationSelection";

export type ObservationMemory = { mode: "aspect-observe" | "aspect-test"; aspectKey?: string; selection: ObservationSelection };
const storageKey = (env: string) => `oc_ontology_observation:${env}`;
export function readObservation(env: string): ObservationMemory | undefined {
  try {
    const value = JSON.parse(localStorage.getItem(storageKey(env)) || "null");
    const selection = value?.selection;
    if (!["aspect-observe", "aspect-test"].includes(value?.mode) || !selection) return;
    if (!["entityTypeIds", "relationshipTypeIds", "centerIds"].every((key) => Array.isArray(selection[key]) && selection[key].every((id: unknown) => typeof id === "string"))) return;
    if (![selection.upstreamDepth, selection.downstreamDepth].every((depth) => Number.isInteger(depth) && depth >= 0 && depth <= 9)) return;
    return value;
  } catch { return; }
}
export function writeObservation(env: string, value: ObservationMemory) {
  try { localStorage.setItem(storageKey(env), JSON.stringify(value)); } catch { /* Storage can be unavailable; observation remains usable. */ }
}
export function aspectSelection(aspect: GraphAspect): ObservationSelection {
  return { entityTypeIds: aspect.entity_type_ids, relationshipTypeIds: aspect.relationship_type_ids, centerIds: aspect.default_center_ids,
    upstreamDepth: aspect.default_upstream_depth ?? 3, downstreamDepth: aspect.default_downstream_depth ?? 3 };
}
export function validSelection(value: ObservationSelection, metadata: { entities: Entity[]; entityTypes: EntityType[]; relationshipTypes: RelationshipType[] }) {
  const types = new Set(metadata.entityTypes.filter((item) => !item.is_deleted).map((item) => item.id));
  const entities = new Set(metadata.entities.filter((item) => !item.is_deleted && value.entityTypeIds.includes(item.entity_type_id)).map((item) => item.id));
  const relationships = new Set(metadata.relationshipTypes.filter((item) => !item.is_deleted).map((item) => item.id));
  return value.entityTypeIds.length > 0 && value.entityTypeIds.every((id) => types.has(id)) && value.centerIds.every((id) => entities.has(id))
    && value.relationshipTypeIds.every((id) => relationships.has(id));
}
export function recommendedAspect(aspects: GraphAspect[]) {
  return [...aspects].sort((a, b) => a.aspect_key.localeCompare(b.aspect_key))[0];
}
