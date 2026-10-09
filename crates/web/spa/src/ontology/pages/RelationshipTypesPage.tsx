import InspectLink from "../navigation/InspectLink";
import { DataTable } from "../ui";
import { Alert, App, Button, Collapse, Descriptions, Drawer, Grid, Space, Tag, Typography } from "antd";
import { useEffect, useState } from "react";
import { api } from "../api";
import { useEnv } from "../env";
import RelationshipTypeForm, { relationshipScope } from "./relationshipTypes/RelationshipTypeForm";
import CreateRelationshipModal from "./relationshipTypes/CreateRelationshipModal";
import type { RelationshipType } from "../types";
import { useResource } from "./admin/useResource";
import ListFilters, { matchesRecord, type StatusFilter } from "./admin/ListFilters";
import DangerMenu from "./admin/DangerMenu";
import { useDraftGuard } from "../navigation/DraftGuard";

export default function RelationshipTypesPage() {
  const { env, canManage } = useEnv();
  return <RelationshipTypesContent key={env} env={env} canManage={canManage} />;
}
function RelationshipTypesContent({ env, canManage }: { env: string; canManage: boolean }) {
  const { message } = App.useApp();
  const screens = Grid.useBreakpoint();
  const guard = useDraftGuard();
  const [query, setQuery] = useState("");
  const [status, setStatus] = useState<StatusFilter>("active");
  const [page, setPage] = useState(1);
  const [selectedId, setSelectedId] = useState<string>();
  const resource = useResource(env, async () => {
    const [result, typeResult] = await Promise.all([api.relationshipTypes(env, true), api.entityTypes(env, true)]);
    return { items: result.items, types: typeResult.items };
  });
  const { items = [], types = [] } = resource.data ?? {};
  const names = Object.fromEntries(types.map((type) => [type.id, type.name]));
  const selected = items.find((item) => item.id === selectedId);
  const sourceName = (item: RelationshipType) => item.source_entity_type_id ? names[item.source_entity_type_id] ?? item.source_entity_type_id : "不限类型";
  const targets = (item: RelationshipType) => (item.target_entity_type_ids ?? []).map((id) => names[id] ?? id);
  const visible = items.filter((item) => matchesRecord(item, status, query, [item.name, item.type_key, item.description, sourceName(item), ...targets(item)]));
  useEffect(() => setPage(1), [query, status]);
  const open = (id: string) => guard.run(() => setSelectedId(id));
  return <div className="admin-content">
    {resource.error ? <Alert type="error" showIcon message="关系类型加载失败" description={resource.error} action={<Button onClick={() => void resource.reload()}>重试</Button>} /> : null}
    <DataTable<RelationshipType> headerTitle="关系类型" rowKey="id" search={false} dataSource={visible} loading={resource.loading} scroll={{ x: 840 }}
      pagination={{ current: page, pageSize: 20, onChange: setPage, showSizeChanger: false }} options={{ reload: () => resource.reload() }}
      tableExtraRender={() => <ListFilters query={query} status={status} count={visible.length} onQuery={setQuery} onStatus={setStatus} />}
      toolBarRender={() => canManage ? [<RelationshipTypeForm key={env} env={env} types={types} onSaved={resource.reload} />] : []}
      columns={[
        { title: "名称", dataIndex: "name", width: 210, render: (_, row) => <InspectLink onClick={() => open(row.id)}>{row.name}</InspectLink> },
        { title: "源类型 →", width: 150, render: (_, row) => sourceName(row) },
        { title: "目标类型", width: 240, render: (_, row) => <Space wrap size={4}>
          {targets(row).slice(0, 2).map((name) => <Tag key={name}>{name}</Tag>)}
          {targets(row).length > 2 ? <Button type="link" size="small" onClick={() => open(row.id)}>另 {targets(row).length - 2} 种</Button> : null}
          {!targets(row).length ? "不限类型" : null}
        </Space> },
        { title: "状态", width: 90, render: (_, row) => row.is_deleted ? <Tag>已停用</Tag> : <Tag color="green">有效</Tag> },
        { title: "操作", width: 150, render: (_, row) => <Space size={0}>
          <Button type="link" onClick={() => open(row.id)}>详情</Button>
          {canManage && !row.is_deleted && !row.is_directory_membership ? <CreateRelationshipModal env={env} relationshipType={row} entityTypes={types} onCreated={resource.reload} /> : null}
          {canManage && !row.is_system && !row.is_deleted ? <DangerMenu label="停用类型" description="已有关系保留，仅停用此类型。" onConfirm={async () => {
            await api.updateRelationshipType(env, row.id, { name: row.name, description: row.description, is_deleted: true, expected_revision: row.revision, ...relationshipScope(row) });
            message.success("关系类型已停用"); await resource.reload();
          }} /> : null}
        </Space> },
      ]} />
    <Drawer title={selected?.name ?? "关系类型详情"} open={Boolean(selected)} width={screens.md ? 640 : "100%"} onClose={() => guard.run(() => setSelectedId(undefined))} destroyOnHidden
      extra={selected && canManage && !selected.is_deleted ? <RelationshipTypeForm key={`${env}:${selected.id}:${selected.revision}`} env={env} types={types} item={selected} onSaved={resource.reload} /> : null}>
      {selected ? <><Descriptions column={1} items={[
        { key: "source", label: "源类型", children: sourceName(selected) },
        { key: "targets", label: "目标类型", children: targets(selected).length ? <Space wrap>{targets(selected).map((name) => <Tag key={name}>{name}</Tag>)}</Space> : "不限类型" },
        { key: "description", label: "描述", children: selected.description || "尚未填写描述" },
        { key: "purpose", label: "用途", children: selected.is_directory_membership ? "目录归属" : "普通关系" },
        { key: "origin", label: "来源", children: selected.is_system ? "系统" : "自定义" },
      ]} /><Collapse items={[{ key: "record", label: "记录信息", children: <Descriptions column={1} items={[
        { key: "key", label: "Key", children: <Typography.Text copyable>{selected.type_key}</Typography.Text> },
        { key: "id", label: "ID", children: <Typography.Text copyable>{selected.id}</Typography.Text> },
        { key: "revision", label: "版本", children: selected.revision },
      ]} /> }]} /></> : null}
    </Drawer>
  </div>;
}
