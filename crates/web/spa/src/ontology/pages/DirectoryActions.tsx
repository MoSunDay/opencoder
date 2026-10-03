import { DeleteOutlined, EditOutlined, SwapOutlined } from "@ant-design/icons";
import { ModalForm, SelectField, TextField, TextAreaField } from "../ui";
import { App, Button, Popconfirm, Space } from "antd";
import { api } from "../api";
import type { DirectoryItem } from "../types";

const ROOT_DIRECTORY = "00000000-0000-4000-8000-000000000002";
type Props = { env: string; selected?: DirectoryItem; directories: DirectoryItem[]; onDone: () => Promise<void> };

export default function DirectoryActions({ env, selected, directories, onDone }: Props) {
  const { message } = App.useApp();
  if (!selected) return null;
  const isRoot = selected.id === ROOT_DIRECTORY;
  return <Space size={0}>
    <ModalForm title="编辑目录" trigger={<Button type="text" icon={<EditOutlined />} />} initialValues={{ name: selected.name, description: selected.description }} onFinish={async values => {
      await api.updateDirectory(env, selected.id, { ...values, is_deleted: false, expected_revision: selected.revision });
      message.success("目录已更新"); await onDone(); return true;
    }}>
      <TextField name="name" label="名称" rules={[{ required: true }]} />
      <TextAreaField name="description" label="描述" />
    </ModalForm>
    {!isRoot && <ModalForm title="移动目录" trigger={<Button type="text" icon={<SwapOutlined />} />} initialValues={{ parent_id: selected.parent_id || ROOT_DIRECTORY }} onFinish={async values => {
      await api.moveDirectory(env, selected.id, values.parent_id); message.success("目录已移动"); await onDone(); return true;
    }}>
      <SelectField name="parent_id" label="新父目录" showSearch options={directories.filter(item => !item.is_deleted && item.id !== selected.id).map(item => ({ value: item.id, label: item.name }))} rules={[{ required: true }]} />
    </ModalForm>}
    {!isRoot && <Popconfirm title="仅空目录可软删除，历史数据保留" onConfirm={async () => {
      await api.updateDirectory(env, selected.id, { name: selected.name, description: selected.description, is_deleted: true, expected_revision: selected.revision });
      message.success("目录已软删除"); await onDone();
    }}><Button type="text" danger icon={<DeleteOutlined />} /></Popconfirm>}
  </Space>;
}
