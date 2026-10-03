import { Alert, Button, Empty, Form, Select, Spin, Typography } from "antd";
import { useMemo } from "react";
import type { CSSProperties } from "react";
import type { Entity, EntityType, GraphAspect, GraphResponse, RelationshipType } from "../../../types";
import GraphCanvas from "../GraphCanvas";
import { entitiesOfTypes, relationshipTypeLabel } from "../observationSelection";
import BatchMultiSelect from "../BatchMultiSelect";

const HOPS = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9].map((value) => ({ value, label: `${value} 跳` }));
const itemStyle = { margin: 0, minWidth: 0, maxWidth: "100%" };
const rowStyle: CSSProperties = { display: "flex", flexWrap: "wrap", columnGap: 16, rowGap: 12, width: "100%", alignItems: "flex-start" };

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
  loading: boolean;
  error: string;
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

/** 切面观测：保存的关系类型是初始筛选，用户可以在当前观测中调整。 */
export default function AspectObservePanel({
  entityTypes, entities, relationshipTypes, aspects, aspectsLoading, aspectsError, aspect, relationshipTypeIds,
  centerIds, upstreamDepth, downstreamDepth, data, loading, error, onAspectChange, onRelationshipTypeIdsChange, onCenterIdsChange,
  onUpstreamDepthChange, onDownstreamDepthChange, onNodeClick, onEdgeClick, onReload, onReloadAspects,
}: Props) {
  const typeNames = useMemo(() => Object.fromEntries(entityTypes.map((item) => [item.id, item.name])), [entityTypes]);
  const relationshipTypeNames = useMemo(() => Object.fromEntries(relationshipTypes.map((item) => [item.id, item.name])), [relationshipTypes]);
  const centerOptions = useMemo(
    () => (aspect ? entitiesOfTypes(entities, aspect.entity_type_ids) : []).map((item) => ({ value: item.id, label: item.name })),
    [aspect, entities],
  );
  const relationshipOptions = useMemo(() => {
    const candidates = new Set([...data.available_relationship_type_ids, ...relationshipTypeIds, ...(aspect?.relationship_type_ids ?? [])]);
    return relationshipTypes.filter((item) => !item.is_deleted && !item.is_directory_membership && candidates.has(item.id))
      .map((item) => ({ value: item.id, label: relationshipTypeLabel(item, typeNames) }));
  }, [aspect, data.available_relationship_type_ids, relationshipTypeIds, relationshipTypes, typeNames]);
  const graph = !loading && data.nodes.length
    ? <GraphCanvas data={data} centerIds={centerIds} relationshipTypeNames={relationshipTypeNames} onNodeClick={onNodeClick} onEdgeClick={onEdgeClick} />
    : null;
  const observation = <Spin spinning={loading}><div style={{ minHeight: loading ? 220 : undefined }}>
    {aspectsError ? <Alert type="error" showIcon title="切面加载失败" description={aspectsError} style={{ marginBottom: 16 }}
      action={<Button onClick={() => void onReloadAspects()}>重试</Button>} /> : null}
    {error ? <Alert type="error" showIcon title="拓扑加载失败" description={error} style={{ marginBottom: 16 }}
      action={<Button onClick={() => void onReload()}>重试</Button>} /> : null}
    {!loading && !error && (!aspect
      ? <Empty description="请选择数据切面" />
      : graph || <Empty description="当前范围暂无关系数据" />)}
  </div></Spin>;

  return <><Form layout="inline" style={{ marginBottom: 16, display: "flex", flexWrap: "wrap", columnGap: 16, rowGap: 12 }}>
    <div style={rowStyle}>
      <Form.Item label="切面" required labelCol={{ flex: "none" }} wrapperCol={{ flex: "1 1 0" }} style={{ ...itemStyle, flex: "1 1 260px" }}>
        <Select aria-label="切面选择" allowClear showSearch optionFilterProp="label"
          placeholder="请选择切面" value={aspect?.id} style={{ width: "100%" }} loading={aspectsLoading}
          onChange={(aspectId) => onAspectChange(aspects.find((item) => item.id === aspectId))}
          options={aspects.map((item) => ({ value: item.id, label: item.name }))} />
      </Form.Item>
      {aspect ?
        <Form.Item label="实体类型" labelCol={{ flex: "none" }} style={itemStyle}>
          <Typography.Text>{aspect.entity_type_ids.map((id) => typeNames[id] ?? id).join("、") || "—"}</Typography.Text>
        </Form.Item>
      : null}
    </div>
    <div style={rowStyle}>
      <Form.Item label="关系类型（可调整）" labelCol={{ flex: "none" }} wrapperCol={{ flex: "1 1 0" }} style={{ ...itemStyle, flex: "1 1 320px" }}>
        <BatchMultiSelect label="切面关系类型多选" placeholder="留空显示全部关系"
          value={relationshipTypeIds} disabled={!aspect}
          onChange={onRelationshipTypeIdsChange} options={relationshipOptions} />
      </Form.Item>
      <Form.Item label="观测实体（可选）" labelCol={{ flex: "none" }} wrapperCol={{ flex: "1 1 0" }} style={{ ...itemStyle, flex: "1 1 360px" }}>
        <BatchMultiSelect label="切面观测实体多选" placeholder="留空观察整个切面范围"
          value={centerIds} disabled={!aspect}
          onChange={onCenterIdsChange} options={centerOptions} />
      </Form.Item>
      <Form.Item label="上游" style={{ ...itemStyle, flex: "0 0 auto" }}>
        <Select aria-label="切面上游跳数" value={upstreamDepth} style={{ width: 100 }} options={HOPS}
          disabled={!aspect} onChange={onUpstreamDepthChange} />
      </Form.Item>
      <Form.Item label="下游" style={{ ...itemStyle, flex: "0 0 auto" }}>
        <Select aria-label="切面下游跳数" value={downstreamDepth} style={{ width: 100 }} options={HOPS}
          disabled={!aspect} onChange={onDownstreamDepthChange} />
      </Form.Item>
    </div>
  </Form><div style={{ width: "100%" }}>{observation}</div></>;
}
