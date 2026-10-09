import { Alert, Form, Input, Modal, Select, Space, Table, Tag } from 'antd';
import { useEffect, useState } from 'react';
import { apiPost } from '../../api.js';
import { EditButton } from '../../ui/permissions.jsx';
import { rosterRowsOf } from './model.js';

export function TeamEditor({ open, team, agents, onClose, onSaved }) {
  const [form] = Form.useForm();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  useEffect(() => {
    if (!open) return;
    form.resetFields(); setError('');
    if (team) form.setFieldsValue({ name: team.name, captain: team.captain, members: (team.members || []).map((m) => m.agent) });
  }, [open, team, form]);
  const captain = Form.useWatch('captain', form), members = Form.useWatch('members', form);
  const rosterRows = rosterRowsOf(captain, members, agents);
  const options = agents.map((a) => ({ value: a.agent, label: a.agent }));
  const save = async (values) => {
    setBusy(true); setError('');
    const payload = { name: values.name, captain: values.captain, members: [...new Set([values.captain, ...(values.members || [])])].map((agent) => ({ agent })) };
    try { await apiPost('/api/teams', payload); onSaved(); }
    catch (failure) { setError(failure.message); }
    finally { setBusy(false); }
  };
  return (
    <Modal open={open} onCancel={() => { if (!busy) onClose(); }} title="Team 成员" footer={null} width={720}>
      {error && <Alert type="error" showIcon title={error} />}
      <Form form={form} disabled={busy} onFinish={save} layout="vertical">
        <Space wrap>
          <Form.Item name="name" label="Team 名称" rules={[{ required: true }, { pattern: /^[a-z0-9][a-z0-9-]{0,63}$/, message: '使用小写字母、数字和连字符' }]}><Input /></Form.Item>
          <Form.Item name="captain" label="队长" rules={[{ required: true }]}><Select showSearch optionFilterProp="label" options={options} style={{ width: 200 }} placeholder="选择队长" /></Form.Item>
        </Space>
        <Form.Item name="members" label="队员"><Select mode="multiple" showSearch optionFilterProp="label" options={options} placeholder="选择 Team 成员" /></Form.Item>
        {rosterRows.length > 0 && <Table
          className="team-roster-table"
          rowKey="key"
          size="small"
          pagination={false}
          scroll={{ x: 640 }}
          dataSource={rosterRows}
          onRow={(row) => ({ 'data-agent': row.name })}
          columns={[
            {
              title: '名称', dataIndex: 'name', key: 'name', width: 170, ellipsis: true,
              render: (value) => <span className="team-roster-cell" title={value}>{value}</span>,
            },
            { title: '角色', dataIndex: 'role', key: 'role', width: 96, render: (value) => value === '队长' ? <Tag color="gold">{value}</Tag> : <Tag>{value}</Tag> },
            {
              title: '描述', dataIndex: 'description', key: 'description', width: 374, ellipsis: true,
              render: (value) => <span className="team-roster-cell" title={value}>{value}</span>,
            },
          ]}
        />}
        <EditButton type="primary" htmlType="submit" loading={busy} style={{ marginTop: 16 }}>保存 Team</EditButton>
      </Form>
    </Modal>
  );
}
