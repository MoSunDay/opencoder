import { useInitialLoad, LoadFeedback } from "../ui/useInitialLoad";
import { DataTable } from "../ui";
import { App, Button, Popconfirm, Space, Tag } from "antd";
import { useState } from "react";
import { api } from "../api";
import { useEnv } from "../env";
import RelationshipTypeForm, { relationshipScope } from "./relationshipTypes/RelationshipTypeForm";
import CreateRelationshipModal from "./relationshipTypes/CreateRelationshipModal";
import type { Entity, EntityType, RelationshipType } from "../types";

export default function RelationshipTypesPage() {
  const { env, canManage } = useEnv();
  return <RelationshipTypesContent key={env} env={env} canManage={canManage} />;
}

function RelationshipTypesContent({ env, canManage }: { env: string; canManage: boolean }) {
  const { message } = App.useApp();
  const [items, setItems] = useState<RelationshipType[]>([]);
  const [types, setTypes] = useState<EntityType[]>([]);
  const [entities, setEntities] = useState<Entity[]>([]);
  const load = async () => {
    const [result, typeResult, entityResult] = await Promise.all([
      api.relationshipTypes(env, true), api.entityTypes(env, true), api.entities(env),
    ]);
    setItems(result.items);
    setTypes(typeResult.items);
    setEntities(entityResult.items);
  };
  const status = useInitialLoad(env, load);
  const names = Object.fromEntries(types.map((type) => [type.id, type.name]));
  return <><LoadFeedback status={status} /><DataTable<RelationshipType>
    headerTitle="关系类型"
    loading={status.loading} rowKey="id"
    search={false}
    dataSource={items}
    toolBarRender={() => canManage ? [
      <RelationshipTypeForm key={env} env={env} types={types} onSaved={load} />,
    ] : []}
    columns={[
      { title: "名称", dataIndex: "name" },
      { title: "Key", render: (_, row) => <Tag>{row.type_key}</Tag> },
      {
        title: "源类型",
        render: (_, row) => row.source_entity_type_id ? names[row.source_entity_type_id] ?? row.source_entity_type_id : <Tag>系统全局</Tag>,
      },
      { title: "允许目标", render: (_, row) => (row.target_entity_type_ids ?? []).map((id) => <Tag key={id}>{names[id] ?? id}</Tag>) },
      { title: "用途", render: (_, row) => row.is_directory_membership ? <Tag color="blue">目录归属</Tag> : "普通关系" },
      { title: "来源", render: (_, row) => row.is_system ? "系统" : "自定义" },
      { title: "Revision", dataIndex: "revision" },
      { title: "状态", render: (_, row) => row.is_deleted ? <Tag>已删除</Tag> : <Tag color="green">有效</Tag> },
      {
        title: "操作",
        render: (_, row) => canManage && !row.is_deleted ? <Space>
          {!row.is_directory_membership && (
            <CreateRelationshipModal env={env} relationshipType={row} entityTypes={types} entities={entities} onCreated={load} />
          )}
          <RelationshipTypeForm key={`${env}:${row.id}:${row.revision}`} env={env} types={types} item={row} onSaved={load} />
          {!row.is_system && (
            <Popconfirm
              title="已有关系保留，仅停用此类型"
              onConfirm={async () => {
                await api.updateRelationshipType(env, row.id, {
                  name: row.name, description: row.description, is_deleted: true, expected_revision: row.revision,
                  ...relationshipScope(row),
                });
                message.success("关系类型已停用");
                await load();
              }}>
              <Button danger type="link">停用</Button>
            </Popconfirm>
          )}
        </Space> : null,
      },
    ]} /></>;
}
