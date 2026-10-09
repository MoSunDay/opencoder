import { EditButton } from '../../ui/permissions.jsx';
import { Button, Drawer, Form, Select, Space, Typography } from 'antd';
import { useState } from 'react';
import { apiPatch } from '../../api.js';
import { err, ok } from '../../notice.js';
import { Markdown } from '../markdown.jsx';
import { TodoBoard } from '../board/board.jsx';
import { flattenInitiatives, projectOptions, searchSelect } from '../model/relations.js';
import { progressOf } from '../model/catalog.js';
import { TodoProgress } from './progress.jsx';
import { MdEditDrawer } from './mdDrawer.jsx';
export function InitiativeDrawer({ initiativeId, overview, refresh, onNotice, onClose, openTodo }) {
  const initiative = flattenInitiatives(overview).find((i) => i.id === initiativeId);
  const [editing, setEditing] = useState(false);
  const save = async (values) => {
    try { await apiPatch(`/api/project/initiatives/${encodeURIComponent(initiativeId)}`, { ...values, goal_id: values.goal_id ?? null }); await refresh(); setEditing(false); onNotice(ok('专项已保存，TODO 标签已重新匹配')); return true; }
    catch (error) { onNotice(err(error.message)); return false; }
  };
  return <Drawer open={!!initiativeId} title={`专项 · ${initiative?.title || ''}`} onClose={onClose} size="100vw" className="project-initiative-drawer">
    {initiative ? <Space orientation="vertical" size={18} style={{ width: '100%' }}>
      <Space wrap><Typography.Text type="secondary">所属项目：{initiative.goal_title || '独立专项'}</Typography.Text><EditButton onClick={() => setEditing(true)}>编辑专项</EditButton></Space>
      <Markdown text={initiative.detail_md} /><TodoProgress progress={progressOf(initiative.todos)} />
      <TodoBoard key={initiativeId} initiative={initiative} overview={overview} refresh={refresh} onNotice={onNotice} openTodo={openTodo} />
    </Space> : initiativeId && <Typography.Text type="secondary">专项已删除或正在加载</Typography.Text>}
    <MdEditDrawer open={editing} title="编辑专项" initial={initiative} onCancel={() => setEditing(false)} onOk={save}
      extraTop={<><Form.Item name="goal_id" label="所属项目"><Select {...searchSelect} aria-label="goal_id" options={projectOptions(overview)} /></Form.Item><Form.Item name="status" label="状态"><Select options={[{ value: 'planned', label: '未开始' }, { value: 'in_progress', label: '进行中' }, { value: 'done', label: '已完成' }]} /></Form.Item></>} />
  </Drawer>;
}
