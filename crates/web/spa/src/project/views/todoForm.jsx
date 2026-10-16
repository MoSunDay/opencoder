import { EditButton, useCanEdit } from '../../ui/permissions.jsx';
import { Alert, Button, Drawer, Form, Tabs } from 'antd';
import { useRef, useState } from 'react';
import { apiPost } from '../../api.js';
import { err, ok } from '../../notice.js';
import { TodoActions } from '../forms/section.jsx';
import { TodoTaskFields, TodoSettingsFields } from '../forms/editor.jsx';
import { useCloseGuard } from '../forms/closeGuard.js';

export function CreateTodo({ open, overview, initiativeId, onClose, onCreated, onNotice }) {
  const canEdit = useCanEdit();
  const [form] = Form.useForm();
  const [tab, setTab] = useState('task');
  const [saving, setSaving] = useState(false);
  const submitting = useRef(false);
  const [dirty, setDirty] = useState(false);
  const [error, setError] = useState('');
  const [close, holder] = useCloseGuard({ dirty, busy: saving, onClose, active: open });
  const create = async (values) => {
    if (!canEdit || submitting.current) return;
    submitting.current = true; setSaving(true); setError('');
    try {
      const todo = await apiPost('/api/project/todos', { title: values.title.trim(), draft: values.draft || '', initiative_id: values.initiative_id || null,
        board_status: values.board_status, tag_ids: values.tag_ids || [], owner_user_name: values.owner_user_name ?? null, follower_user_names: values.follower_user_names || [] });
      setDirty(false); form.resetFields(); onNotice(ok('TODO 已创建')); await onCreated(todo.id);
    } catch (error) { setError(error.message); onNotice(err(error.message)); }
    finally { submitting.current = false; setSaving(false); }
  };
  return <Drawer open={open} title="新建 TODO" onClose={close} size="min(760px, 100vw)" destroyOnHidden
    footer={<TodoActions>{error && <Alert type="error" showIcon title={error} />}<Button disabled={saving} onClick={close}>取消</Button><EditButton type="primary" loading={saving} onClick={() => form.submit()}>创建</EditButton></TodoActions>}>
    {holder}
    <Form form={form} layout="vertical" disabled={!canEdit || saving} onFinish={create} onFinishFailed={() => { setTab('task'); requestAnimationFrame(() => form.scrollToField('title', { focus: true })); }}
      initialValues={{ initiative_id: initiativeId || null, board_status: 'todo', tag_ids: [], title: '', draft: '' }}
      onValuesChange={(changed) => { setDirty(true); if ('initiative_id' in changed) form.setFieldValue('tag_ids', []); }}>
      <Tabs activeKey={tab} onChange={setTab} items={[
        { key: 'task', label: '任务', forceRender: true, children: <TodoTaskFields creating /> },
        { key: 'settings', label: '设置', forceRender: true, children: <TodoSettingsFields overview={overview} /> },
      ]} />
    </Form>
  </Drawer>;
}
