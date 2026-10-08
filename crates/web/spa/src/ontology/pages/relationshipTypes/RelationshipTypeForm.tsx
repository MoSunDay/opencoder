import { ModalForm } from "../../ui/ModalForm";
import { SelectField, TextField, TextAreaField } from "../../ui";
import { Alert, App, Button } from "antd";
import { PlusOutlined } from "@ant-design/icons";
import { api } from "../../api";
import { entityTypeOptions, searchableOptions } from "../../forms/selectOptions";
import type { EntityType, RelationshipType } from "../../types";

type Values = { key: string; name: string; description?: string; source_entity_type_id: string | null; target_entity_type_ids: string[] };
type Props = { env: string; types: EntityType[]; item?: RelationshipType; onSaved: () => Promise<void> };

export function relationshipScope(item: RelationshipType) {
  return { source_entity_type_id: item.source_entity_type_id ?? null, target_entity_type_ids: item.target_entity_type_ids ?? [] };
}

export default function RelationshipTypeForm({ env, types, item, onSaved }: Props) {
  const { message } = App.useApp();
  const fixedScope = Boolean(item && (item.is_system || !item.source_entity_type_id));
  const options = entityTypeOptions(types);
  return <ModalForm<Values>
    title={item ? "编辑关系类型" : "新增关系类型"}
    trigger={item ? <Button type="link">编辑</Button> : <Button type="primary" icon={<PlusOutlined />}>新增</Button>}
    modalProps={{ destroyOnHidden: true }}
    initialValues={item ? { name: item.name, description: item.description, ...relationshipScope(item) } : undefined}
    onFinish={async (values) => {
      const scope = fixedScope && item ? relationshipScope(item) : {
        source_entity_type_id: values.source_entity_type_id,
        target_entity_type_ids: values.target_entity_type_ids,
      };
      if (item) {
        await api.updateRelationshipType(env, item.id, { ...values, ...scope, is_deleted: false, expected_revision: item.revision });
      } else {
        await api.createRelationshipType(env, { ...values, source_entity_type_id: values.source_entity_type_id! });
      }
      message.success(item ? "关系类型已更新" : "关系类型已创建");
      await onSaved();
      return true;
    }}>
    {!item && <TextField name="key" label="类型 Key" rules={[{ required: true, whitespace: true }]} />}
    <TextField name="name" label="名称" rules={[{ required: true, whitespace: true }]} />
    {fixedScope && !item?.source_entity_type_id ? <Alert type="info" message="全局关系保留原有端点范围" style={{ marginBottom: 16 }} /> : <>
      <SelectField name="source_entity_type_id" label="源实体类型" options={options}
        showSearch fieldProps={searchableOptions} disabled={fixedScope} rules={[{ required: true }]}
        placeholder="搜索并选择源实体类型" />
      <SelectField mode="multiple" name="target_entity_type_ids" label="允许目标类型" options={options}
        showSearch fieldProps={searchableOptions} disabled={fixedScope} rules={[{ required: true }]}
        placeholder="搜索并选择允许的目标类型" />
    </>}
    <TextAreaField name="description" label="描述" />
  </ModalForm>;
}
