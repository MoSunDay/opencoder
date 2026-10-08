import { Form } from "antd";
import { useMemo } from "react";
import type { Entity, EntityType, Relationship, RelationshipType } from "../../types";
import { entitiesOfTypes, incidentRelationshipTypeIds, relationshipTypeCandidates, relationshipTypeLabel, type ObservationSelection } from "./observationSelection";
import BatchMultiSelect from "./BatchMultiSelect";
import FilterSection from "./controls/FilterSection";
import HopSelect from "./controls/HopSelect";

type Props = {
  expandNeighbors?: boolean;
  entityTypes: EntityType[];
  entities: Entity[];
  relationshipTypes: RelationshipType[];
  relationships: Relationship[];
  selection: ObservationSelection;
  onChange: (patch: Partial<ObservationSelection>) => void;
};

export default function ObservationFilters({ entityTypes, entities, relationshipTypes, relationships, selection, onChange, expandNeighbors = false }: Props) {
  const typeNames = useMemo(() => Object.fromEntries(entityTypes.map((item) => [item.id, item.name])), [entityTypes]);
  const incidentIds = incidentRelationshipTypeIds(entities, relationships, selection.entityTypeIds);
  const candidates = relationshipTypeCandidates(relationshipTypes, selection.entityTypeIds, expandNeighbors)
    .filter((item) => !expandNeighbors || incidentIds.has(item.id));
  const centerOptions = entitiesOfTypes(entities, selection.entityTypeIds).map((item) => ({ value: item.id, label: item.name, description: typeNames[item.entity_type_id] }));
  return <FilterSection summary={`观测范围 · ${selection.entityTypeIds.length} 种实体类型 · ${selection.centerIds.length} 个中心 · 上游 ${selection.upstreamDepth} / 下游 ${selection.downstreamDepth} 跳`}>
    <Form layout="vertical">
      <div className="graph-filter-grid">
        <Form.Item label="实体类型">
          <BatchMultiSelect label="实体类型多选" placeholder="请选择实体类型" value={selection.entityTypeIds}
            disabled={!entityTypes.length} onChange={(entityTypeIds) => onChange({ entityTypeIds })}
            options={entityTypes.map((item) => ({ value: item.id, label: item.name }))} />
        </Form.Item>
        <Form.Item label={expandNeighbors ? "关系类型（可选）" : "关系类型"}>
          <BatchMultiSelect label="关系类型多选" placeholder={expandNeighbors ? "留空显示全部关系" : "请选择关系类型"}
            value={selection.relationshipTypeIds} disabled={!selection.entityTypeIds.length || !candidates.length}
            onChange={(relationshipTypeIds) => onChange({ relationshipTypeIds })}
            options={candidates.map((item) => ({ value: item.id, label: relationshipTypeLabel(item, typeNames) }))} />
        </Form.Item>
        <Form.Item label="观测实体">
          <BatchMultiSelect label="实体多选" placeholder="请选择观测实体" value={selection.centerIds}
            disabled={!selection.entityTypeIds.length || (!expandNeighbors && !selection.relationshipTypeIds.length)}
            onChange={(centerIds) => onChange({ centerIds })} options={centerOptions} />
        </Form.Item>
        <HopSelect upstream={selection.upstreamDepth} downstream={selection.downstreamDepth} maximum={9}
          onUpstream={(upstreamDepth) => onChange({ upstreamDepth })} onDownstream={(downstreamDepth) => onChange({ downstreamDepth })} />
      </div>
    </Form>
  </FilterSection>;
}
