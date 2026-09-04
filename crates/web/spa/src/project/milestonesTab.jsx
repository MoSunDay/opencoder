// milestonesTab.jsx — 项目 tab 3「里程碑」: goals grouped as Collapse panels,
// each listing its milestones (title + status Tag + TODO badge + Markdown
// detail expandable) with inline 状态切换 Segmented, 编辑 (shared MdEditModal)
// and 删除 (cascades todos + runs). 新建里程碑 requires ≥1 goal — the modal
// carries an extraTop goal Select owned by this tab.

import { Badge, Button, Collapse, Empty, Form, Popconfirm, Segmented, Select, Space, Typography } from 'antd';
import { useState } from 'react';
import { apiDel, apiPatch, apiPost } from '../api.js';
import { MilestoneStatusTag } from './labels.jsx';
import { Markdown } from './markdown.jsx';
import { MdEditModal } from './mdModal.jsx';

const { Text } = Typography;

const STATUS_OPTIONS = [
  { label: '未开始', value: 'planned' },
  { label: '进行中', value: 'in_progress' },
  { label: '已完成', value: 'done' },
];

const msPath = (id) => '/api/project/milestones/' + encodeURIComponent(id);

export function MilestonesTab({ overview, refresh, onNotice }) {
  const goals = (overview && overview.goals) || [];
  const [open, setOpen] = useState(false);
  const [editing, setEditing] = useState(null); // milestone record | null = create
  const [createGoal, setCreateGoal] = useState(null);

  const startCreate = (goalId) => {
    setEditing(null);
    setCreateGoal(goalId || (goals.length === 1 ? goals[0].id : null));
    setOpen(true);
  };
  const startEdit = (m) => {
    setEditing(m);
    setOpen(true);
  };

  const save = async (v) => {
    const body = editing ? v : { ...v, goal_id: createGoal };
    if (!editing && !body.goal_id) {
      onNotice('请选择所属目标');
      return false;
    }
    try {
      if (editing) {
        await apiPatch(msPath(editing.id), body);
        onNotice('里程碑已更新');
      } else {
        await apiPost('/api/project/milestones', body);
        onNotice('里程碑已创建');
      }
      setOpen(false);
      refresh();
      return true;
    } catch (e) {
      onNotice('保存里程碑失败: ' + (e && e.message));
      return false;
    }
  };

  const setStatus = async (m, status) => {
    try {
      await apiPatch(msPath(m.id), { status });
      refresh();
    } catch (e) {
      onNotice('切换里程碑状态失败: ' + (e && e.message));
    }
  };

  const remove = async (m) => {
    try {
      await apiDel(msPath(m.id));
      onNotice('里程碑已删除');
      refresh();
    } catch (e) {
      onNotice('删除里程碑失败: ' + (e && e.message));
    }
  };

  if (!goals.length) {
    return <Empty description="先在「项目目标」tab 创建目标，才能添加里程碑" />;
  }

  const collapseItems = goals.map((g) => ({
    key: g.id,
    label: (
      <Space size={8}>
        <Text strong>{g.title}</Text>
        <Badge count={g.milestones.length} title="里程碑数" color="#1677ff" />
        <Text type="secondary">里程碑 {g.milestones.length}</Text>
      </Space>
    ),
    children: (
      <Space orientation="vertical" style={{ width: '100%' }} size={12}>
        {g.milestones.length ? null : <Text type="secondary">该目标还没有里程碑</Text>}
        {g.milestones.map((m) => (
          <div key={m.id} style={{ padding: '8px 12px', border: '1px solid #f0f0f0', borderRadius: 6 }}>
            <Space size={8} wrap style={{ marginBottom: 4 }}>
              <Text strong>{m.title}</Text>
              <MilestoneStatusTag status={m.status} />
              <Badge count={m.todos.length} title="TODO 数" color="#faad14" />
              <Segmented
                size="small"
                value={m.status}
                options={STATUS_OPTIONS}
                onChange={(v) => setStatus(m, v)}
              />
              <Button type="link" size="small" onClick={() => startEdit(m)}>编辑</Button>
              <Popconfirm
                title="删除该里程碑？"
                description="将级联删除其 TODO 与执行记录。"
                okText="删除"
                okButtonProps={{ danger: true }}
                cancelText="取消"
                onConfirm={() => remove(m)}
              >
                <Button type="link" size="small" danger>删除</Button>
              </Popconfirm>
            </Space>
            <Collapse
              size="small"
              ghost
              items={[{
                key: 'detail',
                label: <Text type="secondary">详情</Text>,
                children: <Markdown text={m.detail_md} />,
              }]}
            />
          </div>
        ))}
        <Button size="small" onClick={() => startCreate(g.id)}>在此目标下新建里程碑</Button>
      </Space>
    ),
  }));

  return (
    <Space orientation="vertical" style={{ width: '100%' }} size={16}>
      <div>
        <Button type="primary" onClick={() => startCreate(null)}>新建里程碑</Button>
        <Text type="secondary" style={{ marginLeft: 12 }}>按目标分组；状态可就地切换</Text>
      </div>
      <Collapse items={collapseItems} defaultActiveKey={goals.slice(0, 1).map((g) => g.id)} />
      <MdEditModal
        open={open}
        title={editing ? '编辑里程碑' : '新建里程碑'}
        initial={editing ? { title: editing.title, sort: editing.sort, detail_md: editing.detail_md } : null}
        extraTop={editing ? null : (
          <Form.Item label="所属目标" required>
            <Select
              value={createGoal}
              onChange={setCreateGoal}
              placeholder="选择目标"
              style={{ width: '100%' }}
              aria-label="goal_id"
              options={goals.map((g) => ({ value: g.id, label: g.title }))}
            />
          </Form.Item>
        )}
        onCancel={() => setOpen(false)}
        onOk={save}
      />
    </Space>
  );
}
