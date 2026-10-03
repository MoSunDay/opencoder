import { ModalForm, SelectField, TextAreaField } from "../../ui";
import { Alert, App, Button } from "antd";
import { useMemo } from "react";
import { entityOptions, searchableOptions } from "../../forms/selectOptions";
import { api } from "../../api";
import type { Entity, EntityType, RelationshipType } from "../../types";

type Props = {
  env: string;
  relationshipType: RelationshipType;
  entityTypes: EntityType[];
  entities: Entity[];
  onCreated: () => Promise<void>;
};

export default function CreateRelationshipModal({ env, relationshipType, entityTypes, entities, onCreated }: Props) {
  const { message } = App.useApp();
  const typeNames = useMemo(() => Object.fromEntries(entityTypes.map((item) => [item.id, item.name])), [entityTypes]);
  const typeName = (id: string) => typeNames[id] ?? id;
  const sourceOptions = entityOptions(entities
    .filter((entity) => !relationshipType.source_entity_type_id || entity.entity_type_id === relationshipType.source_entity_type_id)
    , entityTypes);
  const targetOptions = entityOptions(entities
    .filter((entity) => !(relationshipType.target_entity_type_ids?.length) || relationshipType.target_entity_type_ids.includes(entity.entity_type_id))
    , entityTypes);
  const declared = Boolean(relationshipType.source_entity_type_id) || Boolean(relationshipType.target_entity_type_ids?.length);
  const constraint = declared
    ? `端点约束：${relationshipType.source_entity_type_id ? typeName(relationshipType.source_entity_type_id) : "不限"} → ${
      relationshipType.target_entity_type_ids?.length
        ? relationshipType.target_entity_type_ids.map(typeName).join("、")
        : "不限"}`
    : "该类型未声明端点约束，提交后以后端校验为准";
  return <ModalForm<{ source_entity_id: string; target_entity_id: string; description?: string }>
    title={`创建关系 · ${relationshipType.name}`}
    trigger={<Button type="link">创建关系</Button>}
    modalProps={{ destroyOnHidden: true }}
    onFinish={async (values) => {
      try {
        await api.createRelationship(env, {
          relationship_type_id: relationshipType.id,
          source_entity_id: values.source_entity_id,
          target_entity_id: values.target_entity_id,
          description: values.description,
        });
        message.success("关系已创建");
        await onCreated();
        return true;
      } catch (reason) {
        message.error(reason instanceof Error ? reason.message : "创建关系失败");
        return false;
      }
    }}>
    <Alert type="info" showIcon title={constraint} style={{ marginBottom: 16 }} />
    <SelectField name="source_entity_id" label="源实体" rules={[{ required: true }]} showSearch
      fieldProps={searchableOptions}
      placeholder={sourceOptions.length ? "请选择源实体" : "该类型下暂无实体"} options={sourceOptions} />
    <SelectField name="target_entity_id" label="目标实体" rules={[{ required: true }]} showSearch
      fieldProps={searchableOptions}
      placeholder={targetOptions.length ? "请选择目标实体" : "该类型下暂无实体"} options={targetOptions} />
    <TextAreaField name="description" label="关系描述" />
  </ModalForm>;
}
