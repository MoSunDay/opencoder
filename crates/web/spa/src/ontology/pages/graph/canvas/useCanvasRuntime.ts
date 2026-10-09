import type { Graph } from "@antv/g6";
import { EdgeEvent, GraphEvent, NodeEvent } from "@antv/g6";
import { useCallback, useEffect, useRef, useState, type RefObject } from "react";
import type { GraphData } from "../../../types";
import { NODE_WIDTH, nodeHeight, focusNodeId } from "./presentation";

type Input = {
  container: RefObject<HTMLDivElement>;
  data: GraphData;
  centerIds: string[];
  focusKey?: string;
  loading: boolean;
  selectedNodeId?: string;
  selectedEdgeId?: string;
  onNodeClick: (id: string) => void;
  onEdgeClick: (id: string) => void;
  focusRequest?: { id: string; sequence: number };
};
type Click = { target: { id: string } };

export function useCanvasRuntime(input: Input) {
  const current = useRef(input);
  current.current = input;
  const graphRef = useRef<Graph>();
  const focused = useRef("");
  const handledFocus = useRef(0);
  const resizeFrame = useRef(0);
  const resizeObserver = useRef<ResizeObserver>();
  const [zoom, setZoom] = useState(1);
  const [runtimeError, setRuntimeError] = useState("");
  const perform = useCallback(async (action: (graph: Graph) => Promise<unknown>) => {
    const graph = graphRef.current;
    if (!graph || graph.destroyed) return;
    try { await action(graph); }
    catch (reason) { if (!graph.destroyed) setRuntimeError(reason instanceof Error ? reason.message : "画布操作失败"); }
  }, []);
  const updateZoom = useCallback(() => {
    const graph = graphRef.current;
    if (!graph || graph.destroyed) return;
    setZoom(graph.getZoom());
  }, []);
  const focus = useCallback(async (id: string) => {
    await perform(async (graph) => {
      if (!graph.getNodeData().some((node) => node.id === id)) return;
      const [x, y] = graph.getViewportByCanvas(graph.getElementPosition(id));
      const [cx, cy] = graph.getCanvasCenter();
      await graph.translateBy([cx - x, cy - y], false);
    });
  }, [perform]);
  const autoFrame = useCallback(async () => {
    await perform(async (graph) => {
      await graph.fitView(undefined, false);
      if (graph.destroyed) return;
      const scale = graph.getZoom();
      if (scale >= 0.8) { if (scale > 1) await graph.zoomTo(1, false); return; }
      await graph.zoomTo(0.8, false);
      if (graph.destroyed) return;
      const { data, centerIds } = current.current;
      const id = focusNodeId(data, centerIds);
      if (!id) return;
      const [x, y] = graph.getViewportByCanvas(graph.getElementPosition(id));
      const [cx, cy] = graph.getCanvasCenter();
      const incoming = data.edges.some((edge) => edge.target_entity_id === id);
      const outgoing = data.edges.some((edge) => edge.source_entity_id === id);
      const targetX = outgoing && !incoming ? Math.max(120, cx * 0.5) : incoming && !outgoing ? cx * 1.5 : cx;
      await graph.translateBy([targetX - x, cy - y], false);
    });
  }, [perform]);
  const focusCenter = useCallback(async () => {
    await perform(async (graph) => {
      // Aspect data can arrive before G6 has committed its new layout. Render
      // the current graph first so fit and translation use final node positions.
      // The fallback keeps the test double and older graph adapters usable.
      if (typeof graph.render === "function") await graph.render();
      else await graph.draw();
      if (graph.destroyed) return;
      await graph.fitView(undefined, false);
      if (graph.destroyed) return;
      await graph.zoomTo(1, false);
      if (graph.destroyed) return;
      const { data, centerIds } = current.current;
      const id = focusNodeId(data, centerIds);
      if (!id) return;
      const [x, y] = graph.getViewportByCanvas(graph.getElementPosition(id));
      const [cx, cy] = graph.getCanvasCenter();
      await graph.translateBy([cx - x, cy - y], false);
    });
  }, [perform]);
  const applyRequestedFocus = useCallback(() => {
    const request = current.current.focusRequest;
    if (!request || handledFocus.current === request.sequence || !graphRef.current?.getNodeData().some((node) => node.id === request.id)) return;
    handledFocus.current = request.sequence;
    void focus(request.id);
  }, [focus]);
  const applyFocus = useCallback(() => {
    const { data, centerIds, focusKey, loading } = current.current;
    const key = JSON.stringify([focusKey, centerIds]);
    if (loading || key === focused.current || !data.nodes.length) return;
    const id = focusNodeId(data, centerIds);
    const graph = graphRef.current;
    if (id && graph && !graph.destroyed && graph.getNodeData().some((node) => node.id === id)) {
      focused.current = key;
      void (focusKey ? focusCenter : autoFrame)();
    }
  }, [autoFrame, focusCenter]);
  const select = useCallback(() => {
    const graph = graphRef.current;
    if (!graph || graph.destroyed) return;
    const { selectedNodeId, selectedEdgeId } = current.current;
    const states = Object.fromEntries([...graph.getNodeData(), ...graph.getEdgeData()]
      .map((item) => [item.id, item.id === selectedNodeId || item.id === selectedEdgeId ? ["selected"] : []]));
    void perform((live) => live.setElementState(states, false));
  }, [perform]);
  const onReady = useCallback((graph: Graph) => {
    if (graph.destroyed) return;
    if (graphRef.current !== graph) {
      graphRef.current = graph;
      focused.current = "";
      graph.on(NodeEvent.CLICK, (event) => { if (!current.current.loading) current.current.onNodeClick(String((event as Click).target.id)); });
      graph.on(EdgeEvent.CLICK, (event) => { if (!current.current.loading) current.current.onEdgeClick(String((event as Click).target.id)); });
      graph.on(GraphEvent.AFTER_TRANSFORM, updateZoom);
      let anchor: ReturnType<Graph["getViewportCenter"]> | undefined;
      graph.on(GraphEvent.BEFORE_SIZE_CHANGE, () => { if (!graph.destroyed) anchor = graph.getCanvasByViewport(graph.getCanvasCenter()); });
      graph.on(GraphEvent.AFTER_SIZE_CHANGE, () => {
        if (!anchor || graph.destroyed) return;
        const [x, y] = graph.getViewportByCanvas(anchor);
        const [cx, cy] = graph.getCanvasCenter();
        void perform((live) => live.translateBy([cx - x, cy - y], false));
        cancelAnimationFrame(resizeFrame.current);
        // Responsive headers can resize the canvas more than once in one frame.
        // Wait for their layout to settle before checking the observation center.
        resizeFrame.current = requestAnimationFrame(() => {
          resizeFrame.current = requestAnimationFrame(() => {
            if (graph.destroyed || graphRef.current !== graph) return;
            const { data, centerIds, selectedNodeId, focusRequest, loading } = current.current;
            if (loading) return;
            const id = focusNodeId(data, [focusRequest?.id, selectedNodeId, ...centerIds].filter((value): value is string => Boolean(value)));
            if (!id || !graph.getNodeData().some((node) => node.id === id)) return;
            const [px, py] = graph.getViewportByCanvas(graph.getElementPosition(id));
            const [halfWidth, halfHeight] = graph.getCanvasCenter();
            const node = data.nodes.find((item) => item.id === id)!;
            const marginX = Math.min(halfWidth, NODE_WIDTH * graph.getZoom() / 2 + 12);
            const marginY = Math.min(halfHeight, nodeHeight(node.name || node.id) * graph.getZoom() / 2 + 12);
            if (px < marginX || py < marginY || px > halfWidth * 2 - marginX || py > halfHeight * 2 - marginY) void focus(id);
          });
        });
      });
      resizeObserver.current?.disconnect();
      const container = current.current.container.current;
      if (container) {
        resizeObserver.current = new ResizeObserver(() => {
          if (graph.destroyed || graphRef.current !== graph) return;
          const { clientWidth: width, clientHeight: height } = container;
          if (width > 0 && height > 0) graph.setSize(width, height);
        });
        resizeObserver.current.observe(container);
      }
    }
    setRuntimeError("");
    updateZoom();
    select();
    applyFocus();
    applyRequestedFocus();
  }, [applyFocus, applyRequestedFocus, select, updateZoom, perform, focus]);
  useEffect(() => () => { cancelAnimationFrame(resizeFrame.current); resizeObserver.current?.disconnect(); }, []);
  useEffect(() => { select(); }, [input.selectedNodeId, input.selectedEdgeId, select]);
  useEffect(() => {
    if (!input.focusKey) return;
    // FlowDirectionGraph keeps the G6 instance while an aspect replaces its
    // data. Re-run the one-shot focus after the new data and loading state have
    // settled instead of relying on onReady, which only fires for new graphs.
    focused.current = "";
    if (input.loading) return;
    let frame = requestAnimationFrame(() => {
      frame = requestAnimationFrame(() => applyFocus());
    });
    return () => cancelAnimationFrame(frame);
  }, [input.focusKey, input.loading, input.data, input.centerIds, applyFocus]);
  useEffect(applyRequestedFocus, [input.focusRequest, applyRequestedFocus]);
  const onZoom = (action: "in" | "out" | "reset" | "fit") => void perform(async (graph) => {
    if (action === "fit") await graph.fitView(undefined, false);
    else if (action === "reset") await graph.zoomTo(1, false);
    else await graph.zoomBy(action === "in" ? 1.2 : 1 / 1.2, false);
  });
  const onDestroy = useCallback(() => { cancelAnimationFrame(resizeFrame.current); resizeObserver.current?.disconnect(); graphRef.current = undefined; }, []);
  return { onReady, onDestroy, onFocus: (id: string) => void focus(id), autoFrame: () => void autoFrame(), onZoom, zoom, runtimeError };
}
