import { PlusOutlined } from "@ant-design/icons";
import {
  ModalForm,
  NumberField,
  SelectField,
  SwitchField,
  TextField,
  TextAreaField,
} from "../../ui";
import { Alert, App, Button, Form } from "antd";
import { useEffect, useRef, useState } from "react";
import { entityTypeOptions, searchableOptions } from "../../forms/selectOptions";
import { api } from "../../api";
import { requestId } from "./requestId";
import type { AttributeDefinition, Entity, EntityType } from "../../types";

type Props = {
  env: string;
  types: EntityType[];
  directoryId?: string;
  onCreated: (entity: Entity) => Promise<void>;
};
export default function CreateEntityModal({
  env,
  types,
  directoryId,
  onCreated,
}: Props) {
  const { message } = App.useApp();
  const [form] = Form.useForm();
  const typeId = Form.useWatch("entity_type_id", form);
  const [definitions, setDefinitions] = useState<AttributeDefinition[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const request = useRef<{ body: string; id: string }>();
  useEffect(() => {
    let active = true;
    setDefinitions([]);
    form.setFieldsValue({ attributes: {}, source: undefined, ext: undefined });
    setError("");
    if (!typeId) return;
    setLoading(true);
    void api
      .attributes(env, typeId)
      .then(({ items }) => {
        if (active) setDefinitions(items.filter((item) => !item.is_deleted));
      })
      .catch((reason) => {
        if (active)
          setError(reason instanceof Error ? reason.message : "属性加载失败");
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => {
      active = false;
    };
  }, [env, typeId, form]);
  const ext = definitions.find((item) => item.attribute_role === "ext");
  return (
    <ModalForm
      title="新增实体"
      form={form}
      trigger={
        <Button type="primary" icon={<PlusOutlined />}>
          新增实体
        </Button>
      }
      modalProps={{ destroyOnHidden: true }}
      submitter={{
        submitButtonProps: { disabled: loading || Boolean(error) || !ext },
      }}
      onFinish={async (values) => {
        try {
          const attributes: Record<string, unknown> = {};
          for (const definition of definitions.filter(
            (item) => item.attribute_role === "custom"
          )) {
            const value = values.attributes?.[String(definition.id)];
            if (value !== undefined && value !== "")
              attributes[definition.id] =
                definition.kind === "json" ? JSON.parse(value) : value;
          }
          const payload = { ...values, attributes, directory_id: directoryId };
          const body = JSON.stringify(payload);
          if (request.current?.body !== body)
            request.current = { body, id: requestId(crypto.getRandomValues(new Uint8Array(16))) };
          const result = await api.createEntity(env, {
            ...payload,
            request_id: request.current.id,
          });
          await onCreated(result.item);
          message.success("实体已创建");
          request.current = undefined;
          form.resetFields();
          return true;
        } catch (reason) {
          message.error(reason instanceof Error ? reason.message : "创建失败");
          return false;
        }
      }}
    >
      <SelectField
        name="entity_type_id"
        label="实体类型"
        showSearch
        fieldProps={searchableOptions}
        options={entityTypeOptions(types.filter((type) => !type.is_system))}
        rules={[{ required: true }]}
      />
      <TextField
        name="name"
        label="名称"
        rules={[{ required: true, whitespace: true }]}
      />
      <TextAreaField name="description" label="描述" />
      {error ? <Alert type="error" title={error} /> : null}
      <TextAreaField
        name="source"
        label="来源"
        disabled={loading || !typeId}
        rules={[{ required: true, whitespace: true }]}
      />
      {ext?.storage_mode === "nfs_path" ? (
        <TextField
          name="ext"
          label="扩展文件 NFS path"
          placeholder="受控根目录内的相对路径"
          rules={[{ required: true, whitespace: true }]}
        />
      ) : (
        <TextAreaField
          name="ext"
          label="扩展信息（Markdown）"
          disabled={loading || !typeId}
          rules={[{ required: true, whitespace: true }]}
        />
      )}
      {definitions
        .filter(
          (item) => item.attribute_role === "custom" && item.kind !== "vector"
        )
        .map((item) => {
          const props = {
            name: ["attributes", String(item.id)],
            label: item.name,
            tooltip: item.description,
            rules: [{ required: item.required }],
          };
          if (item.kind === "boolean")
            return (
              <SwitchField key={item.id} {...props} initialValue={false} />
            );
          if (item.kind === "integer" || item.kind === "float")
            return (
              <NumberField
                key={item.id}
                {...props}
                fieldProps={{
                  precision: item.kind === "integer" ? 0 : undefined,
                }}
              />
            );
          return (
            <TextAreaField
              key={item.id}
              {...props}
              placeholder={
                item.kind === "json"
                  ? "JSON"
                  : item.kind === "datetime"
                  ? "RFC3339 时间"
                  : undefined
              }
            />
          );
        })}
    </ModalForm>
  );
}
