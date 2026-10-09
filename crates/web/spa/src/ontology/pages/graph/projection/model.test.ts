// @vitest-environment jsdom
import "../../../testSetup";
import { describe, expect, it } from "vitest";
import type { Entity, GraphData, Relationship } from "../../../types";
import { projectGraph } from "./model";

const node = (id: string): Entity => ({ id, name: id, entity_type_id: "type", env_num: 1, description: "", revision: 1, is_deleted: false });
const edge = (from: string, to: string): Relationship => ({ id: `${from}-${to}`, source_entity_id: from, target_entity_id: to,
  relationship_type_id: "relation", env_num: 1, description: "", revision: 1, is_deleted: false, is_pinned: false });
const star = (count: number): GraphData => ({ nodes: Array.from({ length: count }, (_, index) => node(String(index).padStart(4, "0"))),
  edges: Array.from({ length: count - 1 }, (_, index) => edge("0000", String(index + 1).padStart(4, "0"))) });

describe("bounded graph presentation", () => {
  it("bounds thousand-node data, counts hidden neighbors and keeps the original result intact", () => {
    const full = star(1000);
    const result = projectGraph(full, ["0000"], { nodeBudget: 30, edgeBudget: 60 });
    expect(result.data.nodes).toHaveLength(30); expect(result.data.edges).toHaveLength(29);
    expect(result.remaining["0000"]).toBe(970);
    expect(full.nodes).toHaveLength(1000); expect(full.edges).toHaveLength(999);
    expect(result.data.edges.every((item) => full.edges.includes(item))).toBe(true);
  });
  it("expands without dropping existing nodes or paths", () => {
    const full = star(100);
    const first = projectGraph(full, ["0000"], { nodeBudget: 30, edgeBudget: 60 });
    const next = projectGraph(full, ["0000"], { nodeBudget: 50, edgeBudget: 100, retainedNodes: first.nodeIds, retainedEdges: first.edgeIds });
    expect(next.nodeIds).toHaveLength(50);
    expect(first.nodeIds.every((id) => next.nodeIds.includes(id))).toBe(true);
    expect(first.edgeIds.every((id) => next.edgeIds.includes(id))).toBe(true);
  });
  it("reveals a hidden search result with its complete real path", () => {
    const full: GraphData = { nodes: Array.from({ length: 50 }, (_, index) => node(String(index))),
      edges: Array.from({ length: 49 }, (_, index) => edge(String(index), String(index + 1))) };
    const result = projectGraph(full, ["0"], { nodeBudget: 30, edgeBudget: 60, reveal: "49" });
    expect(result.nodeIds).toHaveLength(50); expect(result.data.edges).toHaveLength(49);
    expect(result.data.edges.every((item) => result.nodeIds.includes(item.source_entity_id) && result.nodeIds.includes(item.target_entity_id))).toBe(true);
  });
  it("prioritizes the neighbors of the clicked node during expansion", () => {
    const full = star(100);
    full.nodes.push(node("z-child")); full.edges.push(edge("0001", "z-child"));
    const first = projectGraph(full, ["0000"], { nodeBudget: 30, edgeBudget: 60 });
    const result = projectGraph(full, ["0000"], { nodeBudget: 31, edgeBudget: 100, retainedNodes: first.nodeIds, retainedEdges: first.edgeIds, reveal: "0001" });
    expect(result.nodeIds).toContain("z-child"); expect(result.edgeIds).toContain("0001-z-child");
  });
  it("limits dense edges and keeps their connecting paths", () => {
    const full = star(30);
    for (let from = 1; from < 30; from++) for (let to = from + 1; to < 30; to++) full.edges.push(edge(full.nodes[from].id, full.nodes[to].id));
    const result = projectGraph(full, ["0000"], { nodeBudget: 30, edgeBudget: 60 });
    expect(result.data.edges).toHaveLength(60);
    expect(full.edges.slice(0, 29).every((item) => result.edgeIds.includes(item.id))).toBe(true);
  });
});
