import { EditOutlined } from "@ant-design/icons";
import { TextField, TextAreaField } from "../ui";
import { ModalForm } from "../ui/ModalForm";
import { App, Button } from "antd";
import { api } from "../api";
import type { Entity } from "../types";

type Props = { env: string; entity: Entity; onDone: (entity: Entity) => Promise<void> };
export default function EntityActions({ env, entity, onDone }: Props) {
  const { message } = App.useApp();
  return <ModalForm title="编辑实体" trigger={<Button icon={<EditOutlined />}>编辑</Button>} initialValues={{ name: entity.name, description: entity.description }} onFinish={async values => {
    const result = await api.updateEntity(env, entity.id, { ...values, is_deleted: false, expected_revision: entity.revision });
    message.success("实体已更新"); await onDone(result.item); return true;
  }}>
    <TextField name="name" label="名称" rules={[{ required: true }]} />
    <TextAreaField name="description" label="描述" />
  </ModalForm>;
}
