import { InfoCircleOutlined } from "@ant-design/icons";
import { Alert, Button, Form, Select, Tag, Tooltip } from "antd";
import { useMemo } from "react";
import type { Entity, EntityType, GraphAspect, GraphResponse, RelationshipType } from "../../../types";
import GraphCanvas from "../GraphCanvas";
import { entitiesOfTypes, relationshipTypeLabel } from "../observationSelection";
import BatchMultiSelect from "../BatchMultiSelect";
import FilterSection from "../controls/FilterSection";
import HopSelect from "../controls/HopSelect";

type Props = {
  entityTypes: EntityType[];
  entities: Entity[];
  relationshipTypes: RelationshipType[];
  aspects: GraphAspect[];
  aspectsLoading: boolean;
  aspectsError: string;
  aspect?: GraphAspect;
  relationshipTypeIds: string[];
  centerIds: string[];
  upstreamDepth: number;
  downstreamDepth: number;
  data: GraphResponse;
  selectedNodeId?: string;
  selectedEdgeId?: string;
  loading: boolean;
  error: string;
  appliedSummary?: string;
  onAspectChange: (aspect?: GraphAspect) => void;
  onRelationshipTypeIdsChange: (ids: string[]) => void;
  onCenterIdsChange: (centerIds: string[]) => void;
  onUpstreamDepthChange: (depth: number) => void;
  onDownstreamDepthChange: (depth: number) => void;
  onNodeClick: (id: string) => void;
  onEdgeClick: (id: string) => void;
  onReload: () => Promise<void>;
  onReloadAspects: () => Promise<void>;
};

export default function AspectObservePanel({
  entityTypes, entities, relationshipTypes, aspects, aspectsLoading, aspectsError, aspect, relationshipTypeIds,
  centerIds, selectedNodeId, selectedEdgeId, upstreamDepth, downstreamDepth, data, loading, error, appliedSummary, onAspectChange, onRelationshipTypeIdsChange, onCenterIdsChange,
  onUpstreamDepthChange, onDownstreamDepthChange, onNodeClick, onEdgeClick, onReload, onReloadAspects,
}: Props) {
  const typeNames = useMemo(() => Object.fromEntries(entityTypes.map((item) => [item.id, item.name])), [entityTypes]);
  const relationshipTypeNames = useMemo(() => Object.fromEntries(relationshipTypes.map((item) => [item.id, item.name])), [relationshipTypes]);
  const centerOptions = useMemo(() => (aspect ? entitiesOfTypes(entities, aspect.entity_type_ids) : [])
    .map((item) => ({ value: item.id, label: item.name, description: typeNames[item.entity_type_id] })), [aspect, entities, typeNames]);
  const relationshipOptions = useMemo(() => {
    const candidates = new Set([...data.available_relationship_type_ids, ...relationshipTypeIds, ...(aspect?.relationship_type_ids ?? [])]);
    return relationshipTypes.filter((item) => !item.is_deleted && !item.is_directory_membership)
      .sort((a, b) => Number(candidates.has(b.id)) - Number(candidates.has(a.id)))
      .map((item) => ({ value: item.id, label: item.name, description: relationshipTypeLabel(item, typeNames) }));
  }, [aspect, data.available_relationship_type_ids, relationshipTypeIds, relationshipTypes, typeNames]);

  const sameIds = (left: string[], right: string[]) => left.length === right.length && left.every((id) => right.includes(id));
  const adjusted = Boolean(aspect && (!sameIds(centerIds, aspect.default_center_ids) || !sameIds(relationshipTypeIds, aspect.relationship_type_ids)
    || upstreamDepth !== (aspect.default_upstream_depth ?? 3) || downstreamDepth !== (aspect.default_downstream_depth ?? 3)));
  const graphCenters = useMemo(() => centerIds.length ? centerIds : data.nodes.filter((node) => aspect?.entity_type_ids.includes(node.entity_type_id)).map((node) => node.id), [centerIds, data, aspect]);
  return <>
    <div className="graph-aspect-header">
      <Select aria-label="切面选择" allowClear showSearch optionFilterProp="label" placeholder="请选择切面"
        value={aspect?.id} loading={aspectsLoading}
        onChange={(id) => onAspectChange(aspects.find((item) => item.id === id))}
        options={aspects.map((item) => ({ value: item.id, label: item.name }))} />
      <Tooltip title={aspect?.description}>
        <Button type="text" aria-label="切面说明" icon={<InfoCircleOutlined />} disabled={!aspect} />
      </Tooltip>
      {adjusted ? <><Tag color="blue">已调整</Tag><Button onClick={() => onAspectChange(aspects.find((item) => item.id === aspect?.id))}>恢复推荐</Button></> : null}
    <FilterSection summary={`观测范围 · ${centerIds.length === 1 ? `中心：${entities.find((item) => item.id === centerIds[0])?.name ?? "加载中"}` : centerIds.length ? `${centerIds.length} 个中心` : "整个切面"} · ${relationshipTypeIds.length ? `${relationshipTypeIds.length} 种关系` : "全部关系"} · 上游 ${upstreamDepth} / 下游 ${downstreamDepth} 跳`}>
      <Form layout="vertical"><div className="graph-filter-grid">
        <Form.Item label="观测实体（可选）"><BatchMultiSelect label="切面观测实体多选" placeholder="留空观察整个切面范围"
          value={centerIds} disabled={!aspect} onChange={onCenterIdsChange} options={centerOptions} /></Form.Item>
        <Form.Item label="关系类型（可调整）"><BatchMultiSelect label="切面关系类型多选" placeholder="留空显示全部关系"
          value={relationshipTypeIds} disabled={!aspect} onChange={onRelationshipTypeIdsChange} options={relationshipOptions} /></Form.Item>
        <HopSelect labelPrefix="切面" upstream={upstreamDepth} downstream={downstreamDepth} maximum={9} disabled={!aspect}
          onUpstream={onUpstreamDepthChange} onDownstream={onDownstreamDepthChange} />
      </div></Form>
    </FilterSection>
    </div>
    {aspectsError ? <Alert type="error" showIcon message="切面加载失败" description={aspectsError}
      action={<Button onClick={() => void onReloadAspects()}>重试</Button>} /> : null}
    <GraphCanvas data={data} centerIds={graphCenters} entityTypeNames={typeNames} relationshipTypeNames={relationshipTypeNames}
      appliedSummary={appliedSummary} onChooseCenter={(id) => onCenterIdsChange([id])}
      selectedNodeId={selectedNodeId} selectedEdgeId={selectedEdgeId} focusKey={aspect?.id} loading={loading} error={error} onRetry={onReload}
      emptyMessage={!aspect ? "请选择数据切面" : "当前范围暂无关系数据"}
      onNodeClick={onNodeClick} onEdgeClick={onEdgeClick} />
  </>;
}
