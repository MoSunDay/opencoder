// goalsTab.jsx — 项目 tab 2「项目目标」: one antd Card per goal (title +
// status Tag + Markdown detail) with 编辑 / 归档·激活 / 删除 actions, plus a
// 新建目标 button opening the shared MdEditModal. All writes go through
// /api/project/goals* and end with refresh() so the parent overview state
// (goals + milestones + todos) stays the single source of truth.

import { Button, Card, Empty, Popconfirm, Space, Typography } from 'antd';
import { useState } from 'react';
import { apiDel, apiPatch, apiPost } from '../api.js';
import { GoalStatusTag } from './labels.jsx';
import { Markdown } from './markdown.jsx';
import { MdEditModal } from './views/mdModal.jsx';
import { err, ok } from '../notice.js';

const { Text } = Typography;

const goalPath = (id) => '/api/project/goals/' + encodeURIComponent(id);

export function GoalsTab({ overview, refresh, onNotice, openMilestones }) {
  const goals = (overview && overview.goals) || [];
  const [open, setOpen] = useState(false);
  const [editing, setEditing] = useState(null); // goal record | null = create

  const startCreate = () => {
    setEditing(null);
    setOpen(true);
  };
  const startEdit = (g) => {
    setEditing(g);
    setOpen(true);
  };

  const save = async (v) => {
    try {
      if (editing) {
        await apiPatch(goalPath(editing.id), v);
        onNotice(ok('目标已更新'));
      } else {
        await apiPost('/api/project/goals', v);
        onNotice(ok('目标已创建'));
      }
      setOpen(false);
      refresh();
      return true;
    } catch (e) {
      onNotice(err('保存目标失败: ' + (e && e.message)));
      return false;
    }
  };

  const toggleStatus = async (g) => {
    const next = g.status === 'archived' ? 'active' : 'archived';
    try {
      await apiPatch(goalPath(g.id), { status: next });
      onNotice(ok(next === 'archived' ? '目标已归档' : '目标已重新激活'));
      refresh();
    } catch (e) {
      onNotice(err('切换状态失败: ' + (e && e.message)));
    }
  };

  const remove = async (g) => {
    try {
      await apiDel(goalPath(g.id));
      onNotice(ok('目标已删除'));
      refresh();
    } catch (e) {
      onNotice(err('删除目标失败: ' + (e && e.message)));
    }
  };

  return (
    <Space orientation="vertical" style={{ width: '100%' }} size={16}>
      <div>
        <Button type="primary" onClick={startCreate}>新建目标</Button>
        <Text type="secondary" style={{ marginLeft: 12 }}>
          里程碑可独立存在，也可随时关联项目
        </Text>
      </div>
      {!goals.length && <Empty description="还没有项目，可以直接创建里程碑或 TODO" />}
      {goals.map((g) => (
        <Card
          key={g.id}
          size="small"
          title={<span>{g.title} <Text type="secondary">#{g.id.slice(0, 8)}</Text></span>}
          extra={<Space size={8}><GoalStatusTag status={g.status} /><Text type="secondary">sort {g.sort}</Text></Space>}
          actions={[
            <Button key="milestones" type="link" size="small" onClick={() => openMilestones?.(g.id)}>查看里程碑</Button>,
            <Button key="edit" type="link" size="small" onClick={() => startEdit(g)}>编辑</Button>,
            <Button key="toggle" type="link" size="small" onClick={() => toggleStatus(g)}>
              {g.status === 'archived' ? '激活' : '归档'}
            </Button>,
            <Popconfirm
              key="del"
              title="删除该目标？"
              description="只删除项目，里程碑转为独立专项；TODO 和执行记录保留。"
              okText="删除"
              okButtonProps={{ danger: true }}
              cancelText="取消"
              onConfirm={() => remove(g)}
            >
              <Button type="link" size="small" danger>删除</Button>
            </Popconfirm>,
          ]}
        >
          <Markdown text={g.detail_md} />
        </Card>
      ))}
      <MdEditModal
        open={open}
        title={editing ? '编辑目标' : '新建目标'}
        initial={editing}
        onCancel={() => setOpen(false)}
        onOk={save}
      />
    </Space>
  );
}
