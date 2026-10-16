import { Alert, Button, Form, Space, Spin, Tag, Typography } from 'antd';
import { useRef, useState } from 'react';
import dayjs from 'dayjs';
import { apiPost } from '../../api.js';
import { useStore } from '../../store.js';
import { EditButton, useCanEdit } from '../../ui/permissions.jsx';
import { newId, KIND_LABELS } from '../../fleet/model.js';
import { Markdown } from '../markdown.jsx';
import { MarkdownInput } from '../forms/markdownInput.jsx';
import { useComments } from './useComments.js';

export function TodoComments({ todoId, active, onDraftChange, onBusyChange, openExecution, refresh }) {
  const canEdit = useCanEdit();
  const { identity } = useStore();
  const list = useComments(todoId, active);
  const [body, setBody] = useState('');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const attempt = useRef(null);
  const submitting = useRef(false);
  const root = useRef(null);
  const submit = async () => {
    if (!canEdit || submitting.current || !body.trim()) return;
    if (new TextEncoder().encode(body).length > 65536) { setError('评论须在 64 KiB 以内'); return; }
    if (!attempt.current || attempt.current.body_md !== body) attempt.current = { request_id: newId('comment'), body_md: body };
    submitting.current = true; setSaving(true); onBusyChange(true); setError('');
    try {
      const comment = await apiPost(`/api/project/todos/${encodeURIComponent(todoId)}/comments`, attempt.current);
      list.merge([comment]); attempt.current = null; setBody(''); onDraftChange(false);
      await refresh();
    } catch (failure) { setError(failure.message); }
    finally { submitting.current = false; setSaving(false); onBusyChange(false); }
  };
  const older = async () => {
    const anchor = root.current?.querySelector('[data-comment-seq]');
    const top = anchor?.getBoundingClientRect().top;
    await list.loadOlder();
    requestAnimationFrame(() => {
      const scroll = root.current?.closest('.ant-drawer-body');
      if (scroll && anchor && top != null) scroll.scrollTop += anchor.getBoundingClientRect().top - top;
    });
  };
  return <div ref={root} className="project-comments">
    {list.error && <Alert type="error" title={list.error} action={<Button onClick={list.reload}>重试评论读取</Button>} />}
    {list.older && <Button loading={list.loading} onClick={older}>加载更早评论</Button>}
    {list.added > 0 && <Typography.Text type="secondary">已收到 {list.added} 条新评论</Typography.Text>}
    {list.loading && <Spin aria-label="正在读取评论" />}
    {!list.loading && !list.error && list.rows.length === 0 && <Typography.Text type="secondary">暂无评论</Typography.Text>}
    <div aria-label="评论记录">{list.rows.map((comment) => <article key={comment.seq} data-comment-seq={comment.seq} className="project-comment">
      <Space wrap>
        <Typography.Text strong>{comment.author_name}</Typography.Text>
        <Tag>{comment.author_kind === 'capability' ? KIND_LABELS[comment.capability_kind] || comment.capability_kind : '用户'}</Tag>
        {comment.author_kind === 'capability' && <Typography.Text type="secondary">{comment.author_id} · 提交用户：{comment.submitted_by}</Typography.Text>}
        <Typography.Text type="secondary">{dayjs(comment.created_at).format('YYYY-MM-DD HH:mm:ss')}</Typography.Text>
        {comment.execution_id && <Button type="link" onClick={() => openExecution(comment.execution_id)}>查看执行来源</Button>}
      </Space>
      <Markdown text={comment.body_md} />
    </article>)}</div>
    {canEdit && <Form layout="vertical" component="div">
      <Form.Item label={`追加评论 · ${identity?.name || '当前用户'}`} htmlFor={`comment-${todoId}`}>
        <MarkdownInput id={`comment-${todoId}`} label="评论正文" minRows={3} maxRows={12} value={body} disabled={saving} onChange={(value) => { setBody(value); onDraftChange(!!value); }} placeholder="补充进展、讨论或说明，支持 Markdown" />
      </Form.Item>
      {error && <Alert style={{ marginBottom: 12 }} type="error" title={error} />}
      <EditButton type="primary" disabled={!body.trim()} loading={saving} onClick={submit}>发表评论</EditButton>
    </Form>}
  </div>;
}
