import { Button, Input, Popconfirm, Space, Tag, Typography } from 'antd';
import { useState } from 'react';
import { apiDel, apiPatch, apiPost } from '../../api.js';
import { err, ok } from '../../notice.js';

export function TagManager(props) { return <TagEditor key={`${props.scopeType}:${props.scopeId}`} {...props} />; }

function TagEditor({ overview, scopeType, scopeId, refresh, onNotice }) {
  const [name, setName] = useState('');
  const [editing, setEditing] = useState(null);
  const [busy, setBusy] = useState(false);
  const tags = (overview?.tags || []).filter((tag) => tag.scope_type === scopeType && tag.scope_id === scopeId);
  const save = async () => {
    if (!name.trim() || busy) return;
    setBusy(true);
    try {
      if (editing) await apiPatch(`/api/project/tags/${encodeURIComponent(editing)}`, { name: name.trim() });
      else await apiPost('/api/project/tags', { name: name.trim(), scope_type: scopeType, scope_id: scopeId });
      setName(''); setEditing(null); await refresh(); onNotice(ok('Tag 已保存，同名标签已按专项优先更新'));
    } catch (error) { onNotice(err(error.message)); } finally { setBusy(false); }
  };
  const remove = async (tag) => {
    setBusy(true);
    try {
      await apiDel(`/api/project/tags/${encodeURIComponent(tag.id)}`); await refresh();
      if (editing === tag.id) { setEditing(null); setName(''); }
      onNotice(ok('Tag 已删除，TODO 标签已重新匹配'));
    } catch (error) { onNotice(err(error.message)); } finally { setBusy(false); }
  };
  return <section className="project-tag-manager" aria-label={`${scopeType === 'project' ? '项目' : '专项'} Tag 管理`}>
    <Typography.Title level={5}>Tag 管理</Typography.Title>
    <Space wrap>{tags.map((tag) => <Space key={tag.id} size={0}>
      <Tag>{tag.name}</Tag><Button type="link" size="small" disabled={busy} aria-label={`编辑 Tag ${tag.name}`} onClick={() => { setEditing(tag.id); setName(tag.name); }}>编辑</Button>
      <Popconfirm title={`删除 Tag「${tag.name}」？`} description="同名项目标签可继续使用，无法匹配的 TODO 标签会解除。" onConfirm={() => remove(tag)}><Button type="link" danger size="small" disabled={busy} aria-label={`删除 Tag ${tag.name}`}>删除</Button></Popconfirm>
    </Space>)}</Space>
    <Space.Compact style={{ width: '100%', marginTop: 10 }}><Input aria-label="Tag 名称" maxLength={128} placeholder="输入 Tag 名称" value={name} onChange={(event) => setName(event.target.value)} onPressEnter={save} disabled={busy} />
      <Button type="primary" onClick={save} loading={busy} disabled={!scopeId || !name.trim()}>{editing ? '保存 Tag' : '新建 Tag'}</Button></Space.Compact>
    {editing && <Button type="link" onClick={() => { setEditing(null); setName(''); }}>取消编辑</Button>}
  </section>;
}
