// todosTab.jsx — 项目 tab 4「TODO」: flattened table over overview (milestone
// todos + backlog), row ops 生成Plan / 执行 / 详情 / 删除, and a 新建 TODO modal
// (title + milestone Select allowClear + agent + draft). Plan/Execute POSTs
// return 202 {run_id}; the row's 生成Plan jumps straight into the drawer so
// the plan-generation workflow continues without a detour.

import { Button, Form, Input, Modal, Popconfirm, Select, Space, Table, Tooltip, Typography } from 'antd';
import { useState } from 'react';
import { apiDel, apiPost } from '../api.js';
import { TodoStatusTag } from './labels.jsx';

const { TextArea } = Input;
const { Text } = Typography;

const todoPath = (id) => '/api/project/todos/' + encodeURIComponent(id);

/// Flatten overview → rows carrying their milestone context (null ⇒ backlog).
export function flattenTodos(overview) {
  const goals = (overview && overview.goals) || [];
  const backlog = (overview && overview.backlog) || [];
  const rows = [];
  goals.forEach((g) => (g.milestones || []).forEach((m) => (m.todos || []).forEach((t) => {
    rows.push({ ...t, milestone_id: m.id, milestone_title: m.title, goal_title: g.title });
  })));
  backlog.forEach((t) => rows.push({ ...t, milestone_id: null, milestone_title: null, goal_title: null }));
  return rows;
}

function CreateTodoModal({ open, overview, onCancel, onNotice, onCreated }) {
  const [form] = Form.useForm();
  const [saving, setSaving] = useState(false);
  const goals = (overview && overview.goals) || [];
  const msOptions = goals.flatMap((g) => (g.milestones || []).map((m) => ({
    value: m.id,
    label: `${g.title} / ${m.title}`,
  })));

  const submit = async () => {
    let v;
    try {
      v = await form.validateFields();
    } catch {
      return;
    }
    setSaving(true);
    try {
      const rec = await apiPost('/api/project/todos', {
        title: v.title,
        draft: v.draft || '',
        agent: v.agent || 'act',
        ...(v.milestone_id ? { milestone_id: v.milestone_id } : {}),
      });
      form.resetFields();
      onNotice('TODO 已创建，可继续生成 Plan');
      onCreated(rec && rec.id);
      return true;
    } catch (e) {
      onNotice('新建 TODO 失败: ' + (e && e.message));
      return false;
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal
      open={open}
      title="新建 TODO"
      onCancel={onCancel}
      destroyOnHidden
      footer={[
        <Button key="cancel" onClick={onCancel}>取消</Button>,
        <Button key="ok" type="primary" loading={saving} onClick={submit}>创建</Button>,
      ]}
    >
      <Form form={form} layout="vertical" preserve={false} initialValues={{ agent: 'act' }}>
        <Form.Item name="title" label="标题" rules={[{ required: true, message: '请输入标题' }]}>
          <Input placeholder="要完成的一件事" />
        </Form.Item>
        <Form.Item name="milestone_id" label="里程碑">
          <Select allowClear placeholder="不选则进入 backlog（未分组）" options={msOptions} aria-label="milestone_id" />
        </Form.Item>
        <Form.Item name="agent" label="执行 agent">
          <Input placeholder="act" style={{ width: 200 }} />
        </Form.Item>
        <Form.Item name="draft" label="草稿">
          <TextArea rows={5} placeholder="粗略描述要做什么…" aria-label="draft" />
        </Form.Item>
      </Form>
    </Modal>
  );
}

export function TodosTab({ overview, refresh, openTodo, onNotice }) {
  const [createOpen, setCreateOpen] = useState(false);
  const rows = flattenTodos(overview);
  const busy = (t) => t.status === 'running';

  const genPlan = async (t) => {
    try {
      await apiPost(todoPath(t.id) + '/plan');
      onNotice(`已开始为「${t.title}」生成 Plan`);
      refresh();
      openTodo(t.id);
    } catch (e) {
      onNotice('生成 Plan 失败: ' + (e && e.message));
    }
  };

  const execute = async (t) => {
    try {
      await apiPost(todoPath(t.id) + '/execute');
      onNotice(`「${t.title}」已开始执行`);
      refresh();
      openTodo(t.id);
    } catch (e) {
      onNotice('执行失败: ' + (e && e.message));
    }
  };

  const remove = async (t) => {
    try {
      await apiDel(todoPath(t.id));
      onNotice('TODO 已删除');
      refresh();
    } catch (e) {
      onNotice('删除 TODO 失败: ' + (e && e.message));
    }
  };

  const columns = [
    { title: '标题', dataIndex: 'title', key: 'title', ellipsis: true, render: (v, r) => <a onClick={() => openTodo(r.id)}>{v}</a> },
    {
      title: '里程碑',
      key: 'milestone',
      width: 180,
      ellipsis: true,
      render: (_, r) => (r.milestone_title
        ? <Tooltip title={r.goal_title}><span>{r.milestone_title}</span></Tooltip>
        : <Text type="secondary">未分组</Text>),
    },
    { title: '状态', dataIndex: 'status', key: 'status', width: 90, render: (v) => <TodoStatusTag status={v} /> },
    {
      title: '计划',
      dataIndex: 'plan_md',
      key: 'plan',
      width: 64,
      align: 'center',
      render: (v) => (v ? <Text type="success">✓</Text> : <Text type="secondary">—</Text>),
    },
    { title: 'Agent', dataIndex: 'agent', key: 'agent', width: 84, render: (v) => v || 'act' },
    {
      title: '操作',
      key: 'ops',
      width: 250,
      render: (_, r) => (
        <Space size={0}>
          <Button type="link" size="small" disabled={busy(r)} onClick={() => genPlan(r)}>生成Plan</Button>
          <Button
            type="link"
            size="small"
            disabled={busy(r) || !r.plan_md}
            onClick={() => execute(r)}
          >
            执行
          </Button>
          <Button type="link" size="small" onClick={() => openTodo(r.id)}>详情</Button>
          <Popconfirm
            title="删除该 TODO？"
            description="将一并删除其执行记录。"
            okText="删除"
            okButtonProps={{ danger: true }}
            cancelText="取消"
            onConfirm={() => remove(r)}
          >
            <Button type="link" size="small" danger>删除</Button>
          </Popconfirm>
        </Space>
      ),
    },
  ];

  return (
    <Space orientation="vertical" style={{ width: '100%' }} size={12}>
      <div>
        <Button type="primary" onClick={() => setCreateOpen(true)}>新建 TODO</Button>
        <Text type="secondary" style={{ marginLeft: 12 }}>
          工作流：草稿 → 生成Plan → 执行 → 版本留存（执行记录里可回看每次 Plan/执行）
        </Text>
      </div>
      <Table
        rowKey="id"
        size="middle"
        columns={columns}
        dataSource={rows}
        pagination={false}
        locale={{ emptyText: '还没有 TODO — 新建一个，先写草稿再生成 Plan' }}
      />
      <CreateTodoModal
        open={createOpen}
        overview={overview}
        onCancel={() => setCreateOpen(false)}
        onNotice={onNotice}
        onCreated={(id) => {
          setCreateOpen(false);
          refresh();
          if (id) {
            openTodo(id); // straight into the drawer for plan generation
          }
        }}
      />
    </Space>
  );
}
