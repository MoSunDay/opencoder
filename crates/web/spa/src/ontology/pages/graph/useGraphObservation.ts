import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../../api";
import type { Entity, EntityType, GraphResponse, Relationship, RelationshipType } from "../../types";
import {
  entitiesOfTypes, incidentRelationshipTypeIds, initialSelection, relationshipTypeCandidates, retainAvailableIds, type ObservationSelection,
} from "./observationSelection";

type Metadata = { entities: Entity[]; entityTypes: EntityType[]; relationships: Relationship[]; relationshipTypes: RelationshipType[] };
const EMPTY_METADATA: Metadata = { entities: [], entityTypes: [], relationships: [], relationshipTypes: [] };
const EMPTY_GRAPH: GraphResponse = { nodes: [], edges: [], available_relationship_type_ids: [] };

async function loadMetadata(env: string): Promise<Metadata> {
  const [entities, entityTypes, relationships, relationshipTypes] = await Promise.all([
    api.entities(env), api.entityTypes(env), api.relationships(env), api.relationshipTypes(env),
  ]);
  return { entities: entities.items, entityTypes: entityTypes.items, relationships: relationships.items, relationshipTypes: relationshipTypes.items };
}

/** The owning component is keyed by ENV; no cache or selection crosses that boundary. */
export function useGraphObservation(env: string, expandNeighbors = false, active = true) {
  const [selection, setSelection] = useState(initialSelection);
  const [metadata, setMetadata] = useState(EMPTY_METADATA);
  const [data, setData] = useState(EMPTY_GRAPH);
  const [appliedSelection, setAppliedSelection] = useState<ObservationSelection>();
  const [metadataReady, setMetadataReady] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [metadataError, setMetadataError] = useState("");
  const metadataRef = useRef<Promise<Metadata>>();
  const requestRef = useRef(0);
  const mountedRef = useRef(false);

  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; requestRef.current += 1; };
  }, []);

  const reload = useCallback(async () => {
    if (!mountedRef.current) return;
    const request = ++requestRef.current;
    const current = () => mountedRef.current && request === requestRef.current;
    let correctingSelection = false;
    let metadataLoaded = false;
    setLoading(true);
    setError("");
    try {
      const pendingMetadata = metadataRef.current ?? loadMetadata(env);
      metadataRef.current = pendingMetadata;
      const nextMetadata = await pendingMetadata;
      if (!current()) return;
      setMetadata(nextMetadata);
      setMetadataReady(true);
      metadataLoaded = true;
      setMetadataError("");
      if (!active) return;
      const entityTypeIds = retainAvailableIds(selection.entityTypeIds, nextMetadata.entityTypes.map((item) => item.id));
      const centerIds = retainAvailableIds(selection.centerIds, entitiesOfTypes(nextMetadata.entities, entityTypeIds).map((item) => item.id));
      const incidentIds = incidentRelationshipTypeIds(nextMetadata.entities, nextMetadata.relationships, entityTypeIds);
      const candidateTypeIds = relationshipTypeCandidates(nextMetadata.relationshipTypes, entityTypeIds, expandNeighbors)
        .filter((item) => !expandNeighbors || incidentIds.has(item.id)).map((item) => item.id);
      const relationshipTypeIds = retainAvailableIds(selection.relationshipTypeIds, expandNeighbors ? nextMetadata.relationshipTypes.map((item) => item.id) : candidateTypeIds);
      if (entityTypeIds !== selection.entityTypeIds || centerIds !== selection.centerIds || relationshipTypeIds !== selection.relationshipTypeIds) {
        correctingSelection = true;
        setSelection({ ...selection, entityTypeIds, centerIds, relationshipTypeIds });
        return;
      }
      if (!entityTypeIds.length || !centerIds.length) { setData(EMPTY_GRAPH); return; }
      const nextGraph = await api.graph(env, {
        entityTypeIds, centerIds, upstreamDepth: selection.upstreamDepth, downstreamDepth: selection.downstreamDepth,
        ...(expandNeighbors ? { expandNeighbors: true } : {}),
        ...(relationshipTypeIds.length ? { relationshipTypeIds } : {}),
      });
      if (!current()) return;
      setData(nextGraph);
      setAppliedSelection({ ...selection });
    } catch (reason) {
      if (!current()) return;
      metadataRef.current = undefined;
      if (!metadataLoaded) setMetadataError(reason instanceof Error ? reason.message : "观测范围加载失败");
      setError(reason instanceof Error ? reason.message : "拓扑加载失败");
    } finally {
      if (current() && !correctingSelection) setLoading(false);
    }
  }, [env, selection, expandNeighbors, active]);

  useEffect(() => {
    void reload();
    return () => { requestRef.current += 1; };
  }, [reload]);

  const refresh = useCallback(async () => {
    metadataRef.current = undefined;
    await reload();
  }, [reload]);

  const changeSelection = (patch: Partial<ObservationSelection>) => {
    requestRef.current += 1;
    setLoading(true);
    setSelection((previous) => {
      const next = { ...previous, ...patch };
      if (patch.entityTypeIds) {
        const incidentIds = incidentRelationshipTypeIds(metadata.entities, metadata.relationships, next.entityTypeIds);
        const candidateIds = new Set(relationshipTypeCandidates(metadata.relationshipTypes, next.entityTypeIds, expandNeighbors)
          .filter((item) => !expandNeighbors || incidentIds.has(item.id)).map((item) => item.id));
        const centerIds = retainAvailableIds(next.centerIds, entitiesOfTypes(metadata.entities, next.entityTypeIds).map((item) => item.id));
        return { ...next, centerIds, relationshipTypeIds: expandNeighbors ? next.relationshipTypeIds : next.relationshipTypeIds.filter((id) => candidateIds.has(id)) };
      }
      return next;
    });
  };
  return { ...metadata, selection, appliedSelection, metadataReady, data, loading, error, metadataError, refresh, changeSelection };
}
