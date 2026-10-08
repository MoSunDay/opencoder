import { MoreOutlined } from "@ant-design/icons";
import { App, Button, Dropdown } from "antd";
import { useState } from "react";
import { useDraftGuard } from "../../navigation/DraftGuard";

export default function DangerMenu({ label, description, onConfirm }: { label: string; description: string; onConfirm: () => Promise<void> }) {
  const { modal, message } = App.useApp();
  const [saving, setSaving] = useState(false);
  useDraftGuard(false, saving);
  return <span onClick={(event) => event.stopPropagation()}><Dropdown menu={{ items: [{ key: "remove", label, danger: true }], onClick: () => {
    modal.confirm({ title: `确认${label}？`, content: description, okText: label, cancelText: "取消", okButtonProps: { danger: true },
      onOk: async () => {
        setSaving(true);
        try { await onConfirm(); } catch (reason) { message.error(reason instanceof Error ? reason.message : "操作失败"); throw reason; }
        finally { setSaving(false); }
      } });
  } }}><Button type="text" aria-label="更多操作" icon={<MoreOutlined />} loading={saving} /></Dropdown></span>;
}
