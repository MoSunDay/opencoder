import { CurrentSummary } from '../summary/view.jsx';
import { ContentSection } from '../summary/section.jsx';
import { PeopleSummary } from '../people/fields.jsx';
import { EditButton } from '../../ui/permissions.jsx';
import { Drawer, Form, Select, Typography } from 'antd';
import { useEffect, useState } from 'react';
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
  useEffect(() => { setEditing(false); }, [initiativeId]);
  const close = (event) => {
    // dnd-kit cancels at document; Drawer receives the same Escape at window.
    if (event?.key === 'Escape' && event.defaultPrevented) return;
    onClose(event);
  };
  const save = async (values) => {
    try { await apiPatch(`/api/project/initiatives/${encodeURIComponent(initiativeId)}`, { ...values, goal_id: values.goal_id ?? null }); await refresh(); setEditing(false); onNotice(ok('子项已保存，TODO 标签已重新匹配')); return true; }
    catch (error) { onNotice(err(error.message)); return false; }
  };
  return <Drawer open={!!initiativeId} title={`子项 · ${initiative?.title || ''}`} onClose={close} size="100vw" className="project-detail-drawer project-initiative-drawer" destroyOnHidden extra={<EditButton onClick={() => setEditing(true)}>编辑子项</EditButton>}>
    {initiative ? <div className="project-detail-overview">
      <div className="project-initiative-context">
        <CurrentSummary record={initiative} collection="initiatives" refresh={refresh} />
        <ContentSection key={initiativeId} title="子项目标"><Markdown text={initiative.detail_md} /></ContentSection>
      </div>
      <div className="project-detail-meta">
        <div className="project-detail-people"><Typography.Text type="secondary">所属项目：{initiative.goal_title || '独立子项'}</Typography.Text><PeopleSummary record={initiative} /></div>
        <TodoProgress progress={progressOf(initiative.todos)} />
      </div>
      <section className="project-initiative-board" aria-label="TODO 看板">
        <Typography.Title level={5}>TODO 看板</Typography.Title>
        <TodoBoard key={initiativeId} initiative={initiative} overview={overview} refresh={refresh} onNotice={onNotice} openTodo={openTodo} />
      </section>
    </div> : initiativeId && <Typography.Text type="secondary">子项已删除或正在加载</Typography.Text>}
    <MdEditDrawer open={editing} title="编辑子项" detailLabel="子项目标" initial={initiative} onCancel={() => setEditing(false)} onOk={save}
      extraTop={<><Form.Item name="goal_id" label="所属项目"><Select {...searchSelect} aria-label="goal_id" options={projectOptions(overview)} /></Form.Item></>} />
  </Drawer>;
}
