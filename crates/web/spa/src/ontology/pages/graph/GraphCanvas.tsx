import { Alert, Button, Empty, Spin, theme } from "antd";
import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import type { GraphData } from "../../types";
import GraphToolbar from "./canvas/GraphToolbar";
import GraphScene, { type SceneHandle } from "./canvas3d/GraphScene";
import SceneBoundary from "./canvas3d/SceneBoundary";
import { useProjection } from "./projection/useProjection";
import ResultList from "./projection/ResultList";

const EMPTY_NAMES: Record<string, string> = {};
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


export default function GraphCanvas({ data, centerIds, entityTypeNames = EMPTY_NAMES, relationshipTypeNames, focusKey, loading = false,
  error, notice, appliedSummary, onChooseCenter, emptyMessage = "当前范围暂无关系数据", selectedNodeId, selectedEdgeId, onRetry, onNodeClick, onEdgeClick }: Props) {
  const { token } = theme.useToken();
  const scene = useRef<SceneHandle>(null);
  const focusSequence = useRef(0);
  const [expanded, setExpanded] = useState(false);
  const [view, setView] = useState("图");
  const [renderError, setRenderError] = useState("");
  const [attempt, setAttempt] = useState(0);
  const [focusRequest, setFocusRequest] = useState<{ id: string; sequence: number }>();
  const focusScope = JSON.stringify([focusKey, centerIds]);
  useEffect(() => setFocusRequest(undefined), [focusScope]);
  const projection = useProjection(data, centerIds, focusKey);
  const listOnly = centerIds.filter((id) => data.nodes.some((node) => node.id === id)).length > 30;
  const currentView = listOnly || renderError ? "列表" : view;
  const displayData = useMemo(() => listOnly ? { nodes: [], edges: [] } : projection.data, [listOnly, projection.data]);
  const fail = useCallback((message: string) => setRenderError(message), []);
  const retryScene = () => { setRenderError(""); setAttempt((value) => value + 1); setView("图"); };
  const locate = (id: string) => {
    if (listOnly) { onChooseCenter?.(id); return; }
    if (!projection.nodeIds.includes(id)) projection.expand(id);
    if (renderError) retryScene();
    setView("图"); setFocusRequest({ id, sequence: ++focusSequence.current });
  };
  useEffect(() => {
    if (!expanded) return;
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !document.querySelector(".ant-drawer-open, .ant-modal-wrap:not([style*='display: none']), .ant-select-dropdown:not(.ant-select-dropdown-hidden)")) setExpanded(false);
    };
    document.addEventListener("keydown", escape);
    return () => document.removeEventListener("keydown", escape);
  }, [expanded]);
  const scopeNotice = notice || (!loading && !error && data.nodes.length > 0 && !data.edges.length
    ? "当前范围未找到关系，可在“调整范围”中修改关系类型或上下游跳数。" : undefined);
  const style = { background: token.colorBgContainer, "--graph-background": token.colorBgContainer, "--graph-text": token.colorText,
    "--graph-secondary": token.colorTextSecondary, "--graph-border": token.colorBorder, "--graph-primary": token.colorPrimary,
    "--graph-selected": token.colorPrimaryBg } as CSSProperties;
  return <div className={`graph-surface${expanded ? " graph-surface-expanded" : ""}`} style={style} data-testid="graph-surface">
    <GraphToolbar data={data} centerIds={centerIds} shownNodes={displayData.nodes.length} shownEdges={displayData.edges.length} expanded={expanded}
      view={currentView} listOnly={listOnly || Boolean(renderError)} onView={setView} onMore={() => projection.expand()} onAutoFrame={() => scene.current?.autoFrame()}
      onFocus={locate} onZoom={(action) => scene.current?.zoom(action)} onExpand={() => setExpanded((value) => !value)} />
    {error ? <Alert type="error" showIcon title={data.nodes.length ? `更新失败，当前显示上一次结果${appliedSummary ? `：${appliedSummary}` : ""}` : "拓扑加载失败"} description={error}
      action={onRetry ? <Button onClick={() => void onRetry()}>重试</Button> : undefined} /> : null}
    {renderError ? <Alert type="warning" showIcon title="三维画布暂不可用，已显示完整列表" description={renderError}
      action={<Button onClick={retryScene}>重试三维画布</Button>} /> : null}
    {scopeNotice || listOnly ? <Alert type="info" showIcon title={listOnly ? "中心超过 30 个，先从完整列表选择一个实体在图中观测。" : scopeNotice} /> : null}
    {currentView === "列表" ? <ResultList data={data} entityTypeNames={entityTypeNames} relationshipTypeNames={relationshipTypeNames}
      shownIds={listOnly || renderError ? [] : projection.nodeIds} onNodeClick={onNodeClick} onEdgeClick={onEdgeClick} onLocate={locate} /> : null}
    <div className="graph-viewport" style={{ display: currentView === "图" ? undefined : "none", background: token.colorFillQuaternary, border: `1px solid ${token.colorBorderSecondary}` }}>
      {displayData.nodes.length && !renderError ? <SceneBoundary key={attempt} onError={fail}>
        <GraphScene ref={scene} data={displayData} centerIds={centerIds} focusKey={focusKey} focusRequest={focusRequest}
          visible={currentView === "图"} loading={loading} remaining={projection.remaining} entityTypeNames={entityTypeNames}
          relationshipTypeNames={relationshipTypeNames} selectedNodeId={selectedNodeId} selectedEdgeId={selectedEdgeId}
          background={token.colorBgContainer} edgeColor={token.colorTextSecondary} primary={token.colorPrimary}
          onNodeClick={onNodeClick} onEdgeClick={onEdgeClick} onExpand={projection.expand} onError={fail} />
      </SceneBoundary> : null}
      {loading ? <div className="graph-message" style={{ background: token.colorBgMask, zIndex: 2200 }}><Spin description="正在更新观测范围"><div style={{ width: 160, height: 80 }} /></Spin></div>
        : !data.nodes.length ? <div className="graph-message"><Empty description={error ? "请重试加载" : emptyMessage} /></div> : null}
    </div>
  </div>;
}
