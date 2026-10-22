import type { Entity, GraphAspect } from "../../../types";
import type { ObservationLink } from "../../../navigation/observationLink";
import { aspectSelection, type ObservationMemory } from "./observationMemory";

export function linkedObservation(link: ObservationLink | null, entities: Entity[], aspects: GraphAspect[]): ObservationMemory | undefined {
  if (!link) return;
  const aspect = aspects.find((item) => !item.is_deleted && item.aspect_key === link.aspectKey);
  const center = entities.find((item) => !item.is_deleted && item.id === link.centerId);
  if ((link.aspectKey && !aspect) || (link.centerId && !center)) return;
  if (center && aspect && !aspect.entity_type_ids.includes(center.entity_type_id)) return;
  const selection = aspect ? aspectSelection(aspect) : center ? {
    entityTypeIds: [center.entity_type_id], relationshipTypeIds: [], centerIds: [center.id], upstreamDepth: 1, downstreamDepth: 1,
  } : undefined;
  if (!selection) return;
  return { mode: aspect ? "aspect-observe" : "aspect-test", aspectKey: aspect?.aspect_key, selection: {
    ...selection, centerIds: center ? [center.id] : selection.centerIds,
    upstreamDepth: link.upstreamDepth ?? selection.upstreamDepth, downstreamDepth: link.downstreamDepth ?? selection.downstreamDepth,
  } };
}
