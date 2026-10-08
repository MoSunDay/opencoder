import { FlowDirectionGraph, type GraphOptions } from "@ant-design/graphs";
import type { NodeData } from "@antv/g6";
import { Alert, Button, Empty, Spin, theme } from "antd";
import { memo, useEffect, useMemo, useRef, useState } from "react";
import type { GraphData } from "../../types";
import GraphNode from "./canvas/GraphNode";
import GraphToolbar from "./canvas/GraphToolbar";
import { NODE_WIDTH, nodeHeight } from "./canvas/presentation";
import { RELATIONSHIP_EDGE } from "./canvas/RelationshipEdge";
import { relationshipLabel, RELATIONSHIP_FONT_FAMILY, RELATIONSHIP_FONT_SIZE, RELATIONSHIP_LINE_HEIGHT } from "./canvas/relationshipLabel";
import { useCanvasRuntime } from "./canvas/useCanvasRuntime";
import { coordinateGraphLifecycle } from "./graphLifecycle";
import { useProjection } from "./projection/useProjection";
import ResultList from "./projection/ResultList";

const StableGraph = memo(FlowDirectionGraph);
type Props = {
  data: GraphData;
  centerIds: string[];
  entityTypeNames?: Record<string, string>;
  relationshipTypeNames: Record<string, string>;
  focusKey?: string;
  loading?: boolean;
  error?: string;
  notice?: string;
  appliedSummary?: string;
  onChooseCenter?: (id: string) => void;
  emptyMessage?: string;
  selectedNodeId?: string;
  selectedEdgeId?: string;
  onRetry?: () => Promise<void>;
  onNodeClick: (id: string) => void;
  onEdgeClick: (id: string) => void;
};

export default function GraphCanvas({ data, centerIds, entityTypeNames, relationshipTypeNames, focusKey, loading = false,
  error, notice, appliedSummary, onChooseCenter, emptyMessage = "当前范围暂无关系数据", selectedNodeId, selectedEdgeId, onRetry, onNodeClick, onEdgeClick }: Props) {
  const { token } = theme.useToken();
  const viewport = useRef<HTMLDivElement>(null);
  const focusSequence = useRef(0);
  const [expanded, setExpanded] = useState(false);
  const [view, setView] = useState("图");
  const [focusRequest, setFocusRequest] = useState<{ id: string; sequence: number }>();
  const projection = useProjection(data, centerIds, focusKey);
  const listOnly = centerIds.filter((id) => data.nodes.some((node) => node.id === id)).length > 30;
  const currentView = listOnly ? "列表" : view;
  const displayData = useMemo(() => listOnly ? { nodes: [], edges: [] } : projection.data, [listOnly, projection.data]);
  const runtime = useCanvasRuntime({ container: viewport, data: displayData, centerIds, focusKey, loading, selectedNodeId, selectedEdgeId, onNodeClick, onEdgeClick, focusRequest });
  useEffect(() => setFocusRequest(undefined), [projection.key]);
  const locate = (id: string) => {
    if (listOnly) { onChooseCenter?.(id); return; }
    if (!projection.nodeIds.includes(id)) projection.expand(id);
    setView("图"); setFocusRequest({ id, sequence: ++focusSequence.current });
  };
  const callbacks = useRef({ onNodeClick, loading, expand: projection.expand });
  callbacks.current = { onNodeClick, loading, expand: projection.expand };
  useEffect(() => {
    if (!expanded) return;
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !document.querySelector(".ant-drawer-open, .ant-modal-wrap:not([style*='display: none']), .ant-select-dropdown:not(.ant-select-dropdown-hidden)")) setExpanded(false);
    };
    document.addEventListener("keydown", escape);
    return () => document.removeEventListener("keydown", escape);
  }, [expanded]);
  const graphData = useMemo(() => {
    const context = document.createElement("canvas").getContext("2d");
    if (context) context.font = `${RELATIONSHIP_FONT_SIZE}px ${RELATIONSHIP_FONT_FAMILY}`;
    const measure = (text: string) => context?.measureText(text).width ?? Array.from(text).length * RELATIONSHIP_FONT_SIZE;
    return {
      nodes: displayData.nodes.map((node) => ({ id: node.id, data: { name: node.name, typeName: entityTypeNames?.[node.entity_type_id] || "实体", center: centerIds.includes(node.id), remaining: projection.remaining[node.id] } })),
      edges: displayData.edges.map((edge) => ({ id: edge.id, source: edge.source_entity_id, target: edge.target_entity_id,
        data: { name: relationshipTypeNames[edge.relationship_type_id] || "关系",
          label: relationshipLabel(relationshipTypeNames[edge.relationship_type_id] || "关系", measure) } })),
    };
  }, [displayData, centerIds, entityTypeNames, relationshipTypeNames, projection.remaining]);
  const latestGraphData = useRef(graphData);
  latestGraphData.current = graphData;
  const options = useMemo<GraphOptions>(() => {
    const colors = { background: token.colorBgContainer, text: token.colorText, secondary: token.colorTextSecondary,
      border: token.colorBorder, primary: token.colorPrimary, selected: token.colorPrimaryBg };
    return {
      data: graphData, animation: false, autoResize: false, zoomRange: [0.1, 3], padding: 24,
      containerStyle: { width: "100%", height: "100%" },
      layout: { type: "dagre", rankdir: "LR", nodesep: 32, ranksep: 48, edgesep: 24, animation: false,
        edgeLabelSize: (edge: { id: string }) => {
          const label = graphData.edges.find((item) => item.id === edge.id)?.data.label;
          return label ? [label.width, label.height] : [0, 0];
        }, edgeLabelPos: "c", edgeLabelOffset: 0 },
      node: { style: {
        size: (node) => [NODE_WIDTH, nodeHeight(String(node.data?.name || node.id))],
        component: (node: NodeData) => <GraphNode name={String(node.data?.name || node.id)} typeName={String(node.data?.typeName || "实体")}
          center={Boolean(node.data?.center)} isSelected={node.states?.includes("selected")} isActive={node.states?.includes("active")} colors={colors}
          remaining={Number(node.data?.remaining ?? 0)} onActivate={() => { if (!callbacks.current.loading) callbacks.current.onNodeClick(String(node.id)); }}
          onExpand={() => { if (!callbacks.current.loading) callbacks.current.expand(String(node.id)); }} />,
      } },
      edge: { type: RELATIONSHIP_EDGE, style: { endArrow: true, radius: 12, stroke: token.colorTextQuaternary, strokeOpacity: 1, lineWidth: 1.5, increasedLineWidthForHitTesting: 10,
        label: true, labelText: (edge) => (edge.data?.label as ReturnType<typeof relationshipLabel>).text, labelBackground: true,
        labelWordWrap: false, labelMaxLines: Number.MAX_SAFE_INTEGER, labelTextOverflow: "clip", labelPadding: 6,
        labelAutoRotate: false, labelBackgroundFill: token.colorBgContainer, labelBackgroundOpacity: 1, labelBackgroundRadius: 4,
        labelFill: token.colorText, labelFontSize: RELATIONSHIP_FONT_SIZE, labelFontFamily: RELATIONSHIP_FONT_FAMILY, labelLineHeight: RELATIONSHIP_LINE_HEIGHT,
      }, state: { active: { stroke: token.colorPrimary, lineWidth: 2, label: true }, selected: { stroke: token.colorPrimary, lineWidth: 2, label: true } } },
      behaviors: (behaviors) => [...behaviors, { type: "hover-activate", degree: 1, animation: false }],
      onInit: coordinateGraphLifecycle,
      onReady: (graph) => { if (latestGraphData.current === graphData) runtime.onReady(graph); },
      onDestroy: runtime.onDestroy,
    };
  }, [graphData, token, runtime.onReady, runtime.onDestroy]);
  const failure = error || runtime.runtimeError;
  const scopeNotice = notice || (!loading && !failure && data.nodes.length > 0 && !data.edges.length
    ? "当前范围未找到关系，可在“调整范围”中修改关系类型或上下游跳数。" : undefined);
  return <div className={`graph-surface${expanded ? " graph-surface-expanded" : ""}`}
    style={{ background: token.colorBgContainer }} data-testid="graph-surface">
    <GraphToolbar data={data} centerIds={centerIds} shownNodes={displayData.nodes.length} shownEdges={displayData.edges.length} zoom={runtime.zoom} expanded={expanded}
      view={currentView} listOnly={listOnly} onView={setView} onMore={() => projection.expand()} onAutoFrame={runtime.autoFrame}
      onFocus={locate} onZoom={runtime.onZoom} onExpand={() => setExpanded((value) => !value)} />
    {failure ? <Alert type="error" showIcon message={data.nodes.length ? `更新失败，当前显示上一次结果${appliedSummary ? `：${appliedSummary}` : ""}` : "拓扑加载失败"} description={failure}
      action={onRetry ? <Button onClick={() => void onRetry()}>重试</Button> : undefined} /> : null}
    {scopeNotice || listOnly ? <Alert type="info" showIcon message={listOnly ? "中心超过 30 个，先从完整列表选择一个实体在图中观测。" : scopeNotice} /> : null}
    {currentView === "列表" ? <ResultList data={data} entityTypeNames={entityTypeNames ?? {}} relationshipTypeNames={relationshipTypeNames}
      shownIds={listOnly ? [] : projection.nodeIds} onNodeClick={onNodeClick} onEdgeClick={onEdgeClick} onLocate={locate} /> : null}
    <div ref={viewport} className="graph-viewport" style={{ display: currentView === "图" ? undefined : "none", background: token.colorFillQuaternary, border: `1px solid ${token.colorBorderSecondary}` }}>
      <div className="graph-renderer"><StableGraph {...options} /></div>
      {loading ? <div className="graph-message" style={{ background: token.colorBgMask, zIndex: 2 }}><Spin tip="正在更新观测范围"><div style={{ width: 160, height: 80 }} /></Spin></div>
        : !data.nodes.length ? <div className="graph-message"><Empty description={failure ? "请重试加载" : emptyMessage} /></div> : null}
    </div>
  </div>;
}
