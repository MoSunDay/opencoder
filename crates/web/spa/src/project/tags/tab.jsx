import { EditButton } from '../../ui/permissions.jsx';
import { Button, Popconfirm, Space, Typography } from 'antd';
import { useRef, useState } from 'react';
import { apiDel, apiPatch, apiPost } from '../../api.js';
import { err, ok } from '../../notice.js';
import { ProjectTable, TableText } from '../views/projectTable.jsx';
import { TagEditor } from './editor.jsx';
import { tagRows } from './model.js';

export function TagsTab({ overview, refresh, onNotice }) {
  const [editing, setEditing] = useState(null);
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const mutating = useRef(false);
  const save = async (body) => {
    if (mutating.current) return false;
    mutating.current = true; setBusy(true);
    try {
      if (editing) await apiPatch(`/api/project/tags/${encodeURIComponent(editing.id)}`, body);
      else await apiPost('/api/project/tags', body);
      setOpen(false); await refresh(); onNotice(ok('Tag 已保存，TODO 标签已按专项优先重新匹配')); return true;
    } catch (error) { onNotice(err(error.message)); return false; }
    finally { mutating.current = false; setBusy(false); }
  };
  const remove = async (row) => {
    if (mutating.current) return;
    mutating.current = true; setBusy(true);
    try { await apiDel(`/api/project/tags/${encodeURIComponent(row.id)}`); await refresh(); onNotice(ok('Tag 已删除，TODO 标签已重新匹配')); }
    catch (error) { onNotice(err(error.message)); }
    finally { mutating.current = false; setBusy(false); }
  };
  const columns = [
    { title: 'Tag 名称', key: 'name', width: '25%', searchValue: (row) => row.name, render: (_, row) => <TableText>{row.name}</TableText> },
    { title: '归属类型', key: 'scope', width: '13%', kind: 'enum', searchValue: (row) => row.scope_label, render: (_, row) => row.scope_label },
    { title: '所属项目', key: 'project', width: '23%', searchValue: (row) => row.project_title, render: (_, row) => <TableText>{row.project_title}</TableText> },
    { title: '所属专项', key: 'initiative', width: '23%', searchValue: (row) => row.initiative_title, render: (_, row) => <TableText>{row.initiative_title}</TableText> },
    { title: '操作', key: 'actions', width: '16%', render: (_, row) => <Space size={4}>
      <EditButton size="small" disabled={busy} aria-label={`编辑 Tag ${row.name}`} onClick={() => { setEditing(row); setOpen(true); }}>编辑</EditButton>
      <Popconfirm title={`删除 Tag「${row.name}」？`} description="同名可用标签会接替，无法匹配的 TODO 标签会解除。" okText="删除" cancelText="取消" onConfirm={() => remove(row)}>
        <EditButton size="small" danger disabled={busy} aria-label={`删除 Tag ${row.name}`}>删除</EditButton>
      </Popconfirm>
    </Space> },
  ];
  return <Space orientation="vertical" size={12} style={{ width: '100%' }}>
    <Space wrap><EditButton type="primary" disabled={busy} onClick={() => { setEditing(null); setOpen(true); }}>新建 Tag</EditButton>
      <Typography.Text type="secondary">专项继承项目 Tag，同名时专项优先。</Typography.Text></Space>
    <ProjectTable label="Tag 表格" viewKey="tags" columns={columns} rows={tagRows(overview)} locale={{ emptyText: '还没有 Tag' }} />
    <TagEditor open={open} initial={editing} overview={overview} onCancel={() => setOpen(false)} onOk={save} />
  </Space>;
}
