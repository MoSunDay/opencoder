// @vitest-environment jsdom
import "../../../testSetup";
import { GraphEvent } from "@antv/g6";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import GraphCanvas from "../GraphCanvas";
import { focusNodeId } from "../canvas/presentation";

const callbacks = vi.hoisted(() => ({ ready: undefined as undefined | ((graph: unknown) => void), renders: 0 }));
vi.mock("antd", async (original) => { const actual = await original<typeof import("antd")>(); return { ...actual, Grid: { ...actual.Grid, useBreakpoint: () => ({ md: true }) } }; });
vi.mock("@ant-design/graphs", () => ({ FlowDirectionGraph: ({ onReady }: { onReady: (graph: unknown) => void }) => {
  callbacks.ready = onReady; callbacks.renders += 1; return null;
} }));
const data = { nodes: ["a", "b"].map((id) => ({ id, env_num: 1, entity_type_id: "type", name: id, description: "", revision: 1, is_deleted: false })), edges: [] };
function liveGraph() {
  return { destroyed: false, on: vi.fn(), setSize: vi.fn(), getZoom: vi.fn(() => 1), getNodeData: () => data.nodes, getEdgeData: () => [],
    setElementState: vi.fn().mockResolvedValue(undefined), updateEdgeData: vi.fn(), draw: vi.fn().mockResolvedValue(undefined), render: vi.fn().mockResolvedValue(undefined),
    zoomTo: vi.fn().mockResolvedValue(undefined), zoomBy: vi.fn().mockResolvedValue(undefined), focusElement: vi.fn().mockResolvedValue(undefined),
      getElementPosition: vi.fn(() => [100, 100]), getViewportByCanvas: vi.fn(() => [100, 100]),
      getCanvasByViewport: vi.fn(() => [1000, 500]), getCanvasCenter: vi.fn(() => [400, 250]), translateBy: vi.fn().mockResolvedValue(undefined),
    fitView: vi.fn().mockResolvedValue(undefined) };
}
function view(extra = {}) { return <GraphCanvas data={data} centerIds={["a"]} relationshipTypeNames={{}} onNodeClick={vi.fn()} onEdgeClick={vi.fn()} {...extra} />; }

describe("graph canvas interaction", () => {
  it("ignores a delayed destroyed callback and binds live handlers once", async () => {
    const onNodeClick = vi.fn(); render(view({ onNodeClick }));
    const stale = { destroyed: true, on: vi.fn() };
    act(() => callbacks.ready?.(stale)); expect(stale.on).not.toHaveBeenCalled();
    const live = liveGraph();
    await act(async () => { callbacks.ready?.(live); callbacks.ready?.(live); });
    expect(live.on).toHaveBeenCalledTimes(5);
    live.on.mock.calls[0][1]({ target: { id: "a" } }); expect(onNodeClick).toHaveBeenCalledWith("a");
    expect(live.fitView).toHaveBeenCalledWith(undefined, false);
  });
  it("preserves the graph and viewport during loading, refresh, and expansion", async () => {
    const onNodeClick = vi.fn(); const onEdgeClick = vi.fn(); const centers = ["a"]; const names = {};
    const props = { data, centerIds: centers, relationshipTypeNames: names, onNodeClick, onEdgeClick };
    const page = render(<GraphCanvas {...props} />); const live = liveGraph();
    await act(async () => callbacks.ready?.(live));
    const rendered = callbacks.renders; const focused = live.getElementPosition.mock.calls.length;
    page.rerender(<GraphCanvas {...props} loading />);
    expect(callbacks.renders).toBe(rendered);
    expect(screen.getByTestId("graph-surface")).toBeInTheDocument();
    page.rerender(<GraphCanvas {...props} data={{ ...data }} />);
    await act(async () => callbacks.ready?.(live));
    expect(live.getElementPosition).toHaveBeenCalledTimes(focused);
    fireEvent.click(screen.getByRole("button", { name: "放大画布" }));
    expect(screen.getByTestId("graph-surface")).toHaveClass("graph-surface-expanded");
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.getByTestId("graph-surface")).not.toHaveClass("graph-surface-expanded");
  });
  it("does not apply a delayed ready callback to a newer graph range", async () => {
    const page = render(view()); const previousReady = callbacks.ready;
    page.rerender(view({ data: { ...data, nodes: data.nodes.slice(1) } }));
    const live = liveGraph();
    await act(async () => previousReady?.(live));
    expect(live.on).not.toHaveBeenCalled(); expect(live.updateEdgeData).not.toHaveBeenCalled();
    await act(async () => callbacks.ready?.(live));
    expect(live.on).toHaveBeenCalledTimes(5);
  });
  it("provides explicit zoom and overview actions", async () => {
    render(view()); const live = liveGraph(); await act(async () => callbacks.ready?.(live));
    fireEvent.click(screen.getByRole("button", { name: "放大" })); await waitFor(() => expect(live.zoomBy).toHaveBeenCalledWith(1.2, false));
    fireEvent.click(screen.getByRole("button", { name: "自动取景" })); await waitFor(() => expect(live.fitView).toHaveBeenCalledWith(undefined, false));
  });
  it("centers a switched observation range at 100 percent", async () => {
    const page = render(view()); const live = liveGraph(); await act(async () => callbacks.ready?.(live));
    live.zoomTo.mockClear(); live.translateBy.mockClear();
    page.rerender(view({ focusKey: "aspect-2" }));
    await waitFor(() => expect(live.render).toHaveBeenCalled());
    await waitFor(() => expect(live.zoomTo).toHaveBeenCalledWith(1, false));
    expect(live.getElementPosition).toHaveBeenCalledWith("a");
    expect(live.translateBy).toHaveBeenCalledWith([300, 150], false);
  });
  it("focuses the only observation center directly", async () => {
    render(view()); const live = liveGraph(); await act(async () => callbacks.ready?.(live));
    live.getElementPosition.mockClear(); live.getViewportByCanvas.mockClear(); live.translateBy.mockClear();
    fireEvent.click(screen.getByRole("button", { name: "定位观测中心" }));
    await waitFor(() => expect(live.getElementPosition).toHaveBeenCalledWith("a"));
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    expect(live.translateBy).toHaveBeenCalled();
  });
  it("labels the complete list separately from the canvas display count", async () => {
    render(view());
    fireEvent.click(screen.getByText("列表"));
    expect(screen.getByText("完整结果：2 个实体 · 0 条关系")).toBeInTheDocument();
    expect(screen.queryByText(/^已展示 \d/)).not.toBeInTheDocument();
  });
  it("updates canvas dimensions when its container changes without a window resize", async () => {
    const watched = new Map<Element, () => void>();
    const disconnect = vi.fn();
    class ContainerObserver {
      constructor(private callback: () => void) {}
      observe(element: Element) { watched.set(element, this.callback); }
      disconnect = disconnect;
      unobserve() {}
    }
    vi.stubGlobal("ResizeObserver", ContainerObserver);
    try {
      const page = render(view()); const live = liveGraph();
      await act(async () => callbacks.ready?.(live));
      const container = page.container.querySelector(".graph-viewport")!;
      let width = 356; let height = 524;
      Object.defineProperties(container, { clientWidth: { get: () => width }, clientHeight: { get: () => height } });
      act(() => watched.get(container)!());
      expect(live.setSize).toHaveBeenLastCalledWith(356, 524);
      width = 1001; height = 452;
      act(() => watched.get(container)!());
      expect(live.setSize).toHaveBeenLastCalledWith(1001, 452);
      width = 0; height = 0;
      act(() => watched.get(container)!());
      expect(live.setSize).toHaveBeenCalledTimes(2);
      page.unmount(); expect(disconnect).toHaveBeenCalled();
    } finally { vi.unstubAllGlobals(); }
  });
  it("keeps the same graph coordinate at the viewport center after resizing", async () => {
    render(view()); const live = liveGraph(); await act(async () => callbacks.ready?.(live));
    const beforeResize = live.on.mock.calls.find(([event]) => event === GraphEvent.BEFORE_SIZE_CHANGE)![1];
    const afterResize = live.on.mock.calls.find(([event]) => event === GraphEvent.AFTER_SIZE_CHANGE)![1];
    beforeResize();
    expect(live.getCanvasByViewport).toHaveBeenCalledWith([400, 250]);
    live.getCanvasCenter.mockReturnValue([200, 150]);
    live.getViewportByCanvas.mockReturnValue([600, 250]);
    await act(async () => afterResize());
    expect(live.getViewportByCanvas).toHaveBeenCalledWith([1000, 500]);
    expect(live.translateBy).toHaveBeenLastCalledWith([-400, -100], false);
    expect(live.zoomTo).not.toHaveBeenCalled();
  });
  it("chooses a visible center", () => {
    expect(focusNodeId(data, ["missing", "b"])).toBe("b");
    expect(focusNodeId(data, [])).toBe("a");
  });
  it("keeps relationship labels when zooming a dense graph", async () => {
    const edges = Array.from({ length: 55 }, (_,i) => ({ id: `r${i}`, source_entity_id: "a", target_entity_id: "b", relationship_type_id: "rel",
      env_num: 1, description: "", revision: 1, is_deleted: false, is_pinned: false }));
    render(view({ data: { ...data, edges } })); const live = liveGraph(); await act(async () => callbacks.ready?.(live));
    const transform = live.on.mock.calls.find(([event]) => event === GraphEvent.AFTER_TRANSFORM)![1];
    live.getZoom.mockReturnValue(0.5);
    await act(async () => transform());
    expect(screen.getByRole("button", { name: "恢复 100%" })).toHaveTextContent("50%");
    expect(live.updateEdgeData).not.toHaveBeenCalled();
  });
});
