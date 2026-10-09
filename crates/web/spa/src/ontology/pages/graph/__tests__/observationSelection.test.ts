// @vitest-environment jsdom
import "../../../testSetup";
import { describe, expect, it } from "vitest";
import type { GraphResponse } from "../../../types";
import { centersDisconnected } from "../observationSelection";

const graph = (ids: string[], pairs: Array<[string, string]>) => ({
  nodes: ids.map((id) => ({ id, env_num: 1, entity_type_id: "type", name: id,
    description: "", revision: 1, is_deleted: false })),
  edges: pairs.map(([source_entity_id, target_entity_id], index) => ({
    id: String(index), env_num: 1, relationship_type_id: "link", source_entity_id, target_entity_id,
    description: "", revision: 1, is_deleted: false, is_pinned: false,
  })),
  available_relationship_type_ids: [],
}) satisfies GraphResponse;

describe("centersDisconnected", () => {
  it("finds connected centers through visible intermediate nodes", () => {
    expect(centersDisconnected(graph(["a", "b", "c"], [["a", "b"], ["b", "c"]]), ["a", "c"])).toBe(false);
  });

  it("reports distinct visible components, including centers with no visible edges", () => {
    expect(centersDisconnected(graph(["a", "b", "c"], [["a", "b"]]), ["a", "c"])).toBe(true);
    expect(centersDisconnected(graph(["a", "c"], []), ["a", "c"])).toBe(true);
  });

  it("does not count directory containment as a business path", () => {
    const data = graph(["root", "a", "b"], [["root", "a"], ["root", "b"]]);
    data.edges.forEach((edge) => { edge.relationship_type_id = "contains"; });
    expect(centersDisconnected(data, ["a", "b"])).toBe(false);
    expect(centersDisconnected(data, ["a", "b"], new Set(["contains"]))).toBe(true);
  });

  it("does not claim a path is missing for one or absent centers", () => {
    expect(centersDisconnected(graph(["a"], []), ["a"])).toBe(false);
    expect(centersDisconnected(graph(["a"], []), ["a", "missing"])).toBe(false);
  });
});
