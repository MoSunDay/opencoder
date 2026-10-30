import { App, Button, Empty, Select, Tag } from "antd";
import { useMemo, useState } from "react";
import { api } from "../../api";
import { useDraftGuard } from "../../navigation/DraftGuard";
import type { Directory, Entity, EntityType, Relationship } from "../../types";
import "./directories.css";

type MemberRow = { relation: Relationship; entity?: Entity; typeName: string };

type Props = {
  env: string; canManage: boolean; directory: Directory; entities: Entity[]; entityTypes?: EntityType[];
  memberships: Relationship[]; membershipTypeId?: string; isRoot?: boolean; reload: () => Promise<void>;
};

export default function MembersPanel({ env, canManage, directory, entities, entityTypes = [], memberships, membershipTypeId, isRoot, reload }: Props) {
  const { message } = App.useApp();
  const [selection, setSelection] = useState<string[]>([]);
  const [saving, setSaving] = useState(false);
  useDraftGuard(false, saving);
  const entityById = useMemo(() => new Map(entities.map((item) => [item.id, item])), [entities]);
  const typeNames = useMemo(() => Object.fromEntries(entityTypes.map((type) => [type.id, type.name])), [entityTypes]);
  const rows = useMemo<MemberRow[]>(() => memberships.map((relation) => {
    const entity = entityById.get(relation.source_entity_id);
    return { relation, entity, typeName: typeNames[entity?.entity_type_id ?? ""] ?? entity?.entity_type_id ?? "" };
  }), [memberships, entityById, typeNames]);
  const memberIds = useMemo(() => new Set(memberships.map((relation) => relation.source_entity_id)), [memberships]);
  const addOptions = useMemo(() => entities
    .filter((item) => !item.is_deleted && !memberIds.has(item.id))
    .map((item) => ({ value: item.id, label: item.name }))
    .sort((left, right) => left.label.localeCompare(right.label)), [entities, memberIds]);
  const removeMember = async (relation: Relationship) => {
    setSaving(true);
    try {
      await api.updateRelationship(env, relation.id, { is_deleted: true, expected_revision: relation.revision });
      message.success("成员已移出目录");
      await reload();
    } catch (reason) { message.error(reason instanceof Error ? reason.message : "移出失败"); }
    finally { setSaving(false); }
  };
  const addMembers = async () => {
    if (!membershipTypeId || selection.length === 0) return;
    setSaving(true);
    try {
      for (const sourceEntityId of selection) {
        await api.createRelationship(env, { relationship_type_id: membershipTypeId, source_entity_id: sourceEntityId, target_entity_id: directory.id });
      }
      message.success("实体已加入目录");
      setSelection([]);
      await reload();
    } catch (reason) { message.error(reason instanceof Error ? reason.message : "添加失败"); }
    finally { setSaving(false); }
  };
  return <section className="dir-members" aria-label="目录成员">
    <header className="dir-members-header">
      <span className="dir-members-title">{isRoot ? "根目录" : directory.name}</span>
      <Tag>{rows.length} 个成员</Tag>
    </header>
    {canManage && membershipTypeId ? <div className="dir-members-add">
      <Select mode="multiple" aria-label="添加实体" className="dir-members-select" allowClear showSearch optionFilterProp="label"
        placeholder="选择要加入目录的实体" value={selection} onChange={setSelection} options={addOptions} notFoundContent="暂无可加入的实体" />
      <Button type="primary" loading={saving} disabled={selection.length === 0} onClick={() => void addMembers()}>添加</Button>
    </div> : null}
    {rows.length === 0
      ? <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="目录暂无成员" />
      : <ul className="dir-members-list">
        {rows.map((row) => <li key={row.relation.id} className="dir-member-row">
          <span className="dir-member-name" title={row.entity?.description || row.entity?.name}>{row.entity?.name ?? row.relation.source_entity_id}</span>
          <Tag>{row.typeName || "未知类型"}</Tag>
          {canManage ? <Button type="link" size="small" danger aria-label={`移出 ${row.entity?.name ?? row.relation.source_entity_id}`}
            onClick={() => void removeMember(row.relation)}>移出</Button> : null}
        </li>)}
      </ul>}
  </section>;
}
