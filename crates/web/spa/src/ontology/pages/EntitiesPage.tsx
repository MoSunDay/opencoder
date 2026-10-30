import InspectLink from "../navigation/InspectLink";
import { DataTable } from "../ui";
import { Alert, App, Button, Select, Space, Tabs, Tag } from "antd";
import { useEffect, useState } from "react";
import { setState } from "../../store.js";
import { api } from "../api";
import { useEnv } from "../env";
import { useDraftGuard } from "../navigation/DraftGuard";
import type { Entity } from "../types";
import CreateEntityModal from "./entities/CreateEntityModal";
import EntityDetailDrawer from "./entityTypes/EntityDetailDrawer";
import { useResource } from "./admin/useResource";
import ListFilters, { matchesRecord, type StatusFilter } from "./admin/ListFilters";
import DangerMenu from "./admin/DangerMenu";
import DirectoryPanel from "./directories/DirectoryPanel";
import LevelBadge from "./freshness/LevelBadge";
import { entityDirectories, membershipType } from "./directories/directoryModel";
import { useDetailNavigation } from "./graph/details/useDetailNavigation";
import EntityRelations from "./graph/details/EntityRelations";
import RelationshipDrawer from "./graph/RelationshipDrawer";
import { writeObservation } from "./graph/session/observationMemory";

export default function EntitiesPage() {
  const { env, canManage } = useEnv();
  return <EntitiesContent key={env} env={env} canManage={canManage} />;
}
function EntitiesContent({ env, canManage }: { env: string; canManage: boolean }) {
  const { message } = App.useApp();
  const guard = useDraftGuard();
  const navigate = () => setState({ page: "ontologyGraph" });
  const detail = useDetailNavigation();
  const resource = useResource(env, async () => {
    const [entities, types, relationships, relationshipTypes, directories] = await Promise.all([
      api.entities(env, true), api.entityTypes(env, true), api.relationships(env), api.relationshipTypes(env, true), api.directoriesTree(env),
    ]);
    return { items: entities.items, types: types.items, relationships: relationships.items, relationshipTypes: relationshipTypes.items, directories: directories.items, rootId: directories.root_id };
  });
  const { items = [], types = [], relationships = [], relationshipTypes = [], directories = [], rootId = "" } = resource.data ?? {};
  const [query, setQuery] = useState("");
  const [status, setStatus] = useState<StatusFilter>("active");
  const [typeId, setTypeId] = useState<string>();
  const [page, setPage] = useState(1);
  const [tab, setTab] = useState("entities");
  const [focusDirectoryId, setFocusDirectoryId] = useState<string>();
  useEffect(() => setPage(1), [query, status, typeId]);
  const typeNames = Object.fromEntries(types.map((type) => [type.id, type.name]));
  const directoryNames = Object.fromEntries(directories.map((directory) => [directory.id, directory.name]));
  const membershipTypeId = membershipType(relationshipTypes)?.id;
  const focusDirectory = (id: string) => { setFocusDirectoryId(id); setTab("directories"); };
  const visible = items.filter((item) => item.entity_type_id !== "00000000-0000-4000-8000-000000000001"
    && (!typeId || item.entity_type_id === typeId)
    && matchesRecord(item, status, query, [item.name, item.description, item.id, typeNames[item.entity_type_id] ?? ""]));
  const selected = detail.current?.kind === "entity" ? items.find((item) => item.id === detail.current?.id) : undefined;
  const selectedRelationship = detail.current?.kind === "relationship" ? relationships.find((item) => item.id === detail.current?.id) : undefined;
  const openEntity = (id: string) => guard.run(() => detail.openEntity(id));
  return <div className="admin-content">
    {resource.error ? <Alert type="error" showIcon message={resource.data ? "更新失败，当前显示上次结果" : "实体加载失败"} description={resource.error}
      action={<Button onClick={() => void resource.reload()}>重试</Button>} /> : null}
    <Tabs activeKey={tab} onChange={(key) => guard.run(() => setTab(key))} items={[
      { key: "entities", label: "实体", children: <DataTable<Entity> scroll={{ x: 760 }} headerTitle="全部实体"
        rowKey="id" search={false} loading={resource.loading} dataSource={visible}
        pagination={{ current: page, pageSize: 20, onChange: setPage, showSizeChanger: false }}
        options={{ reload: () => resource.reload() }}
        tableExtraRender={() => <ListFilters query={query} status={status} count={visible.length} onQuery={setQuery} onStatus={setStatus}>
          <Select aria-label="筛选实体类型" allowClear showSearch optionFilterProp="label" placeholder="全部实体类型" value={typeId} onChange={setTypeId}
            options={types.map((type) => ({ value: type.id, label: type.name }))} />
        </ListFilters>}
        toolBarRender={() => canManage ? [<CreateEntityModal key="new" env={env} types={types} onCreated={resource.reload} />] : []}
        columns={[
          { title: "名称", dataIndex: "name", width: 300, ellipsis: true, render: (_, item) => <InspectLink onClick={() => openEntity(item.id)}>{item.name}</InspectLink> },
          { title: "实体类型", width: 140, render: (_, item) => <Tag>{typeNames[item.entity_type_id] ?? item.entity_type_id}</Tag> },
          { title: "所属目录", width: 220, render: (_, item) => {
            const ids = entityDirectories(relationships, membershipTypeId, item.id);
            if (ids.length === 0) return null;
            return <Space size={4} wrap>{ids.map((id) => directories.some((directory) => directory.id === id)
              ? <Tag key={id} className="entity-dir-tag" onClick={() => focusDirectory(id)}>{directoryNames[id]}</Tag> : null)}</Space>;
          } },
          { title: "描述", dataIndex: "description", ellipsis: true, width: 180 },
          { title: "等级", width: 80, render: (_, item) => <LevelBadge level={item.data_level} /> },
          { title: "状态", width: 80, render: (_, item) => item.is_deleted ? <Tag>已删除</Tag> : <Tag color="green">有效</Tag> },
          { title: "操作", width: 100, render: (_, item) => <Space size={0}>
            <Button type="link" onClick={() => openEntity(item.id)}>详情</Button>
            {canManage && !item.is_deleted ? <DangerMenu label="删除实体" description="属性和关系历史保留。" onConfirm={async () => {
              await api.updateEntity(env, item.id, { name: item.name, description: item.description, is_deleted: true, expected_revision: item.revision });
              message.success("实体已删除"); await resource.reload();
            }} /> : null}
          </Space> },
        ]} /> },
      { key: "directories", label: "目录", children: rootId ? <DirectoryPanel env={env} canManage={canManage} directories={directories} rootId={rootId}
        entities={items} relationships={relationships} relationshipTypes={relationshipTypes} entityTypes={types}
        reload={resource.reload} focusDirectoryId={focusDirectoryId} /> : null },
    ]} />
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
