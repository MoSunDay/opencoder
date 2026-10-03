import { Form, Select, Typography } from "antd";
import { useMemo } from "react";
import type { CSSProperties } from "react";
import type { Entity, EntityType, Relationship, RelationshipType } from "../../types";
import {
  entitiesOfTypes, incidentRelationshipTypeIds, relationshipTypeCandidates, relationshipTypeLabel, type ObservationSelection,
} from "./observationSelection";
import BatchMultiSelect from "./BatchMultiSelect";

const HOPS = [0, 1, 2, 3, 4, 5].map((value) => ({ value, label: `${value} 跳` }));
const itemStyle = { margin: 0, minWidth: 0, maxWidth: "100%" };
const rowStyle: CSSProperties = { display: "flex", flexWrap: "wrap", columnGap: 16, rowGap: 12, width: "100%", alignItems: "flex-start" };

type Props = {
  expandNeighbors?: boolean;
  entityTypes: EntityType[];
  entities: Entity[];
  relationshipTypes: RelationshipType[];
  relationships: Relationship[];
  selection: ObservationSelection;
  onChange: (patch: Partial<ObservationSelection>) => void;
};

export default function ObservationFilters({
  entityTypes, entities, relationshipTypes, relationships, selection, onChange, expandNeighbors = false,
}: Props) {
  const typeNames = useMemo(() => Object.fromEntries(entityTypes.map((item) => [item.id, item.name])), [entityTypes]);
  const incidentIds = incidentRelationshipTypeIds(entities, relationships, selection.entityTypeIds);
  const candidates = relationshipTypeCandidates(relationshipTypes, selection.entityTypeIds, expandNeighbors)
    .filter((item) => !expandNeighbors || incidentIds.has(item.id));
  const relationshipTypeOptions = candidates.map((item) => ({ value: item.id, label: relationshipTypeLabel(item, typeNames) }));
  const centerOptions = entitiesOfTypes(entities, selection.entityTypeIds).map((item) => ({ value: item.id, label: item.name }));
  return <Form layout="inline" style={{ marginBottom: 16, display: "flex", flexWrap: "wrap", columnGap: 16, rowGap: 12 }}>
    <div style={rowStyle}>
      <Typography.Text strong style={{ lineHeight: "32px" }}>第一步 · 拓扑范围</Typography.Text>
      <Form.Item label="实体类型" labelCol={{ flex: "none" }} wrapperCol={{ flex: "1 1 0" }} style={{ ...itemStyle, flex: "1 1 260px" }}>
        <BatchMultiSelect label="实体类型多选" placeholder="请选择实体类型" value={selection.entityTypeIds}
          disabled={!entityTypes.length}
          onChange={(entityTypeIds) => onChange({ entityTypeIds })}
          options={entityTypes.map((item) => ({ value: item.id, label: item.name }))} />
      </Form.Item>
      <Form.Item label="关系类型" labelCol={{ flex: "none" }} wrapperCol={{ flex: "1 1 0" }} style={{ ...itemStyle, flex: "1 1 260px" }}>
        <BatchMultiSelect label="关系类型多选" placeholder="请选择关系类型" value={selection.relationshipTypeIds}
          disabled={!selection.entityTypeIds.length || !candidates.length}
          onChange={(relationshipTypeIds) => onChange({ relationshipTypeIds })}
          options={relationshipTypeOptions} />
      </Form.Item>
    </div>
    <div style={rowStyle}>
      <Typography.Text strong style={{ lineHeight: "32px" }}>第二步 · 观测实体</Typography.Text>
      <Form.Item label="实体" labelCol={{ flex: "none" }} wrapperCol={{ flex: "1 1 0" }} style={{ ...itemStyle, flex: "1 1 360px" }}>
        <BatchMultiSelect label="实体多选" placeholder="请选择观测实体" value={selection.centerIds}
          disabled={!selection.entityTypeIds.length || (!expandNeighbors && !selection.relationshipTypeIds.length)}
          onChange={(centerIds) => onChange({ centerIds })}
          options={centerOptions} />
      </Form.Item>
      <Form.Item label="上游" style={{ ...itemStyle, flex: "0 0 auto" }}>
        <Select aria-label="上游跳数" value={selection.upstreamDepth} style={{ width: 100 }} options={HOPS}
          onChange={(upstreamDepth) => onChange({ upstreamDepth })} />
      </Form.Item>
      <Form.Item label="下游" style={{ ...itemStyle, flex: "0 0 auto" }}>
        <Select aria-label="下游跳数" value={selection.downstreamDepth} style={{ width: 100 }} options={HOPS}
          onChange={(downstreamDepth) => onChange({ downstreamDepth })} />
      </Form.Item>
    </div>
  </Form>;
}
