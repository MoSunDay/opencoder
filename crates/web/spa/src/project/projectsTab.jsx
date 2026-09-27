import { Button, Card, Empty, Popconfirm, Space, Typography } from 'antd';
import { useState } from 'react';
import { apiDel, apiPatch, apiPost } from '../api.js';
import { StatusTag } from '../ui/statusTag.jsx';
import { Markdown } from './markdown.jsx';
import { MdEditDrawer } from './views/mdDrawer.jsx';
import { err, ok } from '../notice.js';

const { Text } = Typography;

const goalPath = (id) => '/api/project/goals/' + encodeURIComponent(id);

export function ProjectsTab({ overview, refresh, onNotice, openInitiatives, openMilestones }) {
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
        onNotice(ok('项目已更新'));
      } else {
        await apiPost('/api/project/goals', v);
        onNotice(ok('项目已创建'));
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
      onNotice(ok(next === 'archived' ? '项目已归档' : '项目已重新激活'));
      refresh();
    } catch (e) {
      onNotice(err('切换状态失败: ' + (e && e.message)));
    }
  };

  const remove = async (g) => {
    try {
      await apiDel(goalPath(g.id));
      onNotice(ok('项目已删除'));
      refresh();
    } catch (e) {
      onNotice(err('删除目标失败: ' + (e && e.message)));
    }
  };

  return (
    <Space orientation="vertical" style={{ width: '100%' }} size={16}>
      <div>
        <Button type="primary" onClick={startCreate}>新建项目</Button>
        <Text type="secondary" style={{ marginLeft: 12 }}>
          里程碑与专项同级，均可独立存在或关联项目
        </Text>
      </div>
      {!goals.length && <Empty description="还没有项目，可以直接创建专项或 TODO" />}
      {goals.map((g) => (
        <Card
          key={g.id}
          size="small"
          title={<span>{g.title} <Text type="secondary">#{g.id.slice(0, 8)}</Text></span>}
          extra={<Space size={8}><StatusTag status={g.status} /><Text type="secondary">sort {g.sort}</Text></Space>}
          actions={[
            <Button key="milestones" type="link" size="small" onClick={() => openMilestones?.(g.id)}>查看里程碑</Button>,
            <Button key="initiatives" type="link" size="small" onClick={() => openInitiatives?.(g.id)}>查看专项</Button>,
            <Button key="edit" type="link" size="small" onClick={() => startEdit(g)}>编辑</Button>,
            <Button key="toggle" type="link" size="small" onClick={() => toggleStatus(g)}>
              {g.status === 'archived' ? '激活' : '归档'}
            </Button>,
            <Popconfirm
              key="del"
              title="删除该项目？"
              description="只删除项目，专项变为独立专项；TODO 和执行关联保留。"
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
      <MdEditDrawer
        open={open}
        title={editing ? '编辑项目' : '新建项目'}
        initial={editing}
        onCancel={() => setOpen(false)}
        onOk={save}
      />
    </Space>
  );
}
