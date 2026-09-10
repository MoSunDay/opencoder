// Capability library: a single searchable table with a separate create/edit drawer.
import { Alert, Button, Input, Popconfirm, Select, Space, Table, Tag } from 'antd';
import { useCallback, useEffect, useRef, useState } from 'react';
import { apiDel, apiGet, apiPost } from './api.js';
import { KIND_LABELS } from './fleet/model.js';
import { TimeText } from './ui/timeText.jsx';
import { CapabilityEditor } from './brain/capabilityEditor.jsx';
import { ok } from './notice.js';

const K_OPTIONS = [3, 5, 10, 20, 50].map((value) => ({ value, label: `前 ${value} 条` }));

export function BrainPanel({ onNotice }) {
  const [rows, setRows] = useState([]);
  const [loading, setLoading] = useState(false);
  const [editor, setEditor] = useState(null);
  const [query, setQuery] = useState('');
  const [k, setK] = useState(10);
  const [hits, setHits] = useState(null);
  const [searching, setSearching] = useState(false);
  const [error, setError] = useState('');
  const searchVersion = useRef(0);
  const load = useCallback(async () => {
    setLoading(true); setError('');
    try { const result = await apiGet('/api/brain/capabilities'); setRows(result.capabilities || []); }
    catch (e) { setError('获取能力库失败: ' + e.message); }
    finally { setLoading(false); }
  }, []);
  useEffect(() => { load(); }, [load]);

  const resetSearch = () => { searchVersion.current += 1; setHits(null); setQuery(''); setSearching(false); };
  const search = async () => {
    if (!query.trim()) { resetSearch(); return; }
    const version = ++searchVersion.current;
    setSearching(true); setError('');
    try {
      const result = await apiPost('/api/brain/search', { query: query.trim(), k });
      if (version === searchVersion.current) setHits(result.hits || []);
    } catch (e) { if (version === searchVersion.current) setError('搜索失败: ' + e.message); }
    finally { if (version === searchVersion.current) setSearching(false); }
  };
  const remove = async (id) => {
    try { await apiDel(`/api/brain/capabilities/${encodeURIComponent(id)}`); resetSearch(); await load(); onNotice?.(ok('能力已删除')); }
    catch (e) { setError('删除失败: ' + e.message); }
  };
  const edit = (entry) => setEditor({ entry });
  const columns = [
    { title: '执行类型', dataIndex: ['capability', 'capability_type'], width: 140, render: (value) => <Tag>{KIND_LABELS[value] || value}</Tag> },
    { title: '一句话描述', dataIndex: ['capability', 'summary'], ellipsis: true },
    { title: '输入描述', dataIndex: ['capability', 'input_desc'], ellipsis: true },
    { title: '输出描述', dataIndex: ['capability', 'output_desc'], ellipsis: true },
    { title: '工程输入', width: 90, render: (_, row) => (row.eng_inputs || rows.find((entry) => entry.capability.id === row.capability.id)?.eng_inputs)?.length ?? '—' },
    ...(hits ? [{ title: '距离', dataIndex: 'distance', width: 90, render: (value) => typeof value === 'number' ? value.toFixed(4) : '—' }] : []),
    { title: '更新时间', dataIndex: ['capability', 'updated_at'], width: 130, render: (value) => <TimeText ts={value} /> },
    { title: '操作', width: 130, fixed: 'right', render: (_, row) => <Space onClick={(event) => event.stopPropagation()}>
      <Button type="link" size="small" onClick={() => edit(row)}>编辑</Button>
      <Popconfirm title="删除该能力？" okText="确认删除" cancelText="取消" okButtonProps={{ danger: true }} onConfirm={() => remove(row.capability.id)}>
        <Button type="link" size="small" danger>删除</Button>
      </Popconfirm>
    </Space> },
  ];
  return <>
    <div style={{ display: 'flex', gap: 12, flexWrap: 'wrap', justifyContent: 'space-between', marginBottom: 16 }}>
      <Space wrap>
        <Input aria-label="搜索能力" placeholder="按意图搜索能力，如：解析依赖图" value={query} onChange={(event) => setQuery(event.target.value)} onPressEnter={search} style={{ width: 300, maxWidth: '100%' }} />
        <Select aria-label="搜索结果数量" value={k} options={K_OPTIONS} onChange={setK} style={{ width: 100 }} />
        <Button loading={searching} onClick={search}>搜索</Button>
        {hits !== null && <Button onClick={resetSearch}>显示全部</Button>}
      </Space>
      <Space><Button onClick={load} loading={loading}>刷新</Button><Button type="primary" onClick={() => setEditor({ entry: null })}>新建能力</Button></Space>
    </div>
    {error && <Alert type="error" showIcon title={error} style={{ marginBottom: 12 }} />}
    <Table rowKey={(row) => row.capability.id} size="middle" columns={columns} dataSource={hits ?? rows}
      loading={loading || searching} scroll={{ x: 1000 }} pagination={{ pageSize: 10, showTotal: (total) => `共 ${total} 项` }}
      onRow={(row) => ({ onClick: () => edit(row), style: { cursor: 'pointer' } })}
      locale={{ emptyText: hits !== null ? '没有匹配的能力' : '暂无能力，点击「新建能力」添加' }} />
    {editor && <CapabilityEditor key={editor.entry?.capability.id || 'create'} entry={editor.entry} onClose={() => setEditor(null)}
      onSaved={() => { setEditor(null); resetSearch(); load(); onNotice?.(ok('能力已保存')); }} />}
  </>;
}
