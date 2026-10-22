import { describe, expect, it, vi } from "vitest";
import { PerspectiveCamera, Vector3 } from "three";
import { sceneData } from "../model";
import { createEdge, disposeEdge, edgeCurve, updateEdge } from "../edges";
import { focusCamera, frameCamera, zoomCamera } from "../camera";
const nodes = ["a", "b"].map((id) => ({ id, env_num: 1, entity_type_id: "type", name: id, description: "", revision: 1, is_deleted: false }));
const edge = { id: "r1", env_num: 1, relationship_type_id: "rel", source_entity_id: "a", target_entity_id: "b", description: "", is_pinned: false, revision: 1, is_deleted: false };
describe("3D scene data and geometry", () => {
  it("owns simulation objects, preserves positions and never changes API records", () => {
    const input = { nodes: nodes.map((node) => Object.freeze({ ...node })), edges: [Object.freeze({ ...edge })] };
    const first = sceneData(input); first.nodes[0].x = 456; first.links[0].source = first.nodes[0];
    expect(sceneData(input, first)).toBe(first);
    const next = sceneData({ nodes: [...input.nodes, { ...nodes[0], id: "c" }], edges: input.edges }, first);
    expect(next.nodes[0]).toBe(first.nodes[0]); expect(next.nodes[0].x).toBe(456);
    expect(input.nodes[0]).not.toHaveProperty("x"); expect(input.edges[0].source_entity_id).toBe("a");
    expect(next.links[0].source).toBe("a");
    expect(sceneData({ nodes: [nodes[0]], edges: input.edges }).links).toEqual([]);
  });
  it("separates parallel and reverse relationships and draws self loops", () => {
    const data = sceneData({ nodes, edges: [edge, { ...edge, id: "r2", source_entity_id: "b", target_entity_id: "a" }, { ...edge, id: "r3", target_entity_id: "a" }] });
    const [a, b] = data.nodes;
    const forward = edgeCurve(a, b, data.links[0]), reverse = edgeCurve(b, a, data.links[1]);
    expect(forward.getPoint(0.5).distanceTo(reverse.getPoint(0.5))).toBeGreaterThan(1);
    const loop = edgeCurve(a, a, data.links[2]);
    expect(loop.getPoint(0.5).distanceTo(a)).toBeGreaterThan(1);
    expect(loop.getPoint(1).distanceTo(a)).toBeLessThan(0.00001);
  });
  it("draws the arrow along the curve and releases custom GPU resources", () => {
    const data = sceneData({ nodes, edges: [edge] }); const object = createEdge("#888");
    updateEdge(object, data.nodes[0], data.nodes[1], data.links[0]);
    const curve = edgeCurve(data.nodes[0], data.nodes[1], data.links[0]);
    expect(object.children[1].position.distanceTo(curve.getPoint(0.78))).toBeLessThan(0.00001);
    const direction = new Vector3(0, 1, 0).applyQuaternion(object.children[1].quaternion);
    expect(direction.dot(curve.getTangent(0.78))).toBeCloseTo(1);
    const dispose = vi.fn(); object.children.forEach((child) => { if ("geometry" in child) (child.geometry as { addEventListener: Function }).addEventListener("dispose", dispose); });
    disposeEdge(object); expect(dispose).toHaveBeenCalledTimes(2);
  });
});
describe("3D camera", () => {
  it("handles empty scenes, zoom bounds, reset and focus at the origin", () => {
    const camera = new PerspectiveCamera(50, 1.6, 0.1, 10000); camera.position.set(0, 0, 600);
    const target = new Vector3(); const graph = { camera: () => camera, controls: () => ({ target }), cameraPosition: vi.fn((p, t) => { camera.position.copy(p); target.copy(t); }) };
    frameCamera(graph, []); expect(graph.cameraPosition).not.toHaveBeenCalled();
    zoomCamera(graph, 1.2); expect(camera.position.z).toBeCloseTo(500);
    focusCamera(graph, { x: 100, y: 50, z: 20 }); expect(camera.position.clone().sub(target).length()).toBeCloseTo(500);
    const data = sceneData({ nodes, edges: [] }); frameCamera(graph, data.nodes, data.nodes[0], true);
    expect(target.distanceTo(data.nodes[0])).toBeLessThan(0.00001);
    const resetPosition = camera.position.clone();
    frameCamera(graph, data.nodes, data.nodes[0], true);
    expect(camera.position.distanceTo(resetPosition)).toBeLessThan(0.00001);
    zoomCamera(graph, 1e10); expect(camera.position.clone().sub(target).length()).toBeCloseTo(40);
  });
});
