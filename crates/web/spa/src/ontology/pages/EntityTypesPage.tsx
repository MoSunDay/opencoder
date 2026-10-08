import { PlusOutlined } from "@ant-design/icons";
import { TextField, TextAreaField, DataTable } from "../ui";
import { Alert, App, Button, Space, Tag, Typography } from "antd";
import { useEffect, useState } from "react";
import { api } from "../api";
import { useEnv } from "../env";
import type { EntityType } from "../types";
import { ModalForm } from "../ui/ModalForm";
import { useDraftGuard } from "../navigation/DraftGuard";
import EntityTypeDetailDrawer from "./entityTypes/EntityTypeDetailDrawer";
import { useResource } from "./admin/useResource";
import ListFilters, { matchesRecord, type StatusFilter } from "./admin/ListFilters";
import DangerMenu from "./admin/DangerMenu";

export default function EntityTypesPage() {
  const { env, canManage } = useEnv();
  return <EntityTypesContent key={env} env={env} canManage={canManage} />;
}
function EntityTypesContent({ env, canManage }: { env: string; canManage: boolean }) {
  const { message } = App.useApp();
  const guard = useDraftGuard();
  const [selectedId, setSelectedId] = useState<string>();
  const [query, setQuery] = useState("");
  const [status, setStatus] = useState<StatusFilter>("active");
  const [page, setPage] = useState(1);
  const resource = useResource(env, async () => (await api.entityTypes(env, true)).items);
  const items = resource.data ?? [];
  const selected = items.find((item) => item.id === selectedId);
  const visible = items.filter((item) => matchesRecord(item, status, query, [item.name, item.type_key, item.description, item.id]));
  useEffect(() => setPage(1), [query, status]);
  const open = (id: string) => guard.run(() => setSelectedId(id));
  return <div className="admin-content">
    {resource.error ? <Alert type="error" showIcon message="实体类型加载失败" description={resource.error} action={<Button onClick={() => void resource.reload()}>重试</Button>} /> : null}
    <DataTable<EntityType> headerTitle="实体类型" rowKey="id" search={false} dataSource={visible} loading={resource.loading} scroll={{ x: 660 }}
      pagination={{ current: page, pageSize: 20, onChange: setPage, showSizeChanger: false }} options={{ reload: () => resource.reload() }}
      tableExtraRender={() => <ListFilters query={query} status={status} count={visible.length} onQuery={setQuery} onStatus={setStatus} />}
      onRow={(row) => ({ onClick: () => open(row.id) })}
      toolBarRender={() => canManage ? [
        <ModalForm key="new" title="新增实体类型" trigger={<Button type="primary" icon={<PlusOutlined />}>新增</Button>}
          onFinish={async (value) => { await api.createEntityType(env, value); message.success("实体类型已创建"); await resource.reload(); return true; }}>
          <TextField name="key" label="类型 Key" rules={[{ required: true }]} />
          <TextField name="name" label="名称" rules={[{ required: true }]} />
          <TextAreaField name="description" label="描述" />
        </ModalForm>,
      ] : []}
      columns={[
        { title: "名称", dataIndex: "name", width: 240, render: (_, row) => <Typography.Text strong>{row.name}</Typography.Text> },
        { title: "描述", dataIndex: "description", ellipsis: true, width: 240 },
        { title: "状态", width: 90, render: (_, row) => row.is_deleted ? <Tag>已停用</Tag> : row.is_system ? <Tag color="blue">系统</Tag> : <Tag color="green">有效</Tag> },
        { title: "操作", width: 100, render: (_, row) => <Space size={0}>
          <Button type="link" onClick={(event) => { event.stopPropagation(); open(row.id); }}>详情</Button>
          {canManage && !row.is_system && !row.is_deleted ? <DangerMenu label="停用类型" description="已有实体保留，仅停用此类型。" onConfirm={async () => {
            await api.updateEntityType(env, row.id, { name: row.name, description: row.description, is_deleted: true, expected_revision: row.revision });
            message.success("实体类型已停用"); await resource.reload();
          }} /> : null}
        </Space> },
      ]} />
    <EntityTypeDetailDrawer key={`${env}:${selectedId ?? "closed"}`} env={env} entityType={selected} canManage={canManage}
      onClose={() => guard.run(() => setSelectedId(undefined))} onSaved={resource.reload} />
  </div>;
}
