// @vitest-environment jsdom
import "../../../testSetup";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import GraphCanvas from "../GraphCanvas";
import type { SceneProps } from "../canvas3d/GraphScene";

const scene = vi.hoisted(() => ({ props: undefined as SceneProps | undefined, zoom: vi.fn(), autoFrame: vi.fn(), mounts: 0 }));
vi.mock("antd", async (original) => { const actual = await original<typeof import("antd")>(); return { ...actual, Grid: { ...actual.Grid, useBreakpoint: () => ({ md: true }) } }; });
vi.mock("../canvas3d/GraphScene", async () => {
  const React = await import("react");
  return { default: React.forwardRef((props: SceneProps, ref) => {
    scene.props = props;
    React.useImperativeHandle(ref, () => ({ zoom: scene.zoom, autoFrame: scene.autoFrame }));
    React.useEffect(() => { scene.mounts++; }, []);
    return <div data-testid="scene" />;
  }) };
});
const data = { nodes: ["a", "b"].map((id) => ({ id, env_num: 1, entity_type_id: "type", name: id, description: "", revision: 1, is_deleted: false })), edges: [] };
const props = { data, centerIds: ["a"], relationshipTypeNames: {}, onNodeClick: vi.fn(), onEdgeClick: vi.fn() };

beforeEach(() => { vi.clearAllMocks(); scene.mounts = 0; });
describe("3D graph workspace", () => {
  it("preserves the scene during loading, refresh, fullscreen and list switching", () => {
    const page = render(<GraphCanvas {...props} />);
    page.rerender(<GraphCanvas {...props} loading />);
    expect(scene.props?.loading).toBe(true);
    page.rerender(<GraphCanvas {...props} data={{ ...data }} />);
    fireEvent.click(screen.getByRole("button", { name: "放大画布" }));
    expect(screen.getByTestId("graph-surface")).toHaveClass("graph-surface-expanded");
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.getByTestId("graph-surface")).not.toHaveClass("graph-surface-expanded");
    fireEvent.click(screen.getByText("列表", { exact: true }));
    expect(scene.props?.visible).toBe(false);
    expect(screen.getByText("完整结果：2 个实体 · 0 条关系")).toBeInTheDocument();
    fireEvent.click(screen.getByText("3D 图", { exact: true }));
    expect(scene.props?.visible).toBe(true); expect(scene.mounts).toBe(1);
  });
  it("provides zoom, reset, overview and direct center focus", () => {
    const page = render(<GraphCanvas {...props} />);
    fireEvent.click(screen.getByRole("button", { name: "放大" })); expect(scene.zoom).toHaveBeenCalledWith("in");
    fireEvent.click(screen.getByRole("button", { name: "重置视角" })); expect(scene.zoom).toHaveBeenCalledWith("reset");
    fireEvent.click(screen.getByRole("button", { name: "自动取景" })); expect(scene.autoFrame).toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "定位观测中心" }));
    expect(scene.props?.focusRequest?.id).toBe("a");
    page.rerender(<GraphCanvas {...props} focusKey="other-aspect" />);
    expect(scene.props?.focusRequest).toBeUndefined();
  });
  it("reveals a node from the complete list before focusing it", () => {
    const nodes = Array.from({ length: 65 }, (_, i) => ({ ...data.nodes[0], id: `n${i}`, name: `node-${i}` }));
    render(<GraphCanvas {...props} data={{ nodes, edges: [] }} centerIds={["n0"]} />);
    expect(scene.props?.data.nodes).toHaveLength(30);
    fireEvent.click(screen.getByText("列表", { exact: true }));
    fireEvent.change(screen.getByLabelText("搜索完整观测结果"), { target: { value: "node-64" } });
    fireEvent.click(screen.getByRole("button", { name: "在图中定位" }));
    expect(scene.props?.data.nodes.some((node) => node.id === "n64")).toBe(true);
    expect(scene.props?.focusRequest?.id).toBe("n64"); expect(scene.props?.visible).toBe(true);
  });
  it("keeps the full list and allows retry after a graphics failure", () => {
    render(<GraphCanvas {...props} />);
    act(() => scene.props?.onError("WebGL unavailable"));
    expect(screen.getByText("三维画布暂不可用，已显示完整列表")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "a" })).toBeInTheDocument();
    expect(screen.queryByTestId("scene")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "重试三维画布" }));
    expect(screen.getByTestId("scene")).toBeInTheDocument(); expect(scene.mounts).toBe(2);
  });
  it("keeps the last successful graph on a query failure", () => {
    const retry = vi.fn(); render(<GraphCanvas {...props} error="查询失败" onRetry={retry} />);
    expect(screen.getByTestId("scene")).toBeInTheDocument();
    expect(screen.getByText("更新失败，当前显示上一次结果")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /重\s*试/ })); expect(retry).toHaveBeenCalled();
  });
  it("uses the list for more than 30 centers and can choose a single center", () => {
    const nodes = Array.from({ length: 31 }, (_, i) => ({ ...data.nodes[0], id: `n${i}`, name: `node-${i}` }));
    const choose = vi.fn(); render(<GraphCanvas {...props} data={{ nodes, edges: [] }} centerIds={nodes.map((node) => node.id)} onChooseCenter={choose} />);
    expect(screen.queryByTestId("scene")).not.toBeInTheDocument();
    fireEvent.click(screen.getAllByRole("button", { name: "在图中定位" })[0]); expect(choose).toHaveBeenCalledWith("n0");
  });
  it("does not initialize WebGL for an empty result", () => {
    render(<GraphCanvas {...props} data={{ nodes: [], edges: [] }} />);
    expect(screen.queryByTestId("scene")).not.toBeInTheDocument();
    expect(screen.getByText("当前范围暂无关系数据")).toBeInTheDocument();
  });
});
