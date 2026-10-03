import { apiJson } from "../api.js";
import type { AttributeDefinition, DirectoryItem, Entity, EntityType, EntityTypeAction, Environment, GraphAspect, GraphResponse, Relationship, RelationshipType, RelationshipTypeCreate, RelationshipTypeUpdate, Session } from "./types";

const ROOT = "/api/ontology";
export class ApiError extends Error { constructor(message: string, readonly status: number, readonly code: string) { super(message); } }
async function request<T>(path: string, init?: RequestInit): Promise<T> {
  try {
    return await apiJson(init?.method ?? "GET", `${ROOT}${path}`, init?.body ? JSON.parse(String(init.body)) : undefined) as T;
  } catch (error) {
    const failure = error as Error & { status?: number; body?: { code?: string } };
    throw new ApiError(failure.message, failure.status ?? 0, failure.body?.code ?? "request_failed");
  }
}
const envPath = (env: string) => `/envs/${encodeURIComponent(env)}`;
export type GraphQuery = {
  centerIds?: string[];
  depth?: number;
  upstreamDepth?: number;
  downstreamDepth?: number;
  entityTypeIds?: string[];
  relationshipTypeIds?: string[];
  expandNeighbors?: boolean;
};
export const graphPath = (env: string, query: GraphQuery = {}) => {
  const params = new URLSearchParams();
  for (const id of new Set(query.centerIds ?? [])) params.append("center", id);
  if (query.depth !== undefined) params.set("depth", String(query.depth));
  if (query.upstreamDepth !== undefined) params.set("upstream_depth", String(query.upstreamDepth));
  if (query.downstreamDepth !== undefined) params.set("downstream_depth", String(query.downstreamDepth));
  for (const id of query.entityTypeIds ?? []) params.append("entity_type_id", id);
  for (const id of query.relationshipTypeIds ?? []) params.append("relationship_type_id", id);
  if (query.expandNeighbors) params.set("expand_neighbors", "true");
  return `${envPath(env)}/graph?${params}`;
};
export const api = {
  session: () => request<Session>("/session"),
  environments: (includeDeleted = false) => request<{ items: Environment[] }>(`/environments?include_deleted=${includeDeleted}`),
  createEnvironment: (value: object) => request<{ item: Environment }>("/environments", { method: "POST", body: JSON.stringify(value) }),
  updateEnvironment: (env: string, value: object) => request<{ item: Environment }>(`/environments/${encodeURIComponent(env)}`, { method: "PATCH", body: JSON.stringify(value) }),
  entityTypes: (env: string, includeDeleted = false) => request<{ items: EntityType[] }>(`${envPath(env)}/entity-types?include_deleted=${includeDeleted}`),
  createEntityType: (env: string, value: object) => request<{ item: EntityType }>(`${envPath(env)}/entity-types`, { method: "POST", body: JSON.stringify(value) }),
  updateEntityType: (env: string, id: string, value: object) => request(`${envPath(env)}/entity-types/${id}`, { method: "PATCH", body: JSON.stringify(value) }),
  attributes: (env: string, typeId: string) => request<{ items: AttributeDefinition[] }>(`${envPath(env)}/entity-types/${typeId}/attributes`),
  createAttribute: (env: string, typeId: string, value: object) => request<{ item: AttributeDefinition }>(`${envPath(env)}/entity-types/${typeId}/attributes`, { method: "POST", body: JSON.stringify(value) }),
  updateAttribute: (env: string, id: string, value: object) => request(`${envPath(env)}/attributes/${id}`, { method: "PATCH", body: JSON.stringify(value) }),
  actions: (env: string, typeId: string, includeDeleted = false) => request<{ items: EntityTypeAction[] }>(`${envPath(env)}/entity-types/${typeId}/actions?include_deleted=${includeDeleted}`),
  createAction: (env: string, typeId: string, value: object) => request<{ item: EntityTypeAction }>(`${envPath(env)}/entity-types/${typeId}/actions`, { method: "POST", body: JSON.stringify(value) }),
  updateAction: (env: string, id: string, value: object) => request<{ item: EntityTypeAction }>(`${envPath(env)}/actions/${id}`, { method: "PATCH", body: JSON.stringify(value) }),
  relationshipTypes: (env: string, includeDeleted = false) => request<{ items: RelationshipType[] }>(`${envPath(env)}/relationship-types?include_deleted=${includeDeleted}`),
  createRelationshipType: (env: string, value: RelationshipTypeCreate) => request<{ item: RelationshipType }>(`${envPath(env)}/relationship-types`, { method: "POST", body: JSON.stringify(value) }),
  updateRelationshipType: (env: string, id: string, value: RelationshipTypeUpdate) => request(`${envPath(env)}/relationship-types/${id}`, { method: "PATCH", body: JSON.stringify(value) }),
  graphAspects: (env: string, includeDeleted = false) => request<{ items: GraphAspect[] }>(`${envPath(env)}/graph-aspects?include_deleted=${includeDeleted}`),
  createGraphAspect: (env: string, value: object) => request<{ item: GraphAspect }>(`${envPath(env)}/graph-aspects`, { method: "POST", body: JSON.stringify(value) }),
  updateGraphAspect: (env: string, id: string, value: object) => request<{ item: GraphAspect }>(`${envPath(env)}/graph-aspects/${id}`, { method: "PATCH", body: JSON.stringify(value) }),
  deleteGraphAspect: (env: string, id: string, expectedRevision: number) => request<{ item: GraphAspect }>(`${envPath(env)}/graph-aspects/${id}?expected_revision=${expectedRevision}`, { method: "DELETE" }),
  entities: async (env: string, includeDeleted = false) => {
    const items = new Map<string, Entity>();
    for (let offset = 0; ; offset += 500) {
      const page = await request<{ items: Entity[] }>(`${envPath(env)}/entities?limit=500&offset=${offset}&include_deleted=${includeDeleted}`);
      for (const item of page.items) items.set(item.id, item);
      if (page.items.length < 500) return { items: [...items.values()] };
    }
  },
  entity: (env: string, id: string) => request<{ item: Entity; attribute_definitions?: AttributeDefinition[]; structured_attributes: unknown[]; text_attributes?: { definition: AttributeDefinition; current?: { content_path?: string; format?: string; revision?: number; bytes?: number } }[]; actions?: EntityTypeAction[]; needs_completion?: boolean }>(`${envPath(env)}/entities/${id}`),
  createEntity: (env: string, value: object) => request<{ item: Entity }>(`${envPath(env)}/entities`, { method: "POST", body: JSON.stringify(value) }),
  updateEntity: (env: string, id: string, value: object) => request<{ item: Entity }>(`${envPath(env)}/entities/${id}`, { method: "PATCH", body: JSON.stringify(value) }),
  setAttribute: (env: string, entity: string, attribute: string, value: object) => request(`${envPath(env)}/entities/${entity}/attributes/${attribute}`, { method: "PUT", body: JSON.stringify(value) }),
  directories: (env: string) => request<{ items: DirectoryItem[]; root_id: string }>(`${envPath(env)}/directories/tree`),
  createDirectory: (env: string, value: object) => request(`${envPath(env)}/directories/tree`, { method: "POST", body: JSON.stringify(value) }),
  moveDirectory: (env: string, id: string, parentId: string) => request(`${envPath(env)}/directories/${id}/move`, { method: "PATCH", body: JSON.stringify({ parent_id: parentId }) }),
  updateDirectory: (env: string, id: string, value: object) => request(`${envPath(env)}/directories/${id}`, { method: "PATCH", body: JSON.stringify(value) }),
  relationships: (env: string, includeDeleted = false) => request<{ items: Relationship[] }>(`${envPath(env)}/relationships?include_deleted=${includeDeleted}`),
  createRelationship: (env: string, value: object) => request<{ item: Relationship }>(`${envPath(env)}/relationships`, { method: "POST", body: JSON.stringify(value) }),
  updateRelationship: (env: string, id: string, value: object) => request<{ item: Relationship }>(`${envPath(env)}/relationships/${id}`, { method: "PATCH", body: JSON.stringify(value) }),
  graph: (env: string, query: GraphQuery = {}) => request<GraphResponse>(graphPath(env, query)),
  setText: (env: string, entity: string, attribute: string, value: object) => request(`${envPath(env)}/entities/${entity}/attributes/${attribute}/text`, { method: "PUT", body: JSON.stringify(value) }),
  textHistory: (env: string, entity: string, attribute: string) => request<{ items: Record<string, unknown>[] }>(`${envPath(env)}/entities/${entity}/attributes/${attribute}/text`),
  textContent: (env: string, entity: string, attribute: string, revision: number) => request<{ format: "md" | "html" | "nfs_path"; content: string; revision: number }>(`${envPath(env)}/entities/${entity}/attributes/${attribute}/text/${revision}`),
  setNfsPath: (env: string, entity: string, attribute: string, value: object) => request(`${envPath(env)}/entities/${entity}/attributes/${attribute}/nfs-path`, { method: "PUT", body: JSON.stringify(value) }),
};
