import { Alert, Descriptions, Drawer, Empty, List, Space, Tabs, Tag, Typography } from "antd";
import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../../api";
import type { AttributeDefinition, Entity, EntityType } from "../../types";
import EntityActions from "../EntityActions";
import AttributeEditor from "../AttributeEditor";
import {
  formatValue,
  type StructuredAttributeRow,
} from "./values";
import TextAttributeSection, { type TextAttribute } from "./TextAttributeSection";

type DetailTab = "basic" | "attributes" | "extension" | "source";
const TABS = [
  { key: "basic", label: "基本信息" },
  { key: "attributes", label: "普通属性" },
  { key: "extension", label: "拓展信息" },
  { key: "source", label: "来源" },
];

type Props = {
  env: string;
  entity?: Entity;
  entityType?: EntityType;
  canManage: boolean;
  onClose: () => void;
  onEntityChanged: (entity: Entity) => Promise<void>;
};

function StructuredAttributeList({
  env,
  entity,
  definitions,
  rows,
  canManage,
  onDone,
}: {
  env: string;
  entity: Entity;
  definitions: AttributeDefinition[];
  rows: StructuredAttributeRow[];
  canManage: boolean;
  onDone: () => void;
}) {
  const valueOf = (id: string) => rows.find((row) => row.attribute_definition_id === id)?.value;
  return (
    <List
      header={<Typography.Text strong>普通 SQL 属性</Typography.Text>}
      dataSource={definitions.filter((definition) => definition.kind !== "text")}
      renderItem={(definition) => (
        <List.Item key={definition.id}>
          <div style={{ width: "100%" }}>
            <Space orientation="vertical" style={{ width: "100%" }} size={2}>
              <Space>
                <Typography.Text strong>{definition.name}</Typography.Text>
                <Tag>{definition.kind}</Tag>
                {definition.required ? <Tag color="red">必填</Tag> : null}
                {canManage ? <AttributeEditor env={env} entity={entity} definition={definition} row={rows.find((row) => row.attribute_definition_id === definition.id)} onDone={onDone} /> : null}
              </Space>
              <Typography.Text type="secondary" style={{ fontSize: 12 }}>
                {definition.description || `键：${definition.attribute_key}`}
              </Typography.Text>
              <Typography.Text>{formatValue(valueOf(definition.id))}</Typography.Text>
            </Space>
          </div>
        </List.Item>
      )}
    />
  );
}

export default function EntityDetailDrawer({
  env,
  entity,
  entityType,
  canManage,
  onClose,
  onEntityChanged,
}: Props) {
  const [definitions, setDefinitions] = useState<AttributeDefinition[]>([]);
  const [rows, setRows] = useState<StructuredAttributeRow[]>([]);
  const [detailEntity, setDetailEntity] = useState<Entity>();
  const [textAttributes, setTextAttributes] = useState<TextAttribute[]>([]);
  const [actions, setActions] = useState<{ id: string; operation_type: string; operation: string }[]>([]);
  const [textValues, setTextValues] = useState<Record<string, string>>({});
  const [textBodies, setTextBodies] = useState<Record<string, string>>({});
  const [error, setError] = useState("");
  const [activeTab, setActiveTab] = useState<DetailTab>("basic");
  const selection = useRef("");
  selection.current = `${env}/${entity?.id ?? ""}`;
  const generation = useRef(0);

  const reload = useCallback(
    async (item: Entity) => {
      const key = `${env}/${item.id}`;
      if (!entityType || selection.current !== key) return;
      const requestGeneration = ++generation.current;
      const current = () => selection.current === key && generation.current === requestGeneration;
      setError("");
      const [typeResult, detail] = await Promise.all([
        api.attributes(env, entityType.id),
        api.entity(env, item.id),
      ]);
      const values: Record<string, string> = {};
      const bodies: Record<string, string> = {};
      for (const attribute of detail.text_attributes ?? []) {
        const id = attribute.definition.id;
        const nfs = attribute.definition.storage_mode === "nfs_path";
        values[id] = nfs && attribute.current?.format === "nfs_path" ? attribute.current.content_path ?? "" : "";
        if (attribute.current?.revision) {
          try {
            bodies[id] = (await api.textContent(env, detail.item.id, id, attribute.current.revision)).content;
            if (!nfs) values[id] = bodies[id];
          } catch (reason) {
            if (current()) setError(reason instanceof Error ? reason.message : "文本读取失败");
          }
        }
      }
      if (!current()) return;
      setDefinitions(typeResult.items.filter((definition) => definition.kind !== "vector"));
      setDetailEntity(detail.item);
      setRows((detail.structured_attributes as StructuredAttributeRow[]) ?? []);
      setTextAttributes(detail.text_attributes ?? []);
      setActions(detail.actions ?? []);
      setTextValues(values);
      setTextBodies(bodies);
    },
    [env, entityType],
  );

  useEffect(() => {
    setActiveTab("basic");
  }, [env, entity?.id]);

  useEffect(() => {
    if (!entity || !entityType) {
      setDefinitions([]);
      setRows([]);
      setDetailEntity(undefined);
      return;
    }
    let active = true;
    void reload(entity).catch((reason) => { if (active) setError(reason instanceof Error ? reason.message : "详情加载失败"); });
    return () => { active = false; generation.current += 1; };
  }, [entity, entityType, reload]);

  const sourceAttributes = textAttributes.filter((item) => item.definition.attribute_role === "source");
  const extensionAttributes = textAttributes.filter((item) => item.definition.attribute_role === "ext");
  const ordinaryTextAttributes = textAttributes.filter((item) => !["source", "ext"].includes(item.definition.attribute_role ?? "custom"));
  const ordinaryDefinitions = definitions.filter((definition) => definition.kind !== "text" && !["source", "ext"].includes(definition.attribute_role ?? "custom"));
  const updateText = (id: string, value: string) => setTextValues((current) => ({ ...current, [id]: value }));
  const saveText = async (item: TextAttribute) => {
    if (!entity) return;
    try {
      const value = textValues[item.definition.id] ?? "";
      if (item.definition.storage_mode === "nfs_path") {
        await api.setNfsPath(env, entity.id, item.definition.id, { path: value, expected_revision: item.current?.revision ?? 0 });
      } else {
        await api.setText(env, entity.id, item.definition.id, { format: "md", content: value, expected_revision: item.current?.revision ?? 0 });
      }
      await reload(entity);
      await onEntityChanged(entity);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "保存失败");
    }
  };
  const textSection = (items: TextAttribute[]) => <TextAttributeSection items={items} values={textValues} bodies={textBodies}
    canManage={canManage} onChange={updateText} onSave={saveText} />;

  return (
    <Drawer
      title={entity?.name}
      extra={canManage && detailEntity && !detailEntity.is_deleted && detailEntity.id === entity?.id
        ? <EntityActions key={`${detailEntity.id}/${detailEntity.revision}`} env={env} entity={detailEntity}
          onDone={async (item) => { await reload(item); await onEntityChanged(item); }} /> : null}
      width="75%"
      open={Boolean(entity)}
      onClose={onClose}
      destroyOnHidden
    >
      {error ? <Alert type="error" title={error} showIcon /> : null}
      {entity && entityType && detailEntity?.id === entity.id ? (
        <>
          <Tabs activeKey={activeTab} onChange={(key) => setActiveTab(key as DetailTab)} items={TABS} />
          {activeTab === "basic" ? <Descriptions
            bordered
            column={2}
            items={[
              { key: "id", label: "ID", children: entity.id },
              { key: "type", label: "类型", children: entityType.name },
              { key: "revision", label: "Revision", children: detailEntity?.revision ?? entity.revision },
              { key: "description", label: "描述", children: detailEntity?.description ?? entity.description },
              {
                key: "actions",
                label: "支持的 Action",
                children:
                  actions.length > 0 ? (
                    <Space wrap>
                      {actions.map((action) => (
                        <Tag color="blue" key={action.id}>
                          {action.operation_type}: {action.operation}
                        </Tag>
                      ))}
                    </Space>
                  ) : (
                    <Typography.Text type="secondary">无</Typography.Text>
                  ),
              },
            ]}
          /> : null}
          {activeTab === "attributes" && (ordinaryDefinitions.length || ordinaryTextAttributes.length) ? <>
          {ordinaryDefinitions.length ? (
            <StructuredAttributeList env={env} entity={entity} definitions={ordinaryDefinitions} rows={rows} canManage={canManage} onDone={() => { void reload(entity).then(() => onEntityChanged(entity)); }} />
          ) : null}
          {textSection(ordinaryTextAttributes)}
          </> : null}
          {activeTab === "attributes" && !ordinaryDefinitions.length && !ordinaryTextAttributes.length ? <Empty description="暂无普通属性" /> : null}
          {activeTab === "extension" ? extensionAttributes.length ? textSection(extensionAttributes) : <Empty description="暂无拓展信息" /> : null}
          {activeTab === "source" ? <>
            <Typography.Text type="secondary">{sourceAttributes.some((item) => (item.current?.bytes ?? 0) > 0) ? "已维护" : "待完善"}</Typography.Text>
            {sourceAttributes.length ? textSection(sourceAttributes) : <Empty description="暂无来源定义" />}
          </> : null}
        </>
      ) : (
        <Empty />
      )}
    </Drawer>
  );
}
