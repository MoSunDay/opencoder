import { Alert, Button, Form, Modal, Space, Spin, Tag, Typography } from 'antd';
import { useEffect, useRef, useState } from 'react';
import dayjs from 'dayjs';
import { apiDel, apiPatch, apiPost } from '../../api.js';
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
  const [editing, setEditing] = useState(null);
  const [deleting, setDeleting] = useState(null);
  useEffect(() => { onDraftChange(!!body || !!editing); }, [body, editing, onDraftChange]);
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
      list.merge([comment]); attempt.current = null; setBody('');
      await refresh();
    } catch (failure) { setError(failure.message); }
    finally { submitting.current = false; setSaving(false); onBusyChange(false); }
  };
  const change = async (comment, deleting = false) => {
    if (!canEdit || submitting.current) return;
    const body_md = editing?.body;
    if (!deleting && (!body_md?.trim() || new TextEncoder().encode(body_md).length > 65536)) {
      setError('评论不能为空且须在 64 KiB 以内'); return;
    }
    submitting.current = true; setSaving(true); onBusyChange(true); setError('');
    const path = `/api/project/todos/${encodeURIComponent(todoId)}/comments/${comment.seq}`;
    try {
      if (deleting) {
        await apiDel(`${path}?expected_revision=${comment.revision}`);
        list.remove(comment.seq);
        if (editing?.seq === comment.seq) setEditing(null);
      } else {
        list.merge([await apiPatch(path, { body_md, expected_revision: editing.revision })]);
        setEditing(null);
      }
      await refresh();
    } catch (failure) { setError(failure.message); }
    finally { if (deleting) setDeleting(null); submitting.current = false; setSaving(false); onBusyChange(false); }
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
        {comment.revision > 1 && <Typography.Text type="secondary">已编辑</Typography.Text>}
        {canEdit && (identity?.role === 'admin' || identity?.name === comment.submitted_by) && <>
          <Button size="small" disabled={saving || !!editing} onClick={() => { setEditing({ seq: comment.seq, body: comment.body_md, revision: comment.revision }); setError(''); }}>编辑评论</Button>
          <Button size="small" danger disabled={saving} onClick={() => setDeleting(comment.seq)}>删除评论</Button>
          <Modal title="删除这条评论？" open={deleting === comment.seq} okText="删除" cancelText="取消" confirmLoading={saving} cancelButtonProps={{ disabled: saving }} closable={!saving} mask={{ closable: !saving }} onCancel={() => setDeleting(null)} onOk={() => change(comment, true)}>
            删除后不再展示此评论，未保存的编辑也会丢弃。
          </Modal>
        </>}
        {comment.execution_id && <Button type="link" onClick={() => openExecution(comment.execution_id)}>查看执行来源</Button>}
      </Space>
      {editing?.seq === comment.seq ? <Space direction="vertical" style={{ width: '100%' }}>
        <MarkdownInput id={`edit-comment-${comment.seq}`} label="编辑评论正文" minRows={3} maxRows={12} value={editing.body} disabled={saving} onChange={(value) => setEditing((old) => ({ ...old, body: value }))} />
        <Space>
          <Button type="primary" disabled={!editing.body.trim()} loading={saving} onClick={() => change(comment)}>保存评论</Button>
          <Button disabled={saving} onClick={() => setEditing(null)}>取消编辑</Button>
        </Space>
      </Space> : <Markdown text={comment.body_md} />}
    </article>)}</div>
    {error && <Alert style={{ marginBottom: 12 }} type="error" title={error} />}
    {canEdit && <Form layout="vertical" component="div">
      <Form.Item label={`追加评论 · ${identity?.name || '当前用户'}`} htmlFor={`comment-${todoId}`}>
        <MarkdownInput id={`comment-${todoId}`} label="评论正文" minRows={3} maxRows={12} value={body} disabled={saving} onChange={setBody} placeholder="补充进展、讨论或说明，支持 Markdown" />
      </Form.Item>
      <EditButton type="primary" disabled={!body.trim() || !!editing} loading={saving} onClick={submit}>发表评论</EditButton>
    </Form>}
  </div>;
}
