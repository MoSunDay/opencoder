import { useInitialLoad, LoadFeedback } from "../ui/useInitialLoad";
import { FolderAddOutlined } from "@ant-design/icons";
import {
  ModalForm,
  Section,
  SelectField,
  TextField,
  TextAreaField,
  DataTable,
} from "../ui";
import { App, Button, Input, Select, Grid, Popconfirm, Space, Tag, Tree } from "antd";
import type { DataNode } from "antd/es/tree";
import { useMemo, useState } from "react";
import { api } from "../api";
import { useEnv } from "../env";
import type { DirectoryItem, Entity, EntityType, Relationship } from "../types";
import CreateEntityModal from "./entities/CreateEntityModal";
import EntityDetailDrawer from "./entityTypes/EntityDetailDrawer";
import DirectoryActions from "./DirectoryActions";

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
  const screens = Grid.useBreakpoint();
  const { message } = App.useApp();
  const [items, setItems] = useState<Entity[]>([]);
  const [types, setTypes] = useState<EntityType[]>([]);
  const [directories, setDirectories] = useState<DirectoryItem[]>([]);
  const [relationships, setRelationships] = useState<Relationship[]>([]);
  const [query, setQuery] = useState("");
  const [typeFilter, setTypeFilter] = useState<string>();
  const [selectedDirectory, setSelectedDirectory] = useState<string>();
  const [selected, setSelected] = useState<Entity>();
  const load = async () => {
    const [e, t, d, r] = await Promise.all([
      api.entities(env, true),
      api.entityTypes(env, true),
      api.directories(env),
      api.relationships(env),
    ]);
    setItems(e.items);
    setTypes(t.items);
    setDirectories(d.items);
    setRelationships(r.items);
  };
  const status = useInitialLoad(env, load);
  const visible = useMemo(() => {
    const ordinary = items.filter(
      (item) => item.entity_type_id !== "00000000-0000-4000-8000-000000000001"
    );
    return selectedDirectory
      ? ordinary.filter((item) =>
          relationships.some(
            (r) =>
              r.source_entity_id === item.id &&
              r.target_entity_id === selectedDirectory &&
              !r.is_deleted
          )
        )
      : ordinary;
  }, [items, relationships, selectedDirectory]);
  const typeNames = useMemo(
    () => Object.fromEntries(types.map((t) => [t.id, t.name])),
    [types]
  );
  const open = async (item: Entity) => {
    setSelected(item);
  };
  return (
    <>
      <Section split={screens.md ? "vertical" : "horizontal"}>
        <Section
          colSpan={screens.md ? "280px" : "100%"}
          title="目录"
          extra={
            canManage ? (
              <Space size={0}>
                <DirectoryActions
                  env={env}
                  selected={directories.find(
                    (item) => item.id === selectedDirectory
                  )}
                  directories={directories}
                  onDone={async () => {
                    setSelectedDirectory(undefined);
                    await load();
                  }}
                />
                <ModalForm
                  title="新建目录"
                  trigger={<Button type="text" icon={<FolderAddOutlined />} />}
                  onFinish={async (value) => {
                    await api.createDirectory(env, value);
                    message.success("目录已创建");
                    await load();
                    return true;
                  }}
                >
                  <TextField
                    name="name"
                    label="名称"
                    rules={[{ required: true }]}
                  />
                  <SelectField
                    name="parent_id"
                    label="父目录"
                    options={directories.map((d) => ({
                      value: d.id,
                      label: d.name,
                    }))}
                  />
                  <TextAreaField name="description" label="描述" />
                </ModalForm>
              </Space>
            ) : null
          }
        >
          <Button type="link" onClick={() => setSelectedDirectory(undefined)}>
            全部实体
          </Button>
          <Tree
            blockNode
            selectedKeys={selectedDirectory ? [selectedDirectory] : []}
            treeData={treeData(directories.filter((d) => !d.is_deleted))}
            onSelect={(keys) => setSelectedDirectory(keys[0]?.toString())}
          />
        </Section>
        <Section>
          <LoadFeedback status={status} /><DataTable<Entity>
            scroll={{ x: 800 }}
            headerTitle={<Space wrap><Input aria-label="搜索实体" placeholder="搜索名称或描述" value={query} onChange={(event) => setQuery(event.target.value)} /><Select aria-label="筛选实体类型" allowClear placeholder="实体类型" value={typeFilter} onChange={setTypeFilter} options={types.map((item) => ({ value: item.id, label: item.name }))} style={{ minWidth: 140 }} /></Space>}
            loading={status.loading} rowKey="id"
            search={false}
            dataSource={visible.filter((item) => (!typeFilter || item.entity_type_id === typeFilter) && `${item.name} ${item.description}`.toLowerCase().includes(query.toLowerCase()))}
            toolBarRender={() =>
              canManage
                ? [
                    <CreateEntityModal
                      key="new"
                      env={env}
                      types={types}
                      directoryId={selectedDirectory}
                      onCreated={async () => {
                        await load();
                      }}
                    />,
                  ]
                : []
            }
            columns={[
              { title: "名称", dataIndex: "name" },
              {
                title: "实体类型",
                dataIndex: "entity_type_id",
                render: (_, r) => (
                  <Tag>{typeNames[r.entity_type_id] || r.entity_type_id}</Tag>
                ),
              },
              {
                title: "描述",
                dataIndex: "description",
                ellipsis: true,
              },
              { title: "Revision", dataIndex: "revision" },
              {
                title: "状态",
                render: (_, r) =>
                  r.is_deleted ? (
                    <Tag>已删除</Tag>
                  ) : (
                    <Tag color="green">有效</Tag>
                  ),
              },
              {
                title: "操作",
                render: (_, r) => (
                  <Space>
                    <Button type="link" onClick={() => void open(r)}>
                      详情
                    </Button>
                    {canManage && !r.is_deleted && (
                      <Popconfirm
                        title="仅软删除，属性和关系历史保留"
                        onConfirm={async () => {
                          await api.updateEntity(env, r.id, {
                            name: r.name,
                            description: r.description,
                            is_deleted: true,
                            expected_revision: r.revision,
                          });
                          message.success("实体已软删除");
                          await load();
                        }}
                      >
                        <Button danger type="link">
                          软删除
                        </Button>
                      </Popconfirm>
                    )}
                  </Space>
                ),
              },
            ]}
          />
        </Section>
      </Section>
      <EntityDetailDrawer
        env={env}
        entity={selected}
        entityType={types.find((type) => type.id === selected?.entity_type_id)}
        canManage={canManage}
        onClose={() => setSelected(undefined)}
        onEntityChanged={async (item) => {
          setSelected((current) => current?.id === item.id ? item : current);
          await load();
        }}
      />{" "}
    </>
  );
}
