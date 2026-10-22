import { forwardRef, useCallback, useEffect, useImperativeHandle, useMemo, useRef, useState } from "react";
import ForceGraph3D, { type ForceGraphMethods, type ForceGraphProps } from "react-force-graph-3d";
import { Group, MeshBasicMaterial, type Mesh } from "three";
import type { GraphData } from "../../../types";
import { focusNodeId } from "../canvas/presentation";
import { focusCamera, frameCamera, zoomCamera } from "./camera";
import { createEdge, disposeEdge, updateEdge } from "./edges";
import Labels, { type LabelsHandle } from "./Labels";
import { sceneData, typeColor, type SceneData, type SceneLink, type SceneNode } from "./model";
import "./scene.css";

export type SceneHandle = { zoom: (action: "in" | "out" | "reset" | "fit") => void; autoFrame: () => void };
export type SceneProps = {
  data: GraphData; centerIds: string[]; focusKey?: string; focusRequest?: { id: string; sequence: number };
  visible: boolean; loading: boolean; remaining: Record<string, number>;
  entityTypeNames: Record<string, string>; relationshipTypeNames: Record<string, string>;
  selectedNodeId?: string; selectedEdgeId?: string; background: string; edgeColor: string; primary: string;
  onNodeClick: (id: string) => void; onEdgeClick: (id: string) => void; onExpand: (id: string) => void; onError: (message: string) => void;
};

export default forwardRef<SceneHandle, SceneProps>(function GraphScene(props, ref) {
  const graph = useRef<ForceGraphMethods<SceneNode, SceneLink>>();
  const host = useRef<HTMLDivElement>(null);
  const labels = useRef<LabelsHandle>(null);
  const previous = useRef<SceneData>();
  const data = useMemo(() => {
    const next = sceneData(props.data, previous.current); previous.current = next; return next;
  }, [props.data]);
  const [size, setSize] = useState({ width: 300, height: 300 });
  const [hasSize, setHasSize] = useState(true);
  const current = useRef({ props, data, size, hasSize }); current.current = { props, data, size, hasSize };
  const edges = useRef(new Map<string, Group>());
  const active = useRef(true);
  const focused = useRef("");
  const settledData = useRef<SceneData>();
  const handledRequest = useRef(0);
  const scope = JSON.stringify([props.focusKey, props.centerIds]);
  const safely = useCallback((action: (live: ForceGraphMethods<SceneNode, SceneLink>) => void) => {
    if (!active.current || !graph.current) return;
    try { action(graph.current); } catch (error) { current.current.props.onError(error instanceof Error ? error.message : "三维画布操作失败"); }
  }, []);
  const frame = useCallback((reset = false, overview = false) => safely((live) => {
    const { data, props } = current.current;
    const center = overview ? undefined : data.nodes.find((node) => node.id === focusNodeId(props.data, props.centerIds));
    frameCamera(live, data.nodes, center, reset);
  }), [safely]);
  useImperativeHandle(ref, () => ({
    zoom: (action) => action === "reset" ? frame(true) : action === "fit" ? frame(false, true)
      : safely((live) => zoomCamera(live, action === "in" ? 1.2 : 1 / 1.2)),
    autoFrame: () => frame(false, true),
  }), [frame, safely]);
  const applyFocus = useCallback(() => {
    const { props, data } = current.current;
    if (props.loading || !props.visible || !current.current.hasSize) return;
    const key = JSON.stringify([props.focusKey, props.centerIds]);
    if (focused.current !== key && data.nodes.length) { frame(true); focused.current = key; }
    const request = props.focusRequest;
    const node = data.nodes.find((item) => item.id === request?.id);
    if (request && node && handledRequest.current !== request.sequence) {
      safely((live) => focusCamera(live, node)); handledRequest.current = request.sequence;
    }
  }, [frame, safely]);
  // The callback belongs to this data version; an old layout cannot move the new view.
  const settled = () => {
    if (active.current && current.current.data === data) {
      settledData.current = data; applyFocus(); host.current?.setAttribute("aria-busy", "false");
    }
  };
  useEffect(() => { host.current?.setAttribute("aria-busy", "true"); }, [data]);
  useEffect(() => {
    if (props.loading || !props.visible) return;
    if (settledData.current === data) applyFocus();
  }, [props.focusRequest, props.loading, props.visible, hasSize, scope, data, applyFocus]);
  useEffect(() => {
    active.current = true;
    const container = host.current!;
    const resize = new ResizeObserver(() => {
      const { clientWidth: width, clientHeight: height } = container;
      setHasSize(width > 0 && height > 0);
      if (width > 0 && height > 0) setSize((old) => old.width === width && old.height === height ? old : { width, height });
    });
    resize.observe(container);
    const renderer = graph.current?.renderer();
    renderer?.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
    const lost = (event: Event) => { event.preventDefault(); current.current.props.onError("浏览器的三维绘图连接已中断"); };
    renderer?.domElement.addEventListener("webglcontextlost", lost);
    const force = graph.current?.d3Force("link");
    force?.distance(220);
    graph.current?.d3Force("charge")?.strength(-260);
    return () => {
      active.current = false; resize.disconnect(); renderer?.domElement.removeEventListener("webglcontextlost", lost);
      edges.current.forEach(disposeEdge); edges.current.clear();
    };
  }, []);
  useEffect(() => {
    const ids = new Set(data.links.map((link) => link.id));
    edges.current.forEach((edge, id) => { if (!ids.has(id)) { disposeEdge(edge); edges.current.delete(id); } });
  }, [data]);
  useEffect(() => {
    let animation = 0;
    const drawLabels = () => {
      if (!active.current || document.hidden || !current.current.props.visible || !current.current.hasSize) return;
      safely((live) => labels.current?.update(live.camera(), current.current.size.width, current.current.size.height));
      animation = requestAnimationFrame(drawLabels);
    };
    const visibility = () => {
      cancelAnimationFrame(animation);
      safely((live) => { if (document.hidden || !current.current.props.visible || !current.current.hasSize) live.pauseAnimation(); else { live.resumeAnimation(); drawLabels(); } });
    };
    visibility(); document.addEventListener("visibilitychange", visibility);
    return () => { cancelAnimationFrame(animation); document.removeEventListener("visibilitychange", visibility); };
  }, [props.visible, hasSize, safely]);
  const edgeObject = useCallback((link: SceneLink) => {
    let object = edges.current.get(link.id);
    if (!object) {
      const { edgeColor, primary, selectedEdgeId } = current.current.props;
      object = createEdge(link.id === selectedEdgeId ? primary : edgeColor); edges.current.set(link.id, object);
    }
    return object;
  }, []);
  useEffect(() => {
    edges.current.forEach((object, id) => {
      ((object.children[0] as Mesh).material as MeshBasicMaterial).color.set(id === props.selectedEdgeId ? props.primary : props.edgeColor);
    });
  }, [props.selectedEdgeId, props.primary, props.edgeColor, data]);
  const updateLink: ForceGraphProps<SceneNode, SceneLink>["linkPositionUpdate"] = (object, { start, end }, link) => {
    updateEdge(object as Group, start, end, link as unknown as SceneLink); return true;
  };
  return <div ref={host} className="graph-3d" data-testid="graph-3d" aria-label="三维关系图谱">
    <ForceGraph3D<SceneNode, SceneLink> ref={graph} graphData={data} width={size.width} height={size.height}
      backgroundColor={props.background} controlType="orbit" showNavInfo={false} numDimensions={3}
      nodeRelSize={7} nodeVal={(node) => props.centerIds.includes(node.id) ? 2 : 1}
      nodeColor={(node) => node.id === props.selectedNodeId ? props.primary : typeColor(node.typeId)} nodeOpacity={1}
      nodeLabel={() => ""} linkLabel={() => ""} linkThreeObject={edgeObject} linkPositionUpdate={updateLink}
      warmupTicks={80} cooldownTicks={120} cooldownTime={2500} onEngineStop={settled}
      enableNodeDrag={false} enablePointerInteraction={!props.loading}
      onNodeClick={(node) => { if (active.current && !current.current.props.loading) current.current.props.onNodeClick(node.id); }}
      onLinkClick={(link) => { if (active.current && !current.current.props.loading) current.current.props.onEdgeClick(link.id); }} />
    <Labels ref={labels} data={data} centers={props.centerIds} remaining={props.remaining} entityTypeNames={props.entityTypeNames}
      relationshipTypeNames={props.relationshipTypeNames} selectedNodeId={props.selectedNodeId} selectedEdgeId={props.selectedEdgeId}
      disabled={props.loading} onNodeClick={props.onNodeClick} onEdgeClick={props.onEdgeClick} onExpand={props.onExpand} />
    <div className="graph-3d-hint">拖动旋转 · 滚轮缩放 · 右键平移 · 双指缩放和平移</div>
  </div>;
});
