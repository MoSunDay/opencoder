import InspectLink from "../../../navigation/InspectLink";
import { Button, Input, List, Select, Space, Tag, Typography } from "antd";
import { useMemo, useState } from "react";
import type { Entity, Relationship, RelationshipType } from "../../../types";

type Props = { entityId: string; entities: Entity[]; relationships: Relationship[]; relationshipTypes: RelationshipType[];
  observedIds: string[]; onEntity: (id: string) => void; onRelationship: (id: string) => void };
export default function EntityRelations({ entityId, entities, relationships, relationshipTypes, observedIds, onEntity, onRelationship }: Props) {
  const [query, setQuery] = useState("");
  const [direction, setDirection] = useState("all");
  const [relationType, setRelationType] = useState<string>();
  const [page, setPage] = useState(1);
  const names = useMemo(() => Object.fromEntries(entities.map((entity) => [entity.id, entity.name])), [entities]);
  const types = useMemo(() => Object.fromEntries(relationshipTypes.map((type) => [type.id, type.name])), [relationshipTypes]);
  const all = relationships.filter((edge) => !edge.is_deleted && (edge.source_entity_id === entityId || edge.target_entity_id === entityId));
  const incoming = all.filter((edge) => edge.target_entity_id === entityId).length;
  const outgoing = all.filter((edge) => edge.source_entity_id === entityId).length;
  const filtered = all.filter((edge) => (direction === "all" || (direction === "in" ? edge.target_entity_id === entityId : edge.source_entity_id === entityId))
    && (!relationType || edge.relationship_type_id === relationType)
    && `${types[edge.relationship_type_id]} ${names[edge.source_entity_id]} ${names[edge.target_entity_id]}`.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase()))
    .sort((a, b) => (types[a.relationship_type_id] ?? "").localeCompare(types[b.relationship_type_id] ?? "", "zh-CN") || a.id.localeCompare(b.id));
  return <section style={{ marginTop: 20 }} aria-label="实体关联关系">
    <Typography.Title level={5}>关联关系 <Typography.Text type="secondary">{all.length}</Typography.Text></Typography.Title>
    <Typography.Paragraph type="secondary">这里列出全部已确认关联；当前观测范围以标签标明。</Typography.Paragraph>
    <Space wrap style={{ marginBottom: 12 }}>
      <Select aria-label="关联方向" value={direction} onChange={(value) => { setDirection(value); setPage(1); }} options={[
        { value: "all", label: "全部方向" }, { value: "in", label: `上游（${incoming}）` }, { value: "out", label: `下游（${outgoing}）` },
      ]} />
      <Select aria-label="关联关系类型" allowClear placeholder="全部关系类型" style={{ minWidth: 150 }} value={relationType} onChange={(value) => { setRelationType(value); setPage(1); }}
        options={[...new Set(all.map((edge) => edge.relationship_type_id))].map((id) => ({ value: id, label: types[id] ?? id }))} />
      <Input.Search aria-label="搜索关联实体" allowClear placeholder="搜索关联实体或关系" value={query} onChange={(e) => { setQuery(e.target.value); setPage(1); }} />
    </Space>
    <List size="small" dataSource={filtered} locale={{ emptyText: all.length ? "没有匹配的关联，可调整搜索条件" : "暂无已确认关联" }}
      pagination={filtered.length > 20 ? { current: page, pageSize: 20, onChange: setPage, showSizeChanger: false } : false}
      renderItem={(edge) => {
        const incoming = edge.target_entity_id === entityId;
        const neighbor = incoming ? edge.source_entity_id : edge.target_entity_id;
        return <List.Item><div style={{ width: "100%" }}>
          <Space wrap size={4}><Tag>{incoming ? "上游 → 当前" : "当前 → 下游"}</Tag>
            <Button type="link" size="small" onClick={() => onRelationship(edge.id)}>{types[edge.relationship_type_id] ?? "关系"}</Button>
            <Tag color={observedIds.includes(edge.id) ? "blue" : undefined}>{observedIds.includes(edge.id) ? "当前范围" : "范围外"}</Tag></Space>
          <div><InspectLink onClick={() => onEntity(neighbor)}>{names[neighbor] ?? neighbor}</InspectLink></div>
        </div></List.Item>;
      }} />
  </section>;
}
