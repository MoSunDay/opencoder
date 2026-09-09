import { submitAttempt } from './replay/attempt.js';
// todosTab.jsx — 项目 tab 4「TODO」: flattened table over overview (milestone
// todos + backlog), row ops 生成Plan / 执行 / 详情 / 删除, and a 新建 TODO modal
// (title + milestone Select allowClear + agent + draft). Plan/Execute POSTs
// return 202 {run_id}; the row's 生成Plan jumps straight into the drawer so
// the plan-generation workflow continues without a detour.

import { Button, Form, Input, Modal, Popconfirm, Select, Space, Table, Tooltip, Typography } from 'antd';
import { useEffect, useState } from 'react';
import { flattenTodos, milestoneOptions, goalOptions, matchesText, searchSelect } from './model/relations.js';
import { RelationSelect } from './views/relationSelect.jsx';
import { apiDel, apiPost } from '../api.js';
import { ExecutorTag, TodoStatusTag } from './labels.jsx';
import { err, info, ok } from '../notice.js';

const { TextArea } = Input;
const { Text } = Typography;

const todoPath = (id) => '/api/project/todos/' + encodeURIComponent(id);

/// 执行器下拉选项（executor_kind）。与 labels.jsx 的 EXECUTOR_META 同源
/// 文案；这里是纯数据以便单测断言四项齐全。
export const EXECUTOR_OPTIONS = [
  { value: 'agent', label: '单Agent' },
  { value: 'team', label: '团队' },
  { value: 'dag', label: 'DAG' },
  { value: 'brain', label: '大脑' },
];

/// Build the executor slice of a create/patch body from form values (pure):
/// kind defaults to agent; ref/spec join only when non-blank (trimmed); a
/// spec must parse as JSON before anything is submitted. Returns
/// `{ fields }` or `{ error }`.
export function executorBody(v) {
  const kind = String((v && v.executor_kind) || 'agent');
  const fields = { executor_kind: kind };
  if (kind === 'agent') {
    return { fields };
  }
  const ref = String((v && v.executor_ref) || '').trim();
  if (ref) {
    fields.executor_ref = ref;
  }
  const spec = String((v && v.executor_spec) || '').trim();
  if (spec) {
    try {
      JSON.parse(spec);
    } catch {
      return { error: 'executor_spec 不是合法 JSON，请检查后重试' };
    }
    fields.executor_spec = spec;
  }
  return { fields };
}

/// Secondary text beside the ExecutorTag in the table: agent shows its
/// agent name (default act), other kinds show executor_ref (inline-spec
/// todos may carry none).
export function executorRefText(r) {
  const kind = (r && r.executor_kind) || 'agent';
  return kind === 'agent' ? (r.agent || 'act') : ((r && r.executor_ref) || '');
}

export { flattenTodos } from './model/relations.js';

function CreateTodoModal({ open, overview, onCancel, onNotice, onCreated }) {
  const [form] = Form.useForm();
  const [saving, setSaving] = useState(false);
  useEffect(() => { if (open) form.resetFields(); }, [open, form]);
  const kind = Form.useWatch('executor_kind', form) || 'agent';
  const msOptions = milestoneOptions(overview);

  const submit = async () => {
    let v;
    try {
      v = await form.validateFields();
    } catch {
      return;
    }
    const exec = executorBody(v);
    if (exec.error) {
      onNotice(err(exec.error));
      return;
    }
    setSaving(true);
    try {
      const rec = await apiPost('/api/project/todos', {
        title: v.title,
        draft: v.draft || '',
        ...(exec.fields.executor_kind === 'agent' ? { agent: v.agent || 'act' } : {}),
        ...exec.fields,
        ...(v.milestone_id ? { milestone_id: v.milestone_id } : {}),
      });
      form.resetFields();
      onNotice(ok('TODO 已创建，可继续生成 Plan'));
      onCreated(rec && rec.id);
      return true;
    } catch (e) {
      onNotice(err('新建 TODO 失败: ' + (e && e.message)));
      return false;
    } finally {
      setSaving(false);
    }
  };

  return (
    <Modal
      open={open}
      title="新建 TODO"
      onCancel={() => { if (!saving) onCancel(); }}
      destroyOnHidden
      footer={[
        <Button key="cancel" disabled={saving} onClick={onCancel}>取消</Button>,
        <Button key="ok" type="primary" loading={saving} onClick={submit}>创建</Button>,
      ]}
    >
      <Form form={form} layout="vertical" disabled={saving} initialValues={{ agent: 'act', executor_kind: 'agent' }}>
        <Form.Item name="title" label="标题" rules={[{ required: true, message: '请输入标题' }]}>
          <Input placeholder="要完成的一件事" />
        </Form.Item>
        <Form.Item name="milestone_id" label="里程碑">
          <Select {...searchSelect} placeholder="可不关联里程碑" options={msOptions} aria-label="milestone_id" />
        </Form.Item>
        <Form.Item name="executor_kind" label="执行器" tooltip="agent 直驱会话；team/dag 走本地多人/DAG；brain 由能力库路由">
          <Select aria-label="executor_kind" options={EXECUTOR_OPTIONS} style={{ width: 160 }} />
        </Form.Item>
        {kind === 'agent' ? (
          <Form.Item name="agent" label="执行 agent">
            <Input placeholder="act" style={{ width: 200 }} />
          </Form.Item>
        ) : (
          <>
            <Form.Item
              name="executor_ref"
              label={kind === 'brain' ? '钉定能力 id' : '目标名'}
              tooltip={kind === 'brain' ? '留空则每次执行时由大脑按标题/草稿自动路由' : '命名团队 / DAG 定义；留空则只看内联定义'}
            >
              <Input placeholder={kind === 'brain' ? '留空 = 自动路由' : '可留空'} style={{ width: 240 }} />
            </Form.Item>
            <Form.Item
              name="executor_spec"
              label={kind === 'brain' ? '路由表 JSON' : '内联定义 JSON'}
              tooltip={kind === 'brain' ? '{"routes":[…],"default":{…}}，可留空' : 'team 定义 / DagSpec；留空则按目标名解析'}
            >
              <TextArea rows={4} placeholder={kind === 'brain' ? '{"routes":[]}' : '{"name":"…","captain":{…}} / {"name":"…","steps":[…]}'} aria-label="executor_spec" />
            </Form.Item>
          </>
        )}
        <Form.Item name="draft" label="草稿">
          <TextArea rows={5} placeholder="粗略描述要做什么…" aria-label="draft" />
        </Form.Item>
      </Form>
    </Modal>
  );
}

export function TodosTab({ overview, refresh, openTodo, onNotice, milestoneFilter, setMilestoneFilter }) {
  const [createOpen, setCreateOpen] = useState(false);
  const [query, setQuery] = useState('');
  const [statusFilter, setStatusFilter] = useState(null);
  const [projectFilter, setProjectFilter] = useState(null);
  const rows = flattenTodos(overview).filter((todo) => matchesText(query, todo.title, todo.id)
    && (!statusFilter || todo.status === statusFilter)
    && (!projectFilter || (projectFilter === 'unlinked' ? !todo.goal_id : todo.goal_id === projectFilter))
    && (!milestoneFilter || (milestoneFilter === 'unlinked' ? !todo.milestone_id : todo.milestone_id === milestoneFilter)));
  const busy = (t) => t.status === 'running' || ['pending', 'running', 'cancelling'].includes(t.execution?.status);
  const executionClosed = (t) => ['cancelled', 'done'].includes(t.execution?.status);

  const genPlan = async (t) => {
    try {
      await submitAttempt(t.id, 'plan');
      onNotice(info(`已开始为「${t.title}」生成 Plan`));
      refresh();
      openTodo(t.id);
    } catch (e) {
      onNotice(err('生成 Plan 失败: ' + (e && e.message)));
    }
  };

  const execute = async (t) => {
    try {
      await submitAttempt(t.id, 'execute');
      onNotice(info(`「${t.title}」已开始执行`));
      refresh();
      openTodo(t.id);
    } catch (e) {
      onNotice(err('执行失败: ' + (e && e.message)));
    }
  };

  const remove = async (t) => {
    try {
      await apiDel(todoPath(t.id));
      onNotice(ok('TODO 已删除'));
      refresh();
    } catch (e) {
      onNotice(err('删除 TODO 失败: ' + (e && e.message)));
    }
  };

  const columns = [
    { title: '标题', dataIndex: 'title', key: 'title', ellipsis: true, render: (v, r) => <a onClick={() => openTodo(r.id)}>{v}</a> },
    {
      title: '里程碑',
      key: 'milestone',
      width: 180,
      ellipsis: true,
      render: (_, r) => <RelationSelect path={todoPath(r.id)} field="milestone_id" value={r.milestone_id}
        options={milestoneOptions(overview)} refresh={refresh} onNotice={onNotice} label={`TODO ${r.title} 所属里程碑`} />,
    },
    { title: '项目', dataIndex: 'goal_title', render: (v) => v || '未关联项目' },
    { title: '状态', dataIndex: 'status', key: 'status', width: 90, render: (v) => <TodoStatusTag status={v} /> },
    {
      title: '计划',
      dataIndex: 'plan_md',
      key: 'plan',
      width: 64,
      align: 'center',
      render: (v) => (v ? <Text type="success">✓</Text> : <Text type="secondary">—</Text>),
    },
    {
      title: '执行器',
      key: 'executor',
      width: 110,
      render: (_, r) => {
        const ref = executorRefText(r);
        return (
          <Space size={4}>
            <ExecutorTag kind={r.executor_kind} />
            {ref ? <Text type="secondary" style={{ fontSize: 12 }}>{ref}</Text> : null}
          </Space>
        );
      },
    },
    {
      title: '操作',
      key: 'ops',
      width: 250,
      render: (_, r) => (
        <Space size={0}>
          <Button type="link" size="small" disabled={busy(r) || executionClosed(r)} onClick={() => genPlan(r)}>生成Plan</Button>
          <Button
            type="link"
            size="small"
            disabled={busy(r) || executionClosed(r) || !r.plan_md}
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
      <Space wrap>
        <Input.Search aria-label="搜索 TODO" placeholder="搜索名称或 ID" value={query} allowClear onChange={(e) => setQuery(e.target.value)} style={{ width: 210 }} />
        <Select {...searchSelect} aria-label="筛选 TODO 里程碑" placeholder="全部里程碑" style={{ width: 230 }} value={milestoneFilter}
          onChange={setMilestoneFilter} options={[{ value: 'unlinked', label: '未关联里程碑' }, ...milestoneOptions(overview)]} />
        <Select {...searchSelect} aria-label="筛选 TODO 项目" placeholder="全部项目" style={{ width: 200 }} value={projectFilter}
          onChange={setProjectFilter} options={[{ value: 'unlinked', label: '未关联项目' }, ...goalOptions(overview)]} />
        <Select allowClear aria-label="筛选 TODO 状态" placeholder="全部状态" style={{ width: 140 }} value={statusFilter} onChange={setStatusFilter}
          options={['draft', 'planned', 'running', 'done', 'failed'].map((value) => ({ value, label: <TodoStatusTag status={value} /> }))} />
      </Space>
      <Table
        scroll={{ x: 'max-content' }}
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
