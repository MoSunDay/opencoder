import InspectLink from "../../navigation/InspectLink";
import { ArrowDownOutlined, EditOutlined, MoreOutlined } from "@ant-design/icons";
import { TextAreaField } from "../../ui";
import { Alert, App, Button, Collapse, Descriptions, Drawer, Dropdown, Grid, Space, Tag, Typography } from "antd";
import { useEffect, useState } from "react";
import { api } from "../../api";
import type { Entity, Relationship, RelationshipType } from "../../types";
import { ModalForm } from "../../ui/ModalForm";
import { useDraftGuard } from "../../navigation/DraftGuard";

type Props = { env: string; relationship?: Relationship; entities: Entity[]; relationshipTypes: RelationshipType[];
  canManage: boolean; onClose: () => void; onChanged: () => Promise<void>; onEntity?: (id: string) => void; onBack?: () => void };
export default function RelationshipDrawer({ env, relationship, entities, relationshipTypes, canManage, onClose, onChanged, onEntity, onBack }: Props) {
  const { message, modal } = App.useApp();
  const screens = Grid.useBreakpoint();
  const [error, setError] = useState("");
  const [deleting, setDeleting] = useState(false);
  const guard = useDraftGuard(false, deleting);
  useEffect(() => { setError(""); }, [relationship?.id]);
  const entityNames = Object.fromEntries(entities.map((item) => [item.id, item.name]));
  const typeNames = Object.fromEntries(relationshipTypes.map((item) => [item.id, item.name]));
  const remove = () => {
    if (!relationship) return;
    modal.confirm({ title: "删除这条关系？", content: "历史记录将保留。", okText: "删除", okButtonProps: { danger: true },
      onOk: async () => {
        setDeleting(true); setError("");
        try {
          await api.updateRelationship(env, relationship.id, { description: relationship.description, is_deleted: true, expected_revision: relationship.revision });
          message.success("关系已删除"); await onChanged(); onClose();
        } catch (reason) { setError(reason instanceof Error ? reason.message : "删除失败"); throw reason; }
        finally { setDeleting(false); }
      },
    });
  };
  return <Drawer title="关系详情" width={screens.md ? 640 : "100%"} open={Boolean(relationship)} onClose={() => guard.run(onClose)} destroyOnHidden
    extra={relationship && canManage ? <Space>
      <ModalForm key={`${relationship.id}/${relationship.revision}`} title="编辑关系描述" trigger={<Button icon={<EditOutlined />}>编辑</Button>}
        initialValues={{ description: relationship.description }} onFinish={async (values) => {
          try {
            await api.updateRelationship(env, relationship.id, { description: values.description, is_deleted: false, expected_revision: relationship.revision });
            message.success("关系描述已更新"); await onChanged(); return true;
          } catch (reason) { message.error(reason instanceof Error ? reason.message : "保存失败"); return false; }
        }}><TextAreaField name="description" label="关系描述" /></ModalForm>
      <Dropdown menu={{ items: [{ key: "delete", label: "删除关系", danger: true }], onClick: remove }}>
        <Button aria-label="更多关系操作" icon={<MoreOutlined />} loading={deleting} />
      </Dropdown>
    </Space> : null}>
    {error ? <Alert type="error" showIcon message={error} /> : null}
    {onBack ? <Button style={{ marginBottom: 16 }} onClick={() => guard.run(onBack)}>返回上一项</Button> : null}
    {relationship ? <>
      <Space direction="vertical" align="center" size="middle" style={{ width: "100%", marginBottom: 24, textAlign: "center" }}>
        <InspectLink onClick={() => onEntity && guard.run(() => onEntity(relationship.source_entity_id))}>{entityNames[relationship.source_entity_id] || relationship.source_entity_id}</InspectLink>
        <Space><ArrowDownOutlined /><Tag color="blue">{typeNames[relationship.relationship_type_id]}</Tag></Space>
        <InspectLink onClick={() => onEntity && guard.run(() => onEntity(relationship.target_entity_id))}>{entityNames[relationship.target_entity_id] || relationship.target_entity_id}</InspectLink>
      </Space>
      <Descriptions column={1} items={[{ key: "description", label: "描述", children: relationship.description || "尚未填写描述" }]} />
      <Collapse size="small" items={[{ key: "metadata", label: "记录信息", children: <Descriptions column={1} items={[
        { key: "id", label: "ID", children: <Typography.Text copyable>{relationship.id}</Typography.Text> },
        { key: "revision", label: "版本", children: relationship.revision },
      ]} /> }]} />
    </> : null}
  </Drawer>;
}
