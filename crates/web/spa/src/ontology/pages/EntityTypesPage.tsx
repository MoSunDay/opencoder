import { useInitialLoad, LoadFeedback } from "../ui/useInitialLoad";
import { PlusOutlined } from "@ant-design/icons";
import { ModalForm, TextField, TextAreaField, DataTable } from "../ui";
import { App, Button, Popconfirm, Space, Tag } from "antd";
import { useEffect, useState } from "react";
import { api } from "../api";
import { useEnv } from "../env";
import type { EntityType } from "../types";
import EntityTypeDetailDrawer from "./entityTypes/EntityTypeDetailDrawer";

export default function EntityTypesPage() {
  const { env, canManage } = useEnv();
  return <EntityTypesContent key={env} env={env} canManage={canManage} />;
}

function EntityTypesContent({ env, canManage }: { env: string; canManage: boolean }) {
  const { message } = App.useApp();
  const [items, setItems] = useState<EntityType[]>([]);
  const [selected, setSelected] = useState<EntityType>();

  const load = async () => {
    const result = (await api.entityTypes(env, true)).items;
    setItems(result);
    setSelected((current) => current ? result.find((item) => item.id === current.id) : undefined);
  };
  useEffect(() => { setSelected(undefined); }, [env]);
  const status = useInitialLoad(env, load);

  return (
    <>
      <LoadFeedback status={status} /><DataTable<EntityType>
        headerTitle="实体类型"
        loading={status.loading} rowKey="id"
        search={false}
        dataSource={items}
        onRow={(row) => ({ onClick: () => setSelected(row) })}
        toolBarRender={() =>
          canManage
            ? [
                <ModalForm
                  key="new"
                  title="新增实体类型"
                  trigger={<Button type="primary" icon={<PlusOutlined />}>新增</Button>}
                  onFinish={async (value) => {
                    await api.createEntityType(env, value);
                    message.success("实体类型已创建");
                    await load();
                    return true;
                  }}
                >
                  <TextField name="key" label="类型 Key" rules={[{ required: true }]} />
                  <TextField name="name" label="名称" rules={[{ required: true }]} />
                  <TextAreaField name="description" label="描述" />
                </ModalForm>,
              ]
            : []
        }
        columns={[
          { title: "名称", dataIndex: "name" },
          { title: "Key", dataIndex: "type_key", render: (_, row) => <Tag>{row.type_key}</Tag> },
          {
            title: "状态",
            render: (_, row) =>
              row.is_deleted ? <Tag>已删除</Tag> : row.is_system ? <Tag color="blue">系统</Tag> : <Tag color="green">有效</Tag>,
          },
          { title: "Revision", dataIndex: "revision" },
          {
            title: "操作",
            render: (_, row) => (
              <Space>
                <Button type="link" onClick={(event) => { event.stopPropagation(); setSelected(row); }}>
                  详情
                </Button>
                {canManage && !row.is_system && !row.is_deleted && (
                  <Popconfirm
                    title="已有实体保留，仅停用此类型"
                    onConfirm={async () => {
                      await api.updateEntityType(env, row.id, {
                        name: row.name,
                        description: row.description,
                        is_deleted: true,
                        expected_revision: row.revision,
                      });
                      message.success("实体类型已停用");
                      await load();
                    }}
                  >
                    <Button danger type="link">
                      停用
                    </Button>
                  </Popconfirm>
                )}
              </Space>
            ),
          },
        ]}
      />
      <EntityTypeDetailDrawer
        key={`${env}:${selected?.id ?? "closed"}`}
        env={env}
        entityType={selected}
        canManage={canManage}
        onClose={() => setSelected(undefined)}
        onSaved={load}
      />
    </>
  );
}
