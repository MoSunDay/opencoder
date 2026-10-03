import { useInitialLoad, LoadFeedback } from "../ui/useInitialLoad";
import { PlusOutlined } from "@ant-design/icons";
import { ModalForm, TextField, TextAreaField, DataTable } from "../ui";
import { App, Button, Popconfirm, Space, Tag } from "antd";
import { useState } from "react";
import { api } from "../api";
import { useEnv } from "../env";
import type { Environment } from "../types";

export default function EnvironmentsPage() {
  const { canManage, env, setEnv, refreshEnvironments } = useEnv();
  const { message } = App.useApp();
  const [items, setItems] = useState<Environment[]>([]);
  const load = async () => setItems((await api.environments(true)).items);
  const status = useInitialLoad(env, load);
  return <><LoadFeedback status={status} /><DataTable<Environment> headerTitle="ENV 数据隔离" loading={status.loading} rowKey="env_key" search={false} dataSource={items}
    toolBarRender={() => canManage ? [<ModalForm key="new" title="新增 ENV" trigger={<Button type="primary" icon={<PlusOutlined />}>新增 ENV</Button>}
      onFinish={async value => { await api.createEnvironment(value); await Promise.all([load(), refreshEnvironments()]); message.success("ENV 已创建并初始化系统类型"); return true; }}>
      <TextField name="key" label="ENV Key" rules={[{ required: true, pattern: /^[a-z][a-z0-9_-]{1,31}$/ }]} />
      <TextField name="name" label="名称" rules={[{ required: true }]} /><TextAreaField name="description" label="描述" />
    </ModalForm>] : []}
    columns={[{ title:"名称",dataIndex:"name" },{ title:"ENV Key",dataIndex:"env_key",render:(_,row)=><Tag color={row.env_key===env?"blue":undefined}>{row.env_key}</Tag> },
      { title:"描述",dataIndex:"description" },{ title:"Revision",dataIndex:"revision" },{ title:"初始化",dataIndex:"initialization_status",render:(_,row)=><Tag color={row.initialization_status==="ready"?"green":row.initialization_status==="failed"?"red":"gold"}>{row.initialization_status}</Tag> },{ title:"状态",render:(_,row)=>row.is_deleted?<Tag>已删除</Tag>:<Tag color="green">有效</Tag> },
      { title:"操作",render:(_,row)=>canManage&&!row.is_deleted?<Space><ModalForm title="编辑 ENV" trigger={<Button type="link">编辑</Button>} initialValues={{name:row.name,description:row.description}} onFinish={async values=>{await api.updateEnvironment(row.env_key,{...values,is_deleted:false,expected_revision:row.revision});await Promise.all([load(),refreshEnvironments()]);message.success("ENV 已更新");return true;}}><TextField name="name" label="名称" rules={[{required:true}]}/><TextAreaField name="description" label="描述"/></ModalForm>{row.env_key!=="debug"&&<Popconfirm title="仅软删除该 ENV，所有业务数据均保留" onConfirm={async()=>{await api.updateEnvironment(row.env_key,{name:row.name,description:row.description,is_deleted:true,expected_revision:row.revision});if(env===row.env_key)setEnv("debug");await Promise.all([load(),refreshEnvironments()]);message.success("ENV 已软删除");}}><Button danger type="link">软删除</Button></Popconfirm>}</Space>:null }]}/></>
}
