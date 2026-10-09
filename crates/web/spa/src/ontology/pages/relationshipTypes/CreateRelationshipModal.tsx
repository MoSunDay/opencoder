import { ModalForm } from "../../ui/ModalForm";
import { SelectField, TextAreaField } from "../../ui";
import { Alert, App, Button } from "antd";
import { useMemo, useState } from "react";
import { entityOptions, searchableOptions } from "../../forms/selectOptions";
import { api } from "../../api";
import type { Entity, EntityType, RelationshipType } from "../../types";
import { useResource } from "../admin/useResource";

type Props = {
  env: string;
  relationshipType: RelationshipType;
  entityTypes: EntityType[];
  entities?: Entity[];
  onCreated: () => Promise<void>;
};

export default function CreateRelationshipModal({ env, relationshipType, entityTypes, entities, onCreated }: Props) {
  const { message } = App.useApp();
  const [open, setOpen] = useState(false);
  const resource = useResource(`${env}/${open}`, async () => open && !entities ? (await api.entities(env)).items : []);
  const candidates = entities ?? resource.data ?? [];
  const typeNames = useMemo(() => Object.fromEntries(entityTypes.map((item) => [item.id, item.name])), [entityTypes]);
  const typeName = (id: string) => typeNames[id] ?? id;
  const sourceOptions = entityOptions(candidates
    .filter((entity) => !relationshipType.source_entity_type_id || entity.entity_type_id === relationshipType.source_entity_type_id)
    , entityTypes);
  const targetOptions = entityOptions(candidates
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
    onOpenChange={setOpen}
    submitter={{ submitButtonProps: { disabled: !entities && (resource.loading || Boolean(resource.error)) } }}
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
    <Alert type="info" showIcon message={constraint} style={{ marginBottom: 16 }} />
    {resource.error ? <Alert type="error" showIcon message="实体候选加载失败" description={resource.error} action={<Button onClick={() => void resource.reload()}>重试</Button>} /> : null}
    <SelectField name="source_entity_id" label="源实体" rules={[{ required: true }]} showSearch
      fieldProps={{ ...searchableOptions, loading: !entities && resource.loading }}
      placeholder={sourceOptions.length ? "请选择源实体" : "该类型下暂无实体"} options={sourceOptions} />
    <SelectField name="target_entity_id" label="目标实体" rules={[{ required: true }]} showSearch
      fieldProps={{ ...searchableOptions, loading: !entities && resource.loading }}
      placeholder={targetOptions.length ? "请选择目标实体" : "该类型下暂无实体"} options={targetOptions} />
    <TextAreaField name="description" label="关系描述" />
  </ModalForm>;
}
