import InspectLink from "../navigation/InspectLink";
import { FolderAddOutlined } from "@ant-design/icons";
import { Section, SelectField, TextField, TextAreaField, DataTable } from "../ui";
import { Alert, App, Button, Grid, Select, Space, Tag, Tree } from "antd";
import type { DataNode } from "antd/es/tree";
import { useEffect, useState } from "react";
import { setState } from "../../store.js";
import { api } from "../api";
import { useEnv } from "../env";
import { useDraftGuard } from "../navigation/DraftGuard";
import { ModalForm } from "../ui/ModalForm";
import type { DirectoryItem, Entity } from "../types";
import CreateEntityModal from "./entities/CreateEntityModal";
import EntityDetailDrawer from "./entityTypes/EntityDetailDrawer";
import DirectoryActions from "./DirectoryActions";
import { useResource } from "./admin/useResource";
import ListFilters, { matchesRecord, type StatusFilter } from "./admin/ListFilters";
import DangerMenu from "./admin/DangerMenu";
import { useDetailNavigation } from "./graph/details/useDetailNavigation";
import EntityRelations from "./graph/details/EntityRelations";
import RelationshipDrawer from "./graph/RelationshipDrawer";
import { writeObservation } from "./graph/session/observationMemory";

function treeData(items: DirectoryItem[]): DataNode[] {
  const children = new Map<string | undefined, DirectoryItem[]>();
  for (const item of items) {
    const group = children.get(item.parent_id) || [];
    group.push(item);
    children.set(item.parent_id, group);
  }
  const build = (
    parent: string | undefined,
    seen = new Set<string>()
  ): DataNode[] =>
    (children.get(parent) || [])
      .filter((item) => !seen.has(item.id))
      .map((item) => {
        const next = new Set(seen).add(item.id);
        return {
          key: item.id,
          title: item.name,
          children: build(item.id, next),
        };
      });
  const roots = build(undefined);
  return roots.length
    ? roots
    : items.map((item) => ({ key: item.id, title: item.name }));
}
export default function EntitiesPage() {
  const { env, canManage } = useEnv();
  return <EntitiesContent key={env} env={env} canManage={canManage} />;
}
function EntitiesContent({ env, canManage }: { env: string; canManage: boolean }) {
  const screens = Grid.useBreakpoint();
  const { message } = App.useApp();
  const guard = useDraftGuard();
  const navigate = () => setState({ page: "ontologyGraph" });
  const detail = useDetailNavigation();
  const resource = useResource(env, async () => {
    const [entities, types, directories, relationships, relationshipTypes] = await Promise.all([
      api.entities(env, true), api.entityTypes(env, true), api.directories(env), api.relationships(env), api.relationshipTypes(env, true),
    ]);
    return { items: entities.items, types: types.items, directories: directories.items, relationships: relationships.items, relationshipTypes: relationshipTypes.items };
  });
  const { items = [], types = [], directories = [], relationships = [], relationshipTypes = [] } = resource.data ?? {};
  const [selectedDirectory, setSelectedDirectory] = useState<string>();
  const [query, setQuery] = useState("");
  const [status, setStatus] = useState<StatusFilter>("active");
  const [typeId, setTypeId] = useState<string>();
  const [page, setPage] = useState(1);
  useEffect(() => setPage(1), [query, status, typeId, selectedDirectory]);
  const typeNames = Object.fromEntries(types.map((type) => [type.id, type.name]));
  const visible = items.filter((item) => item.entity_type_id !== "00000000-0000-4000-8000-000000000001"
    && (!typeId || item.entity_type_id === typeId)
    && matchesRecord(item, status, query, [item.name, item.description, item.id, typeNames[item.entity_type_id] ?? ""])
    && (!selectedDirectory || relationships.some((edge) => !edge.is_deleted && edge.source_entity_id === item.id && edge.target_entity_id === selectedDirectory)));
  const selected = detail.current?.kind === "entity" ? items.find((item) => item.id === detail.current?.id) : undefined;
  const selectedRelationship = detail.current?.kind === "relationship" ? relationships.find((item) => item.id === detail.current?.id) : undefined;
  const openEntity = (id: string) => guard.run(() => detail.openEntity(id));
  const directoryActions = canManage ? <Space size={0}>
    <DirectoryActions env={env} selected={directories.find((item) => item.id === selectedDirectory)} directories={directories}
      onDone={async () => { setSelectedDirectory(undefined); await resource.reload(); }} />
    <ModalForm title="新建目录" trigger={<Button type="text" aria-label="新建目录" icon={<FolderAddOutlined />} />}
      onFinish={async (value) => { await api.createDirectory(env, value); message.success("目录已创建"); await resource.reload(); return true; }}>
      <TextField name="name" label="名称" rules={[{ required: true }]} />
      <SelectField name="parent_id" label="父目录" options={directories.map((item) => ({ value: item.id, label: item.name }))} />
      <TextAreaField name="description" label="描述" />
    </ModalForm>
  </Space> : null;
  return <div className="admin-content">
    {resource.error ? <Alert type="error" showIcon message={resource.data ? "更新失败，当前显示上次结果" : "实体加载失败"} description={resource.error}
      action={<Button onClick={() => void resource.reload()}>重试</Button>} /> : null}
    <Section split={screens.md ? "vertical" : "horizontal"}>
      <Section colSpan={screens.md ? "230px" : "100%"} title={screens.md ? "目录" : undefined} extra={screens.md ? directoryActions : undefined}>
        {screens.md ? <><Button type="link" onClick={() => setSelectedDirectory(undefined)}>全部实体</Button>
          <Tree blockNode selectedKeys={selectedDirectory ? [selectedDirectory] : []} treeData={treeData(directories.filter((item) => !item.is_deleted))}
            onSelect={(keys) => setSelectedDirectory(keys[0]?.toString())} /></>
          : <Space style={{ width: "100%" }}><Select aria-label="实体目录" allowClear showSearch optionFilterProp="label" placeholder="全部目录" style={{ minWidth: 190 }}
              value={selectedDirectory} onChange={setSelectedDirectory} options={directories.filter((item) => !item.is_deleted).map((item) => ({ value: item.id, label: item.name }))} />{directoryActions}</Space>}
      </Section>
      <Section>
        <DataTable<Entity> scroll={{ x: 760 }} headerTitle={selectedDirectory ? `实体 · ${directories.find((item) => item.id === selectedDirectory)?.name ?? "当前目录"}` : "实体"}
          rowKey="id" search={false} loading={resource.loading} dataSource={visible}
          pagination={{ current: page, pageSize: 20, onChange: setPage, showSizeChanger: false }}
          options={{ reload: () => resource.reload() }}
          tableExtraRender={() => <ListFilters query={query} status={status} count={visible.length} onQuery={setQuery} onStatus={setStatus}>
            <Select aria-label="筛选实体类型" allowClear showSearch optionFilterProp="label" placeholder="全部实体类型" value={typeId} onChange={setTypeId}
              options={types.map((type) => ({ value: type.id, label: type.name }))} />
          </ListFilters>}
          toolBarRender={() => canManage ? [<CreateEntityModal key="new" env={env} types={types} directoryId={selectedDirectory} onCreated={resource.reload} />] : []}
          columns={[
            { title: "名称", dataIndex: "name", width: 300, ellipsis: true, render: (_, item) => <InspectLink onClick={() => openEntity(item.id)}>{item.name}</InspectLink> },
            { title: "实体类型", width: 140, render: (_, item) => <Tag>{typeNames[item.entity_type_id] ?? item.entity_type_id}</Tag> },
            { title: "描述", dataIndex: "description", ellipsis: true, width: 180 },
            { title: "状态", width: 80, render: (_, item) => item.is_deleted ? <Tag>已删除</Tag> : <Tag color="green">有效</Tag> },
            { title: "操作", width: 100, render: (_, item) => <Space size={0}>
              <Button type="link" onClick={() => openEntity(item.id)}>详情</Button>
              {canManage && !item.is_deleted ? <DangerMenu label="删除实体" description="属性和关系历史保留。" onConfirm={async () => {
                await api.updateEntity(env, item.id, { name: item.name, description: item.description, is_deleted: true, expected_revision: item.revision });
                message.success("实体已删除"); await resource.reload();
              }} /> : null}
            </Space> },
          ]} />
      </Section>
    </Section>
    <EntityDetailDrawer env={env} entity={selected} entityType={types.find((type) => type.id === selected?.entity_type_id)} canManage={canManage}
      onClose={detail.close} onEntityChanged={resource.reload} onBack={detail.canBack ? detail.back : undefined} view={detail.view} onView={detail.remember}
      onObserve={selected ? () => {
        writeObservation(env, { mode: "aspect-test", selection: { entityTypeIds: [selected.entity_type_id], relationshipTypeIds: [], centerIds: [selected.id], upstreamDepth: 3, downstreamDepth: 3 } });
        detail.close(); navigate();
      } : undefined}
      relations={selected ? <EntityRelations key={selected.id} entityId={selected.id} entities={items} relationships={relationships} relationshipTypes={relationshipTypes}
        observedIds={[]} onEntity={openEntity} onRelationship={(id) => guard.run(() => detail.openRelationship(id))} /> : undefined} />
    <RelationshipDrawer env={env} relationship={selectedRelationship} entities={items} relationshipTypes={relationshipTypes} canManage={canManage}
      onClose={detail.close} onChanged={resource.reload} onEntity={detail.openEntity} onBack={detail.canBack ? detail.back : undefined} />
  </div>;
}
