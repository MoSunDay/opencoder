import { ModalForm, TextAreaField } from "../../ui";
import { App, Button, Descriptions, Drawer, Popconfirm, Space, Tag } from "antd";
import { api } from "../../api";
import type { Entity, Relationship, RelationshipType } from "../../types";

type Props = {
  env: string;
  relationship?: Relationship;
  entities: Entity[];
  relationshipTypes: RelationshipType[];
  canManage: boolean;
  onClose: () => void;
  onChanged: () => Promise<void>;
};

export default function RelationshipDrawer({
  env,
  relationship,
  entities,
  relationshipTypes,
  canManage,
  onClose,
  onChanged,
}: Props) {
  const { message } = App.useApp();
  const entityNames = Object.fromEntries(entities.map((item) => [item.id, item.name]));
  const typeNames = Object.fromEntries(relationshipTypes.map((item) => [item.id, item.name]));

  return (
    <Drawer
      title="关系详情"
      size={620}
      open={Boolean(relationship)}
      onClose={onClose}
      destroyOnHidden
      extra={
        relationship && canManage ? (
          <Space>
            <ModalForm
              title="编辑关系描述"
              trigger={<Button type="primary">编辑</Button>}
              initialValues={{ description: relationship.description }}
              onFinish={async (values) => {
                await api.updateRelationship(env, relationship.id, {
                  description: values.description,
                  is_deleted: false,
                  expected_revision: relationship.revision,
                });
                message.success("关系描述已更新");
                await onChanged();
                return true;
              }}
            >
              <TextAreaField name="description" label="关系描述" />
            </ModalForm>
            <Popconfirm
              title="仅软删除，历史数据保留"
              onConfirm={async () => {
                await api.updateRelationship(env, relationship.id, {
                  description: relationship.description,
                  is_deleted: true,
                  expected_revision: relationship.revision,
                });
                message.success("关系已软删除");
                await onChanged();
              }}
            >
              <Button danger>软删除</Button>
            </Popconfirm>
          </Space>
        ) : null
      }
    >
      {relationship ? (
        <Descriptions
          bordered
          column={1}
          items={[
            { key: "source", label: "源实体", children: entityNames[relationship.source_entity_id] },
            {
              key: "type",
              label: "关系类型",
              children: <Tag color="blue">{typeNames[relationship.relationship_type_id]}</Tag>,
            },
            { key: "target", label: "目标实体", children: entityNames[relationship.target_entity_id] },
            { key: "description", label: "描述", children: relationship.description || "-" },
            { key: "revision", label: "Revision", children: relationship.revision },
          ]}
        />
      ) : null}
    </Drawer>
  );
}
