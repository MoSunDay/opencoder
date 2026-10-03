import { PlusOutlined } from "@ant-design/icons";
import {
  ModalForm,
  CheckField,
  SelectField,
  TextField,
  TextAreaField,
  DataTable,
} from "../../ui";
import { App, Button, Descriptions, Drawer, Popconfirm, Space, Tag, Tabs, Typography } from "antd";
import { useCallback, useEffect, useMemo, useState } from "react";
import { searchableLabels } from "../../forms/selectOptions";
import { api } from "../../api";
import type {
  AttributeDefinition,
  AttributeKind,
  Entity,
  EntityType,
  EntityTypeAction,
  Relationship,
  RelationshipType,
} from "../../types";
import EntityDetailDrawer from "./EntityDetailDrawer";

const kinds: { label: string; value: AttributeKind }[] = ["string", "integer", "float", "boolean", "datetime", "json", "text"].map(
  (value) => ({ label: value, value: value as AttributeKind }),
);

type Props = {
  env: string;
  entityType?: EntityType;
  canManage: boolean;
  onClose: () => void;
  onSaved: () => Promise<void>;
};

function ActionPanel({ env, entityType, canManage, actions, onReload }: { env: string; entityType: EntityType; canManage: boolean; actions: EntityTypeAction[]; onReload: () => Promise<void> }) {
  return (
    <div>
      <Typography.Paragraph>
        来源由实体的 source 属性维护。
      </Typography.Paragraph>
      <Typography.Paragraph>
        支持的 Action：
        {actions.filter((action) => !action.is_deleted).length > 0 ? (
          <Space wrap>
            {actions.filter((action) => !action.is_deleted).map((action) => (
              <Space key={action.id} size={2}><Tag color="blue">{action.operation_type}: {action.operation}</Tag>{canManage ? <Popconfirm title="停用此 Action？" onConfirm={async () => { await api.updateAction(env, action.id, { description: action.description, is_deleted: true, expected_revision: action.revision }); await onReload(); }}><Button type="link" danger size="small">停用</Button></Popconfirm> : null}</Space>
            ))}
          </Space>
        ) : (
          <Typography.Text type="secondary">无</Typography.Text>
        )}
      </Typography.Paragraph>
      {canManage && !entityType.is_deleted ? <ModalForm title="新增 Action" trigger={<Button icon={<PlusOutlined />}>新增 Action</Button>} onFinish={async (value) => { await api.createAction(env, entityType.id, value); await onReload(); return true; }}><SelectField showSearch fieldProps={searchableLabels} name="operation_type" label="读写" options={[{ label: "读", value: "read" }, { label: "写", value: "write" }]} rules={[{ required: true }]} /><TextField name="operation" label="操作" rules={[{ required: true }]} /><TextAreaField name="description" label="描述" /></ModalForm> : null}
    </div>
  );
}

export default function EntityTypeDetailDrawer({ env, entityType, canManage, onClose, onSaved }: Props) {
  const { message } = App.useApp();
  const [attributes, setAttributes] = useState<AttributeDefinition[]>([]);
  const [typeEntities, setTypeEntities] = useState<Entity[]>([]);
  const [allEntities, setAllEntities] = useState<Entity[]>([]);
  const [relationships, setRelationships] = useState<Relationship[]>([]);
  const [relationshipTypes, setRelationshipTypes] = useState<RelationshipType[]>([]);
  const [detail, setDetail] = useState<Entity>();
  const [actions, setActions] = useState<EntityTypeAction[]>([]);

  const loadAttributes = useCallback(
    async (type: EntityType) =>
      setAttributes((await api.attributes(env, type.id)).items.filter((item) => item.kind !== "vector")),
    [env],
  );
  const loadEntities = useCallback(
    async (type: EntityType) => {
      const result = await api.entities(env, true);
      setAllEntities(result.items);
      setTypeEntities(result.items.filter((item) => item.entity_type_id === type.id));
    },
    [env],
  );
  const loadRelationships = useCallback(async () => {
    const [rel, types] = await Promise.all([api.relationships(env), api.relationshipTypes(env)]);
    setRelationships(rel.items);
    setRelationshipTypes(types.items);
  }, [env]);
  const reload = useCallback(
    async (type: EntityType) => {
      await Promise.all([loadAttributes(type), loadEntities(type), loadRelationships(), api.actions(env, type.id, true).then((result) => setActions(result.items))]);
    },
    [loadAttributes, loadEntities, loadRelationships],
  );

  useEffect(() => {
    if (!entityType) {
      setAttributes([]);
      setTypeEntities([]);
      setAllEntities([]);
      setRelationships([]);
      setRelationshipTypes([]);
      setDetail(undefined);
      return;
    }
    void reload(entityType).catch((failure) => message.error(failure.message));
  }, [entityType, reload]);

  const typeEntityIds = useMemo(() => new Set(typeEntities.map((item) => item.id)), [typeEntities]);
  const typeRelationships = useMemo(
    () =>
      relationships.filter(
        (rel) => typeEntityIds.has(rel.source_entity_id) || typeEntityIds.has(rel.target_entity_id),
      ),
    [relationships, typeEntityIds],
  );
  const entityNames = useMemo(() => Object.fromEntries(allEntities.map((item) => [item.id, item.name])), [allEntities]);
  const relTypeNames = useMemo(
    () => Object.fromEntries(relationshipTypes.map((item) => [item.id, item.name])),
    [relationshipTypes],
  );

  return (
    <Drawer title={entityType?.name} size="75%" open={Boolean(entityType)} onClose={onClose} destroyOnHidden>
      {entityType ? (
        <>
          <Descriptions
            bordered
            column={2}
            style={{ marginBottom: 16 }}
            items={[
              { key: "key", label: "Key", children: <Tag>{entityType.type_key}</Tag> },
              {
                key: "status",
                label: "状态",
                children: entityType.is_deleted ? (
                  <Tag>已删除</Tag>
                ) : entityType.is_system ? (
                  <Tag color="blue">系统</Tag>
                ) : (
                  <Tag color="green">有效</Tag>
                ),
              },
              { key: "revision", label: "Revision", children: entityType.revision },
              { key: "description", label: "描述", children: entityType.description || "—" },
            ]}
          />
          {canManage && !entityType.is_deleted ? (
            <div style={{ marginBottom: 16 }}>
              <ModalForm
                key={`${entityType.id}:${entityType.revision}`}
                title="编辑实体类型"
                trigger={<Button type="primary">编辑基本信息</Button>}
                initialValues={{ name: entityType.name, description: entityType.description }}
                onFinish={async (values) => {
                  await api.updateEntityType(env, entityType.id, {
                    ...values,
                    is_deleted: false,
                    expected_revision: entityType.revision,
                  });
                  message.success("实体类型已更新");
                  await onSaved();
                  return true;
                }}
              >
                <TextField name="name" label="名称" rules={[{ required: true }]} />
                <TextAreaField name="description" label="描述" />
              </ModalForm>
            </div>
          ) : null}
          <Tabs
            items={[
              {
                key: "entities",
                label: "实体",
                children: (
                  <DataTable<Entity>
                    headerTitle="实体"
                    rowKey="id"
                    search={false}
                    options={false}
                    dataSource={typeEntities}
                    onRow={(row) => ({ onClick: () => setDetail(row) })}
                    columns={[
                      { title: "名称", dataIndex: "name" },
                      { title: "描述", dataIndex: "description", ellipsis: true },
                      {
                        title: "状态",
                        render: (_, row) => (row.is_deleted ? <Tag>已删除</Tag> : <Tag color="green">有效</Tag>),
                      },
                      { title: "Revision", dataIndex: "revision" },
                      {
                        title: "操作",
                        render: (_, row) => (
                          <Button
                            type="link"
                            onClick={(event) => {
                              event.stopPropagation();
                              setDetail(row);
                            }}
                          >
                            详情
                          </Button>
                        ),
                      },
                    ]}
                  />
                ),
              },
              {
                key: "attributes",
                label: "属性",
                children: (
                  <DataTable<AttributeDefinition>
                    headerTitle="属性定义"
                    rowKey="id"
                    search={false}
                    options={false}
                    dataSource={attributes}
                    toolBarRender={() =>
                      canManage && !entityType.is_deleted
                        ? [
                            <ModalForm
                              key="new"
                              title="新增属性"
                              trigger={<Button icon={<PlusOutlined />}>新增属性</Button>}
                              onFinish={async (value) => {
                                await api.createAttribute(env, entityType.id, {
                                  ...value,
                                  attribute_role: "custom",
                                  storage_mode: value.kind === "text" ? "markdown" : "sql",
                                });
                                message.success("属性已创建");
                                await loadAttributes(entityType);
                                return true;
                              }}
                            >
                              <TextField name="key" label="属性 Key" rules={[{ required: true }]} />
                              <TextField name="name" label="名称" rules={[{ required: true }]} />
                              <SelectField showSearch fieldProps={searchableLabels} name="kind" label="类型" options={kinds} rules={[{ required: true }]} />
                              <CheckField name="required">必填</CheckField>
                              <TextAreaField name="description" label="描述" />
                            </ModalForm>,
                          ]
                        : []
                    }
                    columns={[
                      { title: "名称", dataIndex: "name" },
                      { title: "Key", dataIndex: "attribute_key" },
                      { title: "类型", dataIndex: "kind", render: (_, row) => <Tag>{row.kind}</Tag> },
                      { title: "必填", dataIndex: "required", render: (value) => value ? "是" : "否" },
                      { title: "Revision", dataIndex: "revision" },
                      {
                        title: "操作",
                        render: (_, row) =>
                          canManage && !entityType.is_deleted && !row.is_deleted ? (
                            <Space>
                              <ModalForm
                                title="编辑属性定义"
                                trigger={<Button type="link">编辑</Button>}
                                initialValues={{ name: row.name, description: row.description, required: row.required, attribute_role: row.attribute_role, storage_mode: row.storage_mode }}
                                onFinish={async (values) => {
                                  await api.updateAttribute(env, row.id, {
                                    ...values,
                                    is_deleted: false,
                                    expected_revision: row.revision,
                                    attribute_role: row.attribute_role,
                                    storage_mode: values.storage_mode,
                                  });
                                  message.success("属性定义已更新");
                                  await loadAttributes(entityType);
                                  return true;
                                }}
                              >
                                <TextField name="name" label="名称" rules={[{ required: true }]} />
                                <CheckField name="required" disabled={row.attribute_role !== "custom"}>必填</CheckField>
                                <SelectField showSearch fieldProps={searchableLabels} name="storage_mode" label="存储模式" disabled={row.attribute_role === "source"} options={row.attribute_role === "ext" ? [{ label: "Markdown", value: "markdown" }, { label: "NFS path", value: "nfs_path" }] : row.kind === "text" ? [{ label: "Markdown", value: "markdown" }] : [{ label: "SQL", value: "sql" }]} />
                                <TextAreaField name="description" label="描述" />
                              </ModalForm>
                              {row.attribute_role === "custom" ? <Popconfirm
                                title="已有属性值保留，仅停用定义"
                                onConfirm={async () => {
                                  await api.updateAttribute(env, row.id, {
                                    name: row.name,
                                    description: row.description,
                                    required: row.required,
                                    is_deleted: true,
                                    expected_revision: row.revision,
                                  });
                                  message.success("属性定义已停用");
                                  await loadAttributes(entityType);
                                }}
                              >
                                <Button danger type="link">
                                  停用
                                </Button>
                              </Popconfirm> : null}
                            </Space>
                          ) : null,
                      },
                    ]}
                  />
                ),
              },
              {
                key: "actions",
                label: "Action",
                children: <ActionPanel env={env} entityType={entityType} canManage={canManage} actions={actions} onReload={async () => { setActions((await api.actions(env, entityType.id)).items); }} />,
              },
              {
                key: "relationships",
                label: "关系",
                children: (
                  <DataTable<Relationship>
                    headerTitle="关系"
                    rowKey="id"
                    search={false}
                    options={false}
                    dataSource={typeRelationships}
                    columns={[
                      {
                        title: "源实体",
                        dataIndex: "source_entity_id",
                        render: (_, row) => entityNames[row.source_entity_id] ?? row.source_entity_id,
                      },
                      {
                        title: "关系类型",
                        dataIndex: "relationship_type_id",
                        render: (_, row) => (
                          <Tag color="blue">{relTypeNames[row.relationship_type_id] ?? row.relationship_type_id}</Tag>
                        ),
                      },
                      {
                        title: "目标实体",
                        dataIndex: "target_entity_id",
                        render: (_, row) => entityNames[row.target_entity_id] ?? row.target_entity_id,
                      },
                      { title: "描述", dataIndex: "description", ellipsis: true },
                      {
                        title: "固定观测",
                        dataIndex: "is_pinned",
                        render: (_, row) => (row.is_pinned ? <Tag color="gold">已固定</Tag> : <Tag>未固定</Tag>),
                      },
                      { title: "Revision", dataIndex: "revision" },
                    ]}
                  />
                ),
              },
            ]}
          />
        </>
      ) : null}
      <EntityDetailDrawer
        env={env}
        entity={detail}
        entityType={entityType}
        canManage={canManage}
        onClose={() => setDetail(undefined)}
        onEntityChanged={async () => {
          if (entityType) await loadEntities(entityType);
        }}
      />
    </Drawer>
  );
}
