// @vitest-environment jsdom
import "../../../testSetup";
import { act, fireEvent, render } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { PerspectiveCamera, Vector3 } from "three";
import type { ForceGraphProps } from "react-force-graph-3d";
import GraphScene, { type SceneProps } from "../canvas3d/GraphScene";
import type { SceneLink, SceneNode } from "../canvas3d/model";

const state = vi.hoisted(() => ({ props: undefined as ForceGraphProps<SceneNode, SceneLink> | undefined,
  live: { camera: vi.fn(), controls: vi.fn(), cameraPosition: vi.fn(), renderer: vi.fn(), d3Force: vi.fn(), pauseAnimation: vi.fn(), resumeAnimation: vi.fn() } }));
vi.mock("react-force-graph-3d", async () => {
  const { forwardRef, useImperativeHandle } = await import("react");
  return { default: forwardRef((props: ForceGraphProps<SceneNode, SceneLink>, ref) => {
    state.props = props; useImperativeHandle(ref, () => state.live); return <canvas data-testid="renderer" />;
  }) };
});
const nodes = ["a", "b"].map((id) => ({ id, env_num: 1, entity_type_id: "type", name: id, description: "", revision: 1, is_deleted: false }));
const base: SceneProps = { data: { nodes, edges: [] }, centerIds: ["a"], visible: true, loading: false, remaining: {}, entityTypeNames: {},
  relationshipTypeNames: {}, background: "#fff", primary: "#1677ff", edgeColor: "#888", onError: vi.fn(), onNodeClick: vi.fn(), onEdgeClick: vi.fn(), onExpand: vi.fn() };
let camera: PerspectiveCamera;
let target: Vector3;
let canvas: HTMLCanvasElement;
let resize: () => void;
const disconnect = vi.fn();
beforeEach(() => {
  vi.clearAllMocks(); camera = new PerspectiveCamera(50, 1.6, 0.1, 100000); camera.position.set(0, 0, 600); target = new Vector3();
  canvas = document.createElement("canvas");
  state.live.camera.mockReturnValue(camera); state.live.controls.mockReturnValue({ target });
  state.live.renderer.mockReturnValue({ domElement: canvas, setPixelRatio: vi.fn() });
  state.live.cameraPosition.mockImplementation((position, point) => { camera.position.copy(position); target.copy(point); camera.lookAt(target); camera.updateMatrixWorld(); });
  vi.stubGlobal("ResizeObserver", class { constructor(callback: () => void) { resize = callback; } observe() {} disconnect = disconnect; });
});

describe("3D scene lifecycle", () => {
  it("centers after layout and preserves the camera on refresh and expansion", () => {
    const page = render(<GraphScene {...base} />);
    act(() => state.props?.onEngineStop?.());
    expect(target.x).toBe(state.props?.graphData?.nodes[0].x);
    const calls = state.live.cameraPosition.mock.calls.length;
    page.rerender(<GraphScene {...base} loading />);
    page.rerender(<GraphScene {...base} data={{ ...base.data }} />);
    act(() => state.props?.onEngineStop?.());
    expect(state.live.cameraPosition).toHaveBeenCalledTimes(calls);
    page.rerender(<GraphScene {...base} data={{ nodes: [...nodes, { ...nodes[0], id: "c" }], edges: [] }} />);
    act(() => state.props?.onEngineStop?.());
    expect(state.live.cameraPosition).toHaveBeenCalledTimes(calls);
  });
  it("rejects stale layout callbacks and resets a new aspect only after its layout", () => {
    const page = render(<GraphScene {...base} />); const stale = state.props?.onEngineStop;
    page.rerender(<GraphScene {...base} focusKey="other" data={{ nodes: [nodes[1]], edges: [] }} centerIds={["b"]} />);
    act(() => stale?.()); expect(state.live.cameraPosition).not.toHaveBeenCalled();
    act(() => state.props?.onEngineStop?.());
    expect(target.x).toBe(state.props?.graphData?.nodes[0].x);
    const count = state.live.cameraPosition.mock.calls.length;
    page.unmount(); act(() => state.props?.onEngineStop?.());
    expect(state.live.cameraPosition).toHaveBeenCalledTimes(count); expect(disconnect).toHaveBeenCalled();
  });
  it("focuses an already settled layout when only the aspect changes", () => {
    const page = render(<GraphScene {...base} />); act(() => state.props?.onEngineStop?.());
    const oldCount = state.live.cameraPosition.mock.calls.length;
    page.rerender(<GraphScene {...base} focusKey="same-data" centerIds={["b"]} />);
    expect(state.live.cameraPosition).toHaveBeenCalledTimes(oldCount + 1);
    expect(target.x).toBe(state.props!.graphData!.nodes[1].x);
  });
  it("applies pending focus when loading ends after layout has stopped", () => {
    const page = render(<GraphScene {...base} loading />); act(() => state.props?.onEngineStop?.());
    expect(state.live.cameraPosition).not.toHaveBeenCalled();
    page.rerender(<GraphScene {...base} />);
    expect(state.live.cameraPosition).toHaveBeenCalledTimes(1);
  });
  it("focuses requested nodes without changing viewing direction or distance", () => {
    const page = render(<GraphScene {...base} />); act(() => state.props?.onEngineStop?.());
    const offset = camera.position.clone().sub(target);
    page.rerender(<GraphScene {...base} focusRequest={{ id: "b", sequence: 1 }} />);
    const node = state.props!.graphData!.nodes[1];
    expect(target.toArray()).toEqual([node.x, node.y, node.z]);
    expect(camera.position.clone().sub(target).distanceTo(offset)).toBeLessThan(0.00001);
  });
  it("updates dimensions without moving the camera and ignores zero-sized containers", () => {
    const page = render(<GraphScene {...base} />); act(() => state.props?.onEngineStop?.());
    const host = page.getByTestId("graph-3d");
    Object.defineProperties(host, { clientWidth: { value: 390, configurable: true }, clientHeight: { value: 600, configurable: true } });
    const count = state.live.cameraPosition.mock.calls.length;
    act(() => resize()); expect(state.props?.width).toBe(390); expect(state.props?.height).toBe(600);
    Object.defineProperty(host, "clientWidth", { value: 0 }); act(() => resize());
    expect(state.props?.width).toBe(390); expect(state.live.cameraPosition).toHaveBeenCalledTimes(count);
  });
  it("pauses hidden scenes and reports context loss", () => {
    const page = render(<GraphScene {...base} />);
    expect(state.live.resumeAnimation).toHaveBeenCalled();
    page.rerender(<GraphScene {...base} visible={false} />); expect(state.live.pauseAnimation).toHaveBeenCalled();
    act(() => canvas.dispatchEvent(new Event("webglcontextlost", { cancelable: true })));
    expect(base.onError).toHaveBeenCalledWith("浏览器的三维绘图连接已中断");
    page.unmount(); vi.clearAllMocks(); canvas.dispatchEvent(new Event("webglcontextlost")); expect(base.onError).not.toHaveBeenCalled();
  });
  it("opens labels and meshes through the same callbacks and blocks them while loading", () => {
    const page = render(<GraphScene {...base} />);
    fireEvent.click(page.getByTitle("a")); expect(base.onNodeClick).toHaveBeenCalledWith("a");
    const oldClick = state.props!.onNodeClick!;
    page.rerender(<GraphScene {...base} loading />); vi.mocked(base.onNodeClick).mockClear();
    oldClick({ ...state.props!.graphData!.nodes[0] }, new MouseEvent("click"));
    expect(base.onNodeClick).not.toHaveBeenCalled();
    expect(page.getByTitle("a")).toBeDisabled();
  });
});
