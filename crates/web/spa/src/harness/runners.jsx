import { useEvent } from '../ui/editing/useEvent.js';
import { Button, Form, Input, Space, Table, Tag, Typography, message } from 'antd';
import { useCallback, useEffect, useState } from 'react';
import { apiGet, apiPut } from '../api.js';
import { parseEnvs } from './fields.jsx';
import { err } from '../notice.js';

export function RunnerManagement({ onNotice: noticeCallback }) {
  const onNotice = useEvent(noticeCallback);
  const [items, setItems] = useState([]); const [busy, setBusy] = useState(false); const [editing, setEditing] = useState(false);
  const [form] = Form.useForm();
  const load = useCallback(async () => {
    setBusy(true);
    try { setItems((await apiGet('/api/runners')).items); }
    catch (error) { onNotice(err(`读取 Runner 失败：${error.message}`)); }
    finally { setBusy(false); }
  }, [onNotice]);
  useEffect(() => { load(); }, [load]);
  const edit = (entry) => {
    form.resetFields();
    form.setFieldsValue({ name: entry?.name, command: JSON.stringify(entry?.settings?.command || [], null, 2),
      workdir: entry?.settings?.workdir, parent_unit: entry?.settings?.parent_unit,
      files: JSON.stringify(entry?.settings?.files || {}, null, 2),
      envs: Object.entries(entry?.settings?.envs || {}).map(([k, v]) => `${k}=${v}`).join('\n') });
    setEditing(true);
  };
  const save = async (values) => {
    setBusy(true);
    try {
      await apiPut(`/api/runners/${encodeURIComponent(values.name)}`, { command: JSON.parse(values.command),
        workdir: values.workdir, parent_unit: values.parent_unit || null, files: JSON.parse(values.files), envs: parseEnvs(values.envs) });
      message.success('Runner 已保存，新任务使用新版本'); setEditing(false); await load();
    } catch (error) { onNotice(err(`保存 Runner 失败：${error.message}`)); }
    finally { setBusy(false); }
  };
  return <>
    <Space style={{ marginBottom: 16 }}><Button onClick={() => edit(null)}>登记 Runner</Button><Button onClick={load}>刷新</Button></Space>
    <Table rowKey="name" loading={busy} dataSource={items} pagination={false} columns={[
      { title: 'Runner', dataIndex: 'name' }, { title: '版本', render: (_, entry) => <Tag>v{entry.revision}</Tag> },
      { title: '工作目录', render: (_, entry) => entry.settings.workdir },
      { title: '所属服务', render: (_, entry) => entry.settings.parent_unit || '节点进程' },
      { title: '操作', render: (_, entry) => <Button onClick={() => edit(entry)}>编辑</Button> },
    ]} />
    {editing && <Form disabled={busy} form={form} layout="vertical" onFinish={save} style={{ maxWidth: 760, marginTop: 20 }}>
      <Typography.Title level={5}>Runner 执行配置</Typography.Title>
      <Form.Item name="name" label="名称" rules={[{ required: true }]}><Input /></Form.Item>
      <Form.Item name="command" label="执行入口与参数" extra="JSON 数组，第一个元素为节点上的二进制绝对路径。" rules={[{ required: true }]}><Input.TextArea rows={4} /></Form.Item>
      <Form.Item name="workdir" label="工作目录" rules={[{ required: true }]}><Input /></Form.Item>
      <Form.Item name="parent_unit" label="所属节点服务"><Input placeholder="opencoder-agent.service" /></Form.Item>
      <Form.Item name="files" label="安装文件校验清单" extra="JSON 对象：文件绝对路径对应 SHA-256；接收与执行任务时校验版本。" rules={[{ required: true }]}><Input.TextArea rows={6} /></Form.Item>
      <Form.Item name="envs" label="环境变量"><Input.TextArea rows={4} autoComplete="off" spellCheck={false} /></Form.Item>
      <Space><Button type="primary" htmlType="submit" loading={busy}>保存 Runner</Button><Button onClick={() => setEditing(false)}>取消</Button></Space>
    </Form>}
  </>;
}
