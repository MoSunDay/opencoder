import { Alert, Drawer, Form, Tabs, Typography } from 'antd';
import { useEffect, useRef, useState } from 'react';
import { EditButton, useCanEdit } from '../ui/permissions.jsx';
import { apiPatch } from '../api.js';
import { ok } from '../notice.js';
import { flattenTodos } from './model/relations.js';
import { remapTagIds } from './model/catalog.js';
import { laneOf } from './model/board.js';
import { TodoTaskFields, TodoSettingsFields } from './forms/editor.jsx';
import { TodoActions } from './forms/section.jsx';
import { useCloseGuard } from './forms/closeGuard.js';
import { TodoExecution } from './execute/todoExecution.jsx';
import { TodoComments } from './comments/panel.jsx';
import { UnavailableDetail } from './navigation/unavailable.jsx';

function seed(todo) {
  return { title: todo.title || '', draft: todo.draft || '', initiative_id: todo.initiative_id || null,
    board_status: laneOf(todo), tag_ids: todo.tag_ids || [], capability_id: todo.capability_id || null,
    owner_user_name: todo.owner_user_name ?? null, follower_user_names: todo.follower_user_names || [] };
}
function payload(values, overview) {
  return { ...values, title: (values.title || '').trim(), draft: values.draft || '', initiative_id: values.initiative_id || null,
    tag_ids: remapTagIds(overview, values.tag_ids || [], values.initiative_id), capability_id: values.capability_id || null,
    owner_user_name: values.owner_user_name ?? null, follower_user_names: values.follower_user_names || [] };
}

function TodoDrawerSession({ todoId, todo, overview, refresh, onClose, onNotice }) {
  const canEdit = useCanEdit();
  const [form] = Form.useForm();
  const watched = Form.useWatch([], { form, preserve: true });
  const values = { ...seed(todo), ...form.getFieldsValue(true), ...watched };
  const [baseline, setBaseline] = useState(() => JSON.stringify(payload(seed(todo), overview)));
  const dirty = JSON.stringify(payload(values, overview)) !== baseline;
  const [tab, setTab] = useState('task');
  const [saving, setSaving] = useState(false);
  const [working, setWorking] = useState(false);
  const [commentDirty, setCommentDirty] = useState(false);
  const [source, setSource] = useState(null);
  const [error, setError] = useState('');
  const submitting = useRef(false);
  const [close, holder] = useCloseGuard({ dirty: dirty || commentDirty, busy: saving || working, onClose });
  const updated = JSON.stringify(seed(todo));
  const previous = useRef(updated);
  useEffect(() => {
    if (previous.current === updated || dirty || saving) return;
    previous.current = updated;
    const next = seed(todo); form.setFieldsValue(next); setBaseline(JSON.stringify(payload(next, overview)));
  }, [updated, dirty, saving, form, todo, overview]);
  const validate = async () => {
    try { await form.validateFields(); return true; }
    catch {
      setTab('task');
      requestAnimationFrame(() => form.scrollToField('title', { focus: true }));
      return false;
    }
  };
  const save = async () => {
    if (!canEdit || submitting.current || !await validate()) return false;
    if (submitting.current) return false;
    submitting.current = true; setSaving(true); setError('');
    const body = payload(form.getFieldsValue(true), overview);
    try {
      await apiPatch(`/api/project/todos/${encodeURIComponent(todoId)}`, body);
      setBaseline(JSON.stringify(body)); await refresh();
      onNotice(ok('TODO 已保存')); return true;
    } catch (failure) { setError(failure.message); return false; }
    finally { submitting.current = false; setSaving(false); }
  };
  return <Drawer open title={`TODO · ${todo.title || todoId}`} onClose={close} placement="right" size="100vw" className="project-detail-drawer project-todo-drawer" destroyOnHidden
    footer={<TodoActions>
      {error && <Alert type="error" showIcon title={error} />}
      {dirty && <Typography.Text type="secondary">有未保存的修改</Typography.Text>}
      <EditButton type="primary" disabled={working || !canEdit} loading={saving} onClick={save}>保存 TODO</EditButton>
    </TodoActions>}>
    {holder}
    <div className="project-todo-workspace">
      <Form form={form} component="div" layout="vertical" disabled={!canEdit || saving || working} initialValues={seed(todo)} preserve
        onValuesChange={(changed, all) => {
          if ('initiative_id' in changed) form.setFieldValue('tag_ids', remapTagIds(overview, all.tag_ids || [], all.initiative_id));
        }}>
        <Tabs activeKey={tab} onChange={setTab} items={[
          { key: 'task', label: '任务', forceRender: true, children: <>
            <TodoTaskFields />
            <section className="project-task-comments" aria-label="任务评论">
              <Typography.Title level={5}>评论</Typography.Title>
              <TodoComments todoId={todoId} active={tab === 'task'} onDraftChange={setCommentDirty} onBusyChange={setWorking}
                openExecution={(id) => { setSource(id); setTab('execution'); }} refresh={refresh} />
            </section>
          </> },
          { key: 'settings', label: '设置', forceRender: true, children: <TodoSettingsFields overview={overview} /> },
          { key: 'execution', label: '执行', forceRender: true, children: <TodoExecution todoId={todoId} active={tab === 'execution'} save={save} source={source}
            onSourceOpened={() => setSource(null)} onBusyChange={setWorking} onNotice={onNotice} /> },
        ]} />
      </Form>
    </div>
  </Drawer>;
}

export function TodoDrawer({ todoId, overview, overviewError, refresh, onClose, onNotice }) {
  if (!todoId) return null;
  const todo = flattenTodos(overview).find((item) => item.id === todoId);
  if (!todo) return <Drawer open title={`TODO · ${todoId}`} onClose={onClose} placement="right" size="100vw"><UnavailableDetail label="TODO" overview={overview} error={overviewError} refresh={refresh} /></Drawer>;
  return <TodoDrawerSession key={todoId} {...{ todoId, todo, overview, refresh, onClose, onNotice }} />;
}
