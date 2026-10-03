import { Section } from "../../ui";
import { Alert, Button, Checkbox, Empty, Spin, Tabs, Tooltip, Typography } from "antd";
import { useEffect, useMemo, useState } from "react";
import { useEnv } from "../../env";
import type { GraphAspect } from "../../types";
import AspectObservePanel from "./aspects/AspectObservePanel";
import SaveAspectModal from "./aspects/SaveAspectModal";
import { useAspectObservation } from "./aspects/useAspectObservation";
import { useGraphAspects } from "./aspects/useGraphAspects";
import EntityDrawer from "./EntityDrawer";
import GraphCanvas from "./GraphCanvas";
import ObservationFilters from "./ObservationFilters";
import RelationshipDrawer from "./RelationshipDrawer";
import { centersDisconnected } from "./observationSelection";
import { useGraphObservation } from "./useGraphObservation";

const ASPECT_TEST_TAB = "aspect-test";
const ASPECT_OBSERVE_TAB = "aspect-observe";

export default function GraphPage() {
  const { env, canManage } = useEnv();
  return <GraphWorkspace key={env} env={env} canManage={canManage} />;
}

function GraphWorkspace({ env, canManage }: { env: string; canManage: boolean }) {
  const [expandNeighbors, setExpandNeighbors] = useState(false);
  const { entities, entityTypes, relationships, relationshipTypes, selection, data, loading, error, refresh, changeSelection } = useGraphObservation(env, expandNeighbors);
  const aspectStore = useGraphAspects(env);
  const [selectedEntityId, setSelectedEntityId] = useState<string>();
  const [selectedRelationshipId, setSelectedRelationshipId] = useState<string>();
  const [activeTab, setActiveTab] = useState(ASPECT_TEST_TAB);
  const [saveModalOpen, setSaveModalOpen] = useState(false);
  const [aspect, setAspect] = useState<GraphAspect>();
  const [aspectRelationshipTypeIds, setAspectRelationshipTypeIds] = useState<string[]>([]);
  const [aspectCenterIds, setAspectCenterIds] = useState<string[]>([]);
  const [aspectUpstreamDepth, setAspectUpstreamDepth] = useState<number>(3);
  const [aspectDownstreamDepth, setAspectDownstreamDepth] = useState<number>(3);
  const aspectObservation = useAspectObservation({ facet: aspect, relationshipTypeIds: aspectRelationshipTypeIds,
    centerIds: aspectCenterIds, upstreamDepth: aspectUpstreamDepth, downstreamDepth: aspectDownstreamDepth, expandNeighbors });
  const selectedEntity = entities.find((item) => item.id === selectedEntityId);
  const selectedRelationship = relationships.find((item) => item.id === selectedRelationshipId);
  const typeNames = useMemo(() => Object.fromEntries(relationshipTypes.map((item) => [item.id, item.name])), [relationshipTypes]);

  // 切面被删除后不再保留失效的观测选择
  useEffect(() => {
    if (aspect && !aspectStore.aspects.some((item) => item.id === aspect.id)) {
      setAspect(undefined);
      setAspectRelationshipTypeIds([]);
      setAspectCenterIds([]);
      setAspectUpstreamDepth(3);
      setAspectDownstreamDepth(3);
    }
  }, [aspect, aspectStore.aspects]);

  const handleNodeClick = (nodeId: string) => {
    if (loading || aspectObservation.loading) return;
    setSelectedRelationshipId(undefined);
    setSelectedEntityId(nodeId);
  };
  const handleEdgeClick = (edgeId: string) => {
    if (loading || aspectObservation.loading) return;
    setSelectedEntityId(undefined);
    setSelectedRelationshipId(edgeId);
  };
  const handleTabChange = (key: string) => {
    setSelectedEntityId(undefined);
    setSelectedRelationshipId(undefined);
    setActiveTab(key);
  };
  const handleAspectChange = (next?: GraphAspect) => {
    setAspect(next);
    setAspectRelationshipTypeIds(next?.relationship_type_ids ?? []);
    setAspectCenterIds(next?.default_center_ids ?? []);
    setAspectUpstreamDepth(next?.default_upstream_depth ?? 3);
    setAspectDownstreamDepth(next?.default_downstream_depth ?? 3);
  };
  const handleWorkspaceChanged = async () => {
    await refresh();
    await aspectStore.reload();
    await aspectObservation.reload();
  };

  const graph = !loading && data.nodes.length ? <GraphCanvas
    data={data} centerIds={selection.centerIds} relationshipTypeNames={typeNames} onNodeClick={handleNodeClick} onEdgeClick={handleEdgeClick}
  /> : null;
  const directoryTypeIds = new Set(relationshipTypes.filter((item) => item.is_directory_membership).map((item) => item.id));
  const disconnected = expandNeighbors && !loading && !error && centersDisconnected(data, selection.centerIds, directoryTypeIds);
  const observation = <Spin spinning={loading}><div style={{ minHeight: loading ? 220 : undefined }}>
    {disconnected ? <Alert type="info" showIcon style={{ marginBottom: 12 }}
      title="所选实体在当前关系筛选和跳数内没有已确认路径；可调整关系类型或上下游跳数。" /> : null}
    {!loading && !error && (!selection.entityTypeIds.length
      ? <Empty description={expandNeighbors ? "请先选择实体类型" : "请先选择实体类型，再选择关系类型"} />
      : !expandNeighbors && !selection.relationshipTypeIds.length
        ? <Empty description="请选择关系类型，再选择观测实体" />
        : !selection.centerIds.length
          ? <Empty description="请选择一个或多个实体，再设置上下游观测跳数" />
          : expandNeighbors && !data.edges.length
            ? <Empty description={data.available_relationship_type_ids.length ? "当前关系筛选没有匹配关系" : "当前范围暂无已确认关系"} />
            : graph || <Empty description="当前范围暂无关系数据" />)}
  </div></Spin>;

  const scopeReady = selection.entityTypeIds.length > 0 && (expandNeighbors || selection.relationshipTypeIds.length > 0);
  const aspectTestTab = <>
    <ObservationFilters entityTypes={entityTypes} entities={entities} relationshipTypes={relationshipTypes} relationships={relationships}
      expandNeighbors={expandNeighbors}
      selection={selection}
      onChange={changeSelection} />
    <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 16, marginBottom: 16 }}>
      <Typography.Text type="secondary">{expandNeighbors ? "切面固定起点实体类型，可选关系类型" : "切面固定实体类型与关系类型组合，供切面观测复用"}</Typography.Text>
      {canManage ? (scopeReady ? <Button type="primary" onClick={() => setSaveModalOpen(true)}>保存切面</Button>
        : <Tooltip title={expandNeighbors ? "请先选择实体类型" : "请先选择实体类型与关系类型"}><span><Button type="primary" disabled>保存切面</Button></span></Tooltip>) : null}
    </div>
    {observation}
    <Typography.Text type="secondary">{expandNeighbors ? "选择实体类型和观测实体，自动展开真实关联；关系类型可选。" : "先选择实体类型与关系类型，再选择观测实体并设置上下游跳数。"}</Typography.Text>
    <SaveAspectModal entityTypes={entityTypes} relationshipTypes={relationshipTypes} entityTypeIds={selection.entityTypeIds}
      relationshipTypeIds={selection.relationshipTypeIds} centerIds={selection.centerIds} upstreamDepth={selection.upstreamDepth}
      downstreamDepth={selection.downstreamDepth} aspects={aspectStore.aspects} open={saveModalOpen} expandNeighbors={expandNeighbors}
      onClose={() => setSaveModalOpen(false)} onCreate={aspectStore.create} onUpdate={aspectStore.update} onDelete={aspectStore.remove} />
  </>;
  const aspectObserveTab = <AspectObservePanel entityTypes={entityTypes} entities={entities} relationshipTypes={relationshipTypes}
    aspects={aspectStore.aspects} aspectsLoading={aspectStore.loading} aspectsError={aspectStore.error} aspect={aspect}
    relationshipTypeIds={aspectRelationshipTypeIds} centerIds={aspectCenterIds}
    upstreamDepth={aspectUpstreamDepth} downstreamDepth={aspectDownstreamDepth}
    data={aspectObservation.data} loading={aspectObservation.loading} error={aspectObservation.error}
    onAspectChange={handleAspectChange} onRelationshipTypeIdsChange={setAspectRelationshipTypeIds}
    onCenterIdsChange={setAspectCenterIds} onUpstreamDepthChange={setAspectUpstreamDepth}
    onDownstreamDepthChange={setAspectDownstreamDepth} onNodeClick={handleNodeClick} onEdgeClick={handleEdgeClick}
    onReload={aspectObservation.reload} onReloadAspects={aspectStore.reload} />;

  return <>
    <Section>
      {error ? <Alert type="error" showIcon title="拓扑加载失败" description={error} style={{ marginBottom: 16 }}
        action={<Button onClick={() => void refresh()}>重试</Button>} /> : null}
      <Checkbox checked={expandNeighbors} onChange={(event) => setExpandNeighbors(event.target.checked)} style={{ marginBottom: 16 }}>展开跨类型邻居</Checkbox>
      <Tabs activeKey={activeTab} onChange={handleTabChange} items={[
        { key: ASPECT_TEST_TAB, label: "切面测试", children: aspectTestTab },
        { key: ASPECT_OBSERVE_TAB, label: "切面观测", children: aspectObserveTab },
      ]} />
    </Section>
    <EntityDrawer env={env} entity={selectedEntity} entityTypes={entityTypes} canManage={canManage} onClose={() => setSelectedEntityId(undefined)} onEntityChanged={handleWorkspaceChanged} />
    <RelationshipDrawer env={env} relationship={selectedRelationship} entities={entities} relationshipTypes={relationshipTypes} canManage={canManage} onClose={() => setSelectedRelationshipId(undefined)} onChanged={handleWorkspaceChanged} />
  </>;
}
