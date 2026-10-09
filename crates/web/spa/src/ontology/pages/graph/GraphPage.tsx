import { ReloadOutlined } from "@ant-design/icons";
import { Section } from "../../ui";
import { Alert, Button, Space, Tabs, Tooltip } from "antd";
import { useEffect, useMemo, useRef, useState } from "react";
import { useEnv } from "../../env";
import type { GraphAspect } from "../../types";
import AspectObservePanel from "./aspects/AspectObservePanel";
import SaveAspectModal from "./aspects/SaveAspectModal";
import { useAspectObservation } from "./aspects/useAspectObservation";
import { useGraphAspects } from "./aspects/useGraphAspects";
import { useViewportHeight } from "./canvas/useViewportHeight";
import EntityDrawer from "./EntityDrawer";
import GraphCanvas from "./GraphCanvas";
import ObservationFilters from "./ObservationFilters";
import RelationshipDrawer from "./RelationshipDrawer";
import { centersDisconnected } from "./observationSelection";
import { useGraphObservation } from "./useGraphObservation";
import "./workspace.css";
import { useDraftGuard } from "../../navigation/DraftGuard";
import { useDetailNavigation } from "./details/useDetailNavigation";
import EntityRelations from "./details/EntityRelations";
import { aspectSelection, readObservation, recommendedAspect, validSelection, writeObservation } from "./session/observationMemory";

const TEST = "aspect-test";
const OBSERVE = "aspect-observe";
export default function GraphPage() {
  const { env, canManage } = useEnv();
  return <GraphWorkspace key={env} env={env} canManage={canManage} />;
}

function GraphWorkspace({ env, canManage }: { env: string; canManage: boolean }) {
  const expandNeighbors = true;
  const guard = useDraftGuard();
  const detail = useDetailNavigation();
  const restored = useRef(false);
  const [restoreNotice, setRestoreNotice] = useState("");
  const [activeTab, setActiveTab] = useState(TEST);
  const observing = activeTab === OBSERVE;
  const { entities, entityTypes, relationships, relationshipTypes, selection, appliedSelection, metadataReady, data, loading, error, metadataError, refresh, changeSelection } = useGraphObservation(env, expandNeighbors, !observing);
  const aspectStore = useGraphAspects(env);
  const selectedEntityId = detail.current?.kind === "entity" ? detail.current.id : undefined;
  const selectedRelationshipId = detail.current?.kind === "relationship" ? detail.current.id : undefined;
  const [saveModalOpen, setSaveModalOpen] = useState(false);
  const [aspect, setAspect] = useState<GraphAspect>();
  const [aspectRelationshipTypeIds, setAspectRelationshipTypeIds] = useState<string[]>([]);
  const [aspectCenterIds, setAspectCenterIds] = useState<string[]>([]);
  const [aspectUpstreamDepth, setAspectUpstreamDepth] = useState(3);
  const [aspectDownstreamDepth, setAspectDownstreamDepth] = useState(3);
  const aspectObservation = useAspectObservation({ facet: aspect, active: observing, relationshipTypeIds: aspectRelationshipTypeIds,
    centerIds: aspectCenterIds, upstreamDepth: aspectUpstreamDepth, downstreamDepth: aspectDownstreamDepth, expandNeighbors });
  const workspace = useViewportHeight();
  const visibleData = observing ? aspectObservation.data : data;
  const selectedEntity = visibleData.nodes.find((item) => item.id === selectedEntityId) || entities.find((item) => item.id === selectedEntityId);
  const selectedRelationship = relationships.find((item) => item.id === selectedRelationshipId) || visibleData.edges.find((item) => item.id === selectedRelationshipId);
  const typeNames = useMemo(() => Object.fromEntries(relationshipTypes.map((item) => [item.id, item.name])), [relationshipTypes]);
  const entityTypeNames = useMemo(() => Object.fromEntries(entityTypes.map((item) => [item.id, item.name])), [entityTypes]);
  const clearDetail = detail.close;
  const applyAspect = (next?: GraphAspect) => {
    clearDetail(); setAspect(next);
    setAspectRelationshipTypeIds(next?.relationship_type_ids ?? []);
    setAspectCenterIds(next?.default_center_ids ?? []);
    setAspectUpstreamDepth(next?.default_upstream_depth ?? 3);
    setAspectDownstreamDepth(next?.default_downstream_depth ?? 3);
  };
  const handleAspectChange = (next?: GraphAspect) => guard.run(() => applyAspect(next));
  useEffect(() => {
    if (restored.current || !metadataReady || aspectStore.loading || aspectStore.error) return;
    restored.current = true;
    const memory = readObservation(env);
    const savedAspect = aspectStore.aspects.find((item) => item.aspect_key === memory?.aspectKey);
    if (memory && validSelection(memory.selection, { entities, entityTypes, relationshipTypes })
      && (memory.mode === TEST || (savedAspect && savedAspect.entity_type_ids.length === memory.selection.entityTypeIds.length
        && savedAspect.entity_type_ids.every((id) => memory.selection.entityTypeIds.includes(id))))) {
      setActiveTab(memory.mode);
      if (memory.mode === TEST) changeSelection(memory.selection);
      else {
        setAspect(savedAspect); setAspectCenterIds(memory.selection.centerIds); setAspectRelationshipTypeIds(memory.selection.relationshipTypeIds);
        setAspectUpstreamDepth(memory.selection.upstreamDepth); setAspectDownstreamDepth(memory.selection.downstreamDepth);
      }
    } else if (aspectStore.aspects.length) {
      setActiveTab(OBSERVE);
      applyAspect(recommendedAspect(aspectStore.aspects));
      if (memory) setRestoreNotice("上次观测条件已失效，已恢复推荐切面。");
    }
  }, [metadataReady, aspectStore.loading, aspectStore.aspects, aspectStore.error]);
  useEffect(() => {
    if (aspect && !aspectStore.loading) {
      const latest = aspectStore.aspects.find((item) => item.id === aspect.id);
      if (!latest) guard.run(() => { applyAspect(recommendedAspect(aspectStore.aspects)); setRestoreNotice("原切面已停用，已切换到可用切面。"); });
      else if (latest.revision !== aspect.revision) setAspect(latest);
    }
  }, [aspect, aspectStore.aspects, aspectStore.loading]);
  useEffect(() => {
    const applied = aspectObservation.applied;
    if (restored.current && applied) writeObservation(env, { mode: OBSERVE, aspectKey: applied.aspectKey, selection: applied.selection });
  }, [aspectObservation.applied, env]);
  useEffect(() => {
    if (restored.current && appliedSelection) writeObservation(env, { mode: TEST, selection: appliedSelection });
  }, [appliedSelection, env]);
  const handleNodeClick = (id: string) => {
    if (observing ? aspectObservation.loading : loading) return;
    guard.run(() => detail.openEntity(id));
  };
  const handleEdgeClick = (id: string) => {
    if (observing ? aspectObservation.loading : loading) return;
    guard.run(() => detail.openRelationship(id));
  };
  const currentSelection = observing && aspect ? { ...aspectSelection(aspect), relationshipTypeIds: aspectRelationshipTypeIds,
    centerIds: aspectCenterIds, upstreamDepth: aspectUpstreamDepth, downstreamDepth: aspectDownstreamDepth } : selection;
  const observeEntity = (id: string) => {
    const entity = entities.find((item) => item.id === id);
    if (!entity) return;
    clearDetail(); setActiveTab(TEST);
    changeSelection({ ...currentSelection, centerIds: [id], entityTypeIds: [entity.entity_type_id] });
  };
  const applied = observing ? aspectObservation.applied?.selection : appliedSelection;
  const appliedSummary = applied ? `${observing ? aspectObservation.applied?.name ?? "" : "自定义观测"} · ${applied.centerIds.map((id) => entities.find((item) => item.id === id)?.name ?? id).join("、") || "整个切面"} · 上游 ${applied.upstreamDepth} / 下游 ${applied.downstreamDepth} 跳 · ${applied.relationshipTypeIds.map((id) => typeNames[id] ?? id).join("、") || "全部关系"}` : undefined;
  const handleWorkspaceChanged = async () => {
    await Promise.all([refresh(), aspectStore.reload(), ...(observing ? [aspectObservation.reload()] : [])]);
  };
  const directoryIds = new Set(relationshipTypes.filter((item) => item.is_directory_membership).map((item) => item.id));
  const disconnected = expandNeighbors && !loading && !error && centersDisconnected(data, selection.centerIds, directoryIds);
  const emptyMessage = !selection.entityTypeIds.length ? (expandNeighbors ? "请先选择实体类型" : "请先选择实体类型，再选择关系类型")
    : !expandNeighbors && !selection.relationshipTypeIds.length ? "请选择关系类型，再选择观测实体"
      : !selection.centerIds.length ? "请选择一个或多个实体，再设置上下游观测跳数"
        : expandNeighbors && !data.edges.length ? (data.available_relationship_type_ids.length ? "当前关系筛选没有匹配关系" : "当前范围暂无已确认关系") : "当前范围暂无关系数据";
  const scopeReady = currentSelection.entityTypeIds.length > 0 && (expandNeighbors || currentSelection.relationshipTypeIds.length > 0);
  const toolbar = <Space>
    {canManage ? <Tooltip title={!scopeReady ? "请先选择观测范围" : undefined}><Button disabled={!scopeReady} onClick={() => setSaveModalOpen(true)}>{observing ? "另存切面" : "保存切面"}</Button></Tooltip> : null}
    <Tooltip title="刷新当前观测"><Button aria-label="刷新当前观测" icon={<ReloadOutlined />} loading={loading || aspectStore.loading || (observing && aspectObservation.loading)}
      onClick={() => void handleWorkspaceChanged()} /></Tooltip>
  </Space>;
  return <>
    <div ref={workspace.ref} className="graph-workspace" style={{ height: workspace.height }}>
      <Section>
        {restoreNotice ? <Alert type="info" showIcon closable onClose={() => setRestoreNotice("")} message={restoreNotice} /> : null}
        {metadataError ? <Alert type="error" message="观测范围加载失败" description={metadataError} showIcon action={<Button onClick={() => void refresh()}>重试</Button>} /> : null}
        <Tabs activeKey={activeTab} onChange={(key) => guard.run(() => { clearDetail(); setActiveTab(key); })} tabBarExtraContent={toolbar} items={[
          { key: OBSERVE, label: "切面观测", children: <AspectObservePanel
            entityTypes={entityTypes} entities={entities} relationshipTypes={relationshipTypes} aspects={aspectStore.aspects}
            aspectsLoading={aspectStore.loading} aspectsError={aspectStore.error} aspect={aspect}
            relationshipTypeIds={aspectRelationshipTypeIds} centerIds={aspectCenterIds} upstreamDepth={aspectUpstreamDepth} downstreamDepth={aspectDownstreamDepth}
            data={aspectObservation.data} appliedSummary={appliedSummary} loading={aspectObservation.loading} error={aspectObservation.error}
            selectedNodeId={selectedEntityId} selectedEdgeId={selectedRelationshipId}
            onAspectChange={handleAspectChange} onRelationshipTypeIdsChange={setAspectRelationshipTypeIds} onCenterIdsChange={setAspectCenterIds}
            onUpstreamDepthChange={setAspectUpstreamDepth} onDownstreamDepthChange={setAspectDownstreamDepth}
            onNodeClick={handleNodeClick} onEdgeClick={handleEdgeClick} onReload={aspectObservation.reload} onReloadAspects={aspectStore.reload} /> },
          { key: TEST, label: "自定义观测", children: <>
            <ObservationFilters entityTypes={entityTypes} entities={entities} relationshipTypes={relationshipTypes} relationships={relationships}
              expandNeighbors={expandNeighbors} selection={selection} onChange={changeSelection} />
            <GraphCanvas data={data} centerIds={selection.centerIds} entityTypeNames={entityTypeNames} relationshipTypeNames={typeNames}
              appliedSummary={appliedSummary} onChooseCenter={observeEntity} loading={loading} error={metadataError ? undefined : error} emptyMessage={emptyMessage} onRetry={refresh}
              notice={disconnected ? "所选实体在当前关系筛选和跳数内没有已确认路径；可调整关系类型或上下游跳数。" : undefined}
              selectedNodeId={selectedEntityId} selectedEdgeId={selectedRelationshipId} onNodeClick={handleNodeClick} onEdgeClick={handleEdgeClick} />
          </> },
        ]} />
      </Section>
    </div>
    <SaveAspectModal entityTypes={entityTypes} relationshipTypes={relationshipTypes} currentAspect={observing ? aspect : undefined} entityTypeIds={currentSelection.entityTypeIds}
      relationshipTypeIds={currentSelection.relationshipTypeIds} centerIds={currentSelection.centerIds} upstreamDepth={currentSelection.upstreamDepth}
      downstreamDepth={currentSelection.downstreamDepth} aspects={aspectStore.aspects} open={saveModalOpen} expandNeighbors={expandNeighbors}
      onClose={() => setSaveModalOpen(false)} onCreate={aspectStore.create} onUpdate={aspectStore.update} onDelete={aspectStore.remove} />
    <EntityDrawer env={env} entity={selectedEntity} entityTypes={entityTypes} canManage={canManage} onClose={clearDetail} onEntityChanged={handleWorkspaceChanged}
      onBack={detail.canBack ? detail.back : undefined} onObserve={selectedEntity ? () => observeEntity(selectedEntity.id) : undefined}
      view={detail.view} onView={detail.remember}
      relations={selectedEntity ? <EntityRelations key={selectedEntity.id} entityId={selectedEntity.id} entities={entities} relationships={relationships}
        relationshipTypes={relationshipTypes} observedIds={visibleData.edges.map((edge) => edge.id)}
        onEntity={(id) => guard.run(() => detail.openEntity(id))} onRelationship={(id) => guard.run(() => detail.openRelationship(id))} /> : undefined} />
    <RelationshipDrawer env={env} relationship={selectedRelationship} entities={entities} relationshipTypes={relationshipTypes} canManage={canManage}
      onClose={clearDetail} onChanged={handleWorkspaceChanged} onEntity={detail.openEntity} onBack={detail.canBack ? detail.back : undefined} />
  </>;
}
