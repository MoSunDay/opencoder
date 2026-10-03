// @vitest-environment jsdom
import "../../../../test/setup-dom.js";
import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useGraphObservation } from "../useGraphObservation";
import type { Entity, EntityType, GraphResponse, RelationshipType } from "../../../types";

const apiMock = vi.hoisted(() => ({ graph: vi.fn(), entities: vi.fn(), entityTypes: vi.fn(), relationships: vi.fn(), relationshipTypes: vi.fn() }));
vi.mock("../../../api", () => ({ api: apiMock }));
const entityTypes: EntityType[] = [
  { id: "service", env_num: 1, type_key: "service", name: "服务", description: "", is_system: false, revision: 1, is_deleted: false },
  { id: "other", env_num: 1, type_key: "other", name: "其他", description: "", is_system: false, revision: 1, is_deleted: false },
];
const entities: Entity[] = [
  { id: "a", env_num: 1, entity_type_id: "service", name: "实体 A", description: "", revision: 1, is_deleted: false },
  { id: "b", env_num: 1, entity_type_id: "service", name: "实体 B", description: "", revision: 1, is_deleted: false },
  { id: "c", env_num: 1, entity_type_id: "other", name: "实体 C", description: "", revision: 1, is_deleted: false },
];
const relationshipTypes: RelationshipType[] = [
  {
    id: "calls", env_num: 1, type_key: "calls", name: "调用", description: "", is_directory_membership: false, is_system: false,
    source_entity_type_id: "service", target_entity_type_ids: ["service"], revision: 1, is_deleted: false,
  },
  {
    id: "depends", env_num: 1, type_key: "depends", name: "依赖", description: "", is_directory_membership: false, is_system: false,
    source_entity_type_id: "other", target_entity_type_ids: ["other"], revision: 1, is_deleted: false,
  },
];
const graph: GraphResponse = { nodes: entities, edges: [], available_relationship_type_ids: ["calls", "depends"] };
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

async function ready() {
  const hook = renderHook(() => useGraphObservation("debug"));
  await waitFor(() => expect(hook.result.current.loading).toBe(false));
  return hook;
}

describe("observation loading and selection", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    apiMock.entities.mockResolvedValue({ items: entities });
    apiMock.entityTypes.mockResolvedValue({ items: entityTypes });
    apiMock.relationships.mockResolvedValue({ items: [] });
    apiMock.relationshipTypes.mockResolvedValue({ items: relationshipTypes });
    apiMock.graph.mockResolvedValue(graph);
  });

  it("requests the graph only after both entity types and centers are chosen", async () => {
    const { result } = await ready();
    expect(apiMock.graph).not.toHaveBeenCalled();
    act(() => result.current.changeSelection({ entityTypeIds: ["service"] }));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(apiMock.graph).not.toHaveBeenCalled();
    expect(result.current.data.nodes).toEqual([]);
    act(() => result.current.changeSelection({ centerIds: ["a"] }));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(apiMock.graph).toHaveBeenLastCalledWith("debug", {
      entityTypeIds: ["service"], centerIds: ["a"], upstreamDepth: 1, downstreamDepth: 1,
    });
  });

  it("expands cross-type neighbors when expansion is enabled without requiring a relationship filter", async () => {
    apiMock.relationships.mockResolvedValue({ items: [{
      id: "edge", env_num: 1, relationship_type_id: "cross", source_entity_id: "a", target_entity_id: "c",
      description: "已确认的跨类型调用", revision: 1, is_deleted: false, is_pinned: false,
    }] });
    apiMock.relationshipTypes.mockResolvedValue({ items: [...relationshipTypes, {
      ...relationshipTypes[0], id: "cross", source_entity_type_id: "service", target_entity_type_ids: ["other"],
    }] });
    apiMock.graph.mockResolvedValue({ ...graph, available_relationship_type_ids: ["cross"] });
    const { result } = renderHook(() => useGraphObservation("debug", true));
    await waitFor(() => expect(result.current.loading).toBe(false));
    act(() => result.current.changeSelection({ entityTypeIds: ["other"], centerIds: ["c"] }));
    await waitFor(() => expect(apiMock.graph).toHaveBeenCalledWith("debug", {
      entityTypeIds: ["other"], centerIds: ["c"], upstreamDepth: 1, downstreamDepth: 1,
      expandNeighbors: true,
    }));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.relationshipTypes.some((item) => item.id === "cross")).toBe(true);
    act(() => result.current.changeSelection({ relationshipTypeIds: ["cross"] }));
    await waitFor(() => expect(apiMock.graph).toHaveBeenLastCalledWith("debug", {
      entityTypeIds: ["other"], centerIds: ["c"], upstreamDepth: 1, downstreamDepth: 1,
      relationshipTypeIds: ["cross"], expandNeighbors: true,
    }));
  });

  it("sends relationship type filters only while selected", async () => {
    const { result } = await ready();
    act(() => result.current.changeSelection({ entityTypeIds: ["service"], centerIds: ["a"], relationshipTypeIds: ["calls"] }));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(apiMock.graph).toHaveBeenLastCalledWith("debug", {
      entityTypeIds: ["service"], centerIds: ["a"], upstreamDepth: 1, downstreamDepth: 1, relationshipTypeIds: ["calls"],
    });
    act(() => result.current.changeSelection({ relationshipTypeIds: [] }));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(apiMock.graph).toHaveBeenLastCalledWith("debug", {
      entityTypeIds: ["service"], centerIds: ["a"], upstreamDepth: 1, downstreamDepth: 1,
    });
  });

  it("keeps the relationship type selection before centers are chosen", async () => {
    const { result } = await ready();
    act(() => result.current.changeSelection({ entityTypeIds: ["service"] }));
    await waitFor(() => expect(result.current.loading).toBe(false));
    act(() => result.current.changeSelection({ relationshipTypeIds: ["calls"] }));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.selection.relationshipTypeIds).toEqual(["calls"]);
    expect(apiMock.graph).not.toHaveBeenCalled();
    act(() => result.current.changeSelection({ centerIds: ["a"] }));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(apiMock.graph).toHaveBeenLastCalledWith("debug", {
      entityTypeIds: ["service"], centerIds: ["a"], upstreamDepth: 1, downstreamDepth: 1, relationshipTypeIds: ["calls"],
    });
  });

  it("cascades centers and relation types when entity types change", async () => {
    const { result } = await ready();
    act(() => result.current.changeSelection({ entityTypeIds: ["service", "other"] }));
    await waitFor(() => expect(result.current.loading).toBe(false));
    act(() => result.current.changeSelection({ centerIds: ["a", "c"], relationshipTypeIds: ["calls", "depends"] }));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(apiMock.graph).toHaveBeenLastCalledWith("debug", {
      entityTypeIds: ["service", "other"], centerIds: ["a", "c"], upstreamDepth: 1, downstreamDepth: 1,
      relationshipTypeIds: ["calls", "depends"],
    });
    act(() => result.current.changeSelection({ entityTypeIds: ["other"] }));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.selection.centerIds).toEqual(["c"]);
    expect(result.current.selection.relationshipTypeIds).toEqual(["depends"]);
    expect(apiMock.graph).toHaveBeenLastCalledWith("debug", {
      entityTypeIds: ["other"], centerIds: ["c"], upstreamDepth: 1, downstreamDepth: 1, relationshipTypeIds: ["depends"],
    });
  });

  it("prunes deleted centers when metadata refreshes", async () => {
    const { result } = await ready();
    act(() => result.current.changeSelection({ entityTypeIds: ["service"], centerIds: ["a", "b"] }));
    await waitFor(() => expect(result.current.loading).toBe(false));
    apiMock.entities.mockResolvedValue({ items: entities.filter((item) => item.id !== "a") });
    await act(async () => { await result.current.refresh(); });
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.selection.centerIds).toEqual(["b"]);
    expect(apiMock.graph).toHaveBeenLastCalledWith("debug", {
      entityTypeIds: ["service"], centerIds: ["b"], upstreamDepth: 1, downstreamDepth: 1,
    });
  });

  it("prunes deleted entity types with their dependent selections when metadata refreshes", async () => {
    const { result } = await ready();
    act(() => result.current.changeSelection({ entityTypeIds: ["service", "other"], centerIds: ["a", "c"], relationshipTypeIds: ["calls", "depends"] }));
    await waitFor(() => expect(result.current.loading).toBe(false));
    apiMock.entityTypes.mockResolvedValue({ items: entityTypes.filter((item) => item.id !== "other") });
    await act(async () => { await result.current.refresh(); });
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.selection.entityTypeIds).toEqual(["service"]);
    expect(result.current.selection.centerIds).toEqual(["a"]);
    expect(result.current.selection.relationshipTypeIds).toEqual(["calls"]);
    expect(apiMock.graph).toHaveBeenLastCalledWith("debug", {
      entityTypeIds: ["service"], centerIds: ["a"], upstreamDepth: 1, downstreamDepth: 1, relationshipTypeIds: ["calls"],
    });
  });

  it("keeps selected relation types when the graph has no matching edges", async () => {
    const { result } = await ready();
    apiMock.graph.mockResolvedValue({ ...graph, available_relationship_type_ids: ["depends"] });
    act(() => result.current.changeSelection({ entityTypeIds: ["service"], centerIds: ["a"], relationshipTypeIds: ["calls"] }));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.selection.relationshipTypeIds).toEqual(["calls"]);
    expect(apiMock.graph).toHaveBeenLastCalledWith("debug", {
      entityTypeIds: ["service"], centerIds: ["a"], upstreamDepth: 1, downstreamDepth: 1, relationshipTypeIds: ["calls"],
    });
  });

  it("ignores old graph successes and failures after changing the range", async () => {
    const { result } = await ready();
    const oldSuccess = deferred<GraphResponse>();
    const oldFailure = deferred<GraphResponse>();
    apiMock.graph.mockReturnValueOnce(oldSuccess.promise).mockReturnValueOnce(oldFailure.promise);
    act(() => result.current.changeSelection({ entityTypeIds: ["service"], centerIds: ["a"] }));
    await waitFor(() => expect(apiMock.graph).toHaveBeenCalledTimes(1));
    act(() => result.current.changeSelection({ centerIds: ["b"] }));
    await waitFor(() => expect(apiMock.graph).toHaveBeenCalledTimes(2));
    act(() => result.current.changeSelection({ downstreamDepth: 2 }));
    await waitFor(() => expect(result.current.loading).toBe(false));
    await act(async () => { oldSuccess.resolve({ nodes: [], edges: [], available_relationship_type_ids: [] }); oldFailure.reject(new Error("old failure")); });
    expect(result.current.selection.centerIds).toEqual(["b"]);
    expect(result.current.data).toEqual(graph);
    expect(result.current.error).toBe("");
  });

  it("does not publish an old metadata request after unmounting", async () => {
    const old = deferred<{ items: typeof entities }>();
    apiMock.entities.mockReturnValueOnce(old.promise);
    const first = renderHook(() => useGraphObservation("old"));
    first.unmount();
    const second = renderHook(() => useGraphObservation("new"));
    await waitFor(() => expect(second.result.current.loading).toBe(false));
    await act(async () => { old.resolve({ items: [] }); });
    act(() => second.result.current.changeSelection({ entityTypeIds: ["service"], centerIds: ["a"] }));
    await waitFor(() => expect(second.result.current.loading).toBe(false));
    expect(apiMock.graph).toHaveBeenLastCalledWith("new", {
      entityTypeIds: ["service"], centerIds: ["a"], upstreamDepth: 1, downstreamDepth: 1,
    });
    expect(second.result.current.entities).toEqual(entities);
  });

  it("reports graph errors and refreshes metadata on retry", async () => {
    const { result } = await ready();
    apiMock.graph.mockRejectedValueOnce(new Error("graph failed"));
    act(() => result.current.changeSelection({ entityTypeIds: ["service"], centerIds: ["a"] }));
    await waitFor(() => expect(result.current.error).toBe("graph failed"));
    expect(result.current.data.nodes).toEqual([]);
    await act(async () => { await result.current.refresh(); });
    expect(result.current.error).toBe("");
    expect(result.current.data).toEqual(graph);
    expect(apiMock.entities).toHaveBeenCalledTimes(2);
  });
});
