import {
  ModalForm,
  NumberField,
  SwitchField,
  TextField,
  TextAreaField,
} from "../ui";
import { App, Button } from "antd";
import { api } from "../api";
import type { AttributeDefinition, Entity } from "../types";

type Row = { value: unknown; revision: number; is_deleted?: boolean };
type Props = {
  env: string;
  entity: Entity;
  definition: AttributeDefinition;
  row?: Row;
  onDone: () => void;
};
export default function AttributeEditor({
  env,
  entity,
  definition,
  row,
  onDone,
}: Props) {
  const { message } = App.useApp();
  const current = row?.is_deleted ? undefined : row?.value;
  const initialValue =
    definition.kind === "json" && current !== undefined
      ? JSON.stringify(current, null, 2)
      : current;
  return (
    <ModalForm
      key={`${entity.id}/${definition.id}/${row?.revision ?? 0}`}
      title={`编辑属性 · ${definition.name}`}
      initialValues={{ value: initialValue }}
      trigger={<Button type="link">编辑</Button>}
      modalProps={{ destroyOnHidden: true }}
      onFinish={async (values) => {
        try {
          const value =
            definition.kind === "json"
              ? JSON.parse(String(values.value))
              : values.value;
          await api.setAttribute(env, entity.id, definition.id, {
            kind: definition.kind,
            value,
            is_deleted: false,
            expected_revision: row?.revision ?? 0,
          });
          message.success("属性已保存");
          onDone();
          return true;
        } catch (reason) {
          message.error(reason instanceof Error ? reason.message : "保存失败");
          return false;
        }
      }}
    >
      {definition.kind === "boolean" ? (
        <SwitchField name="value" label="值" initialValue={false} />
      ) : definition.kind === "integer" || definition.kind === "float" ? (
        <NumberField
          name="value"
          label="值"
          fieldProps={{
            precision: definition.kind === "integer" ? 0 : undefined,
          }}
          rules={[{ required: true }]}
        />
      ) : definition.kind === "datetime" ? (
        <TextField
          name="value"
          label="RFC3339 时间"
          placeholder="2026-08-15T12:00:00+08:00"
          rules={[{ required: true }]}
        />
      ) : (
        <TextAreaField
          name="value"
          label={definition.kind === "json" ? "JSON" : "值"}
          rules={[{ required: true }]}
        />
      )}
    </ModalForm>
  );
}
