import { EditButton } from './ui/permissions.jsx';
// Capability library: a single searchable table with a separate create/edit drawer.
import { Alert, Button, Input, Popconfirm, Select, Space, Table, Tag } from 'antd';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { apiDel, apiGet, apiPost } from './api.js';
import { KIND_LABELS } from './fleet/model.js';
import { TimeText } from './ui/timeText.jsx';
import { CapabilityEditor } from './brain/capabilityEditor.jsx';
import { ok } from './notice.js';
import './brain/library.css';

const K_OPTIONS = [3, 5, 10, 20, 50].map((value) => ({ value, label: `前 ${value} 条` }));

export function BrainPanel({ onNotice, executionCatalog = null, catalogLoading = false, catalogError = '', onRefreshCatalog }) {
  const [rows, setRows] = useState([]);
  const [loading, setLoading] = useState(false);
  const [editor, setEditor] = useState(null);
  const [query, setQuery] = useState('');
  const [k, setK] = useState(10);
  const [hits, setHits] = useState(null);
  const [searching, setSearching] = useState(false);
  const [error, setError] = useState('');
  const [pageSize, setPageSize] = useState(10);
  const [current, setCurrent] = useState(1);
  const searchVersion = useRef(0);
  const listRequest = useRef(null);
  const targets = useMemo(() => new Map((executionCatalog || []).map((item) => [item.id, item])), [executionCatalog]);
  const data = hits ?? rows;
  const currentPage = Math.min(current, Math.max(1, Math.ceil(data.length / pageSize)));
  useEffect(() => { setCurrent(currentPage); }, [currentPage]);
  const load = useCallback(async () => {
    listRequest.current?.abort();
    const request = new AbortController();
    listRequest.current = request;
    setLoading(true); setError('');
    try {
      const result = await apiGet('/api/brain/capabilities', { signal: request.signal });
      if (!request.signal.aborted) setRows(result.capabilities || []);
    } catch (e) { if (!request.signal.aborted) setError('获取能力库失败: ' + e.message); }
    finally { if (!request.signal.aborted) setLoading(false); }
  }, []);
  useEffect(() => {
    load();
    return () => { listRequest.current?.abort(); searchVersion.current += 1; };
  }, [load]);
  const refresh = () => Promise.all([load(), onRefreshCatalog?.()]);

  const resetSearch = (resetPage = true) => {
    searchVersion.current += 1; setHits(null); setQuery(''); setSearching(false);
    if (resetPage) setCurrent(1);
  };
  const search = async () => {
    if (!query.trim()) { resetSearch(); return; }
    const version = ++searchVersion.current;
    setSearching(true); setError('');
    try {
      const result = await apiPost('/api/brain/search', { query: query.trim(), k });
      if (version === searchVersion.current) { setHits(result.hits || []); setCurrent(1); }
    } catch (e) { if (version === searchVersion.current) setError('搜索失败: ' + e.message); }
    finally { if (version === searchVersion.current) setSearching(false); }
  };
  const remove = async (id) => {
    try { await apiDel(`/api/brain/capabilities/${encodeURIComponent(id)}`); resetSearch(hits !== null); await refresh(); onNotice?.(ok('能力已删除')); }
    catch (e) { setError('删除失败: ' + e.message); }
  };
  const edit = (entry) => setEditor({ entry });
  const bindingText = (row, field) => {
    if (catalogError) return '加载失败';
    if (catalogLoading || executionCatalog === null) return '加载中';
    const target = targets.get(row.capability.id);
    if (!target) return '未绑定';
    return field === 'kind' ? (KIND_LABELS[target.kind] || target.kind) : target.target;
  };
  const columns = [
    { title: '执行类型', key: 'kind', width: 140,
      render: (_, row) => <Tag className="brain-capability-type">{bindingText(row, 'kind')}</Tag> },
    { title: '执行名称', key: 'target', width: 260,
      render: (_, row) => <span className="brain-capability-name">{bindingText(row, 'target')}</span> },
    { title: '一句话描述', dataIndex: ['capability', 'summary'], ellipsis: true,
      render: (summary) => <span className="brain-capability-summary" title={summary}>{summary}</span> },
    ...(hits ? [{ title: '距离', dataIndex: 'distance', width: 90, render: (value) => typeof value === 'number' ? value.toFixed(4) : '—' }] : []),
    { title: '更新时间', dataIndex: ['capability', 'updated_at'], width: 130, render: (value) => <TimeText ts={value} /> },
    { title: '操作', width: 130, fixed: 'right', render: (_, row) => <Space onClick={(event) => event.stopPropagation()}>
      <EditButton type="link" size="small" onClick={() => edit(row)}>编辑</EditButton>
      <Popconfirm title="删除该能力？" okText="确认删除" cancelText="取消" okButtonProps={{ danger: true }} onConfirm={() => remove(row.capability.id)}>
        <EditButton type="link" size="small" danger>删除</EditButton>
      </Popconfirm>
    </Space> },
  ];
  return <>
    <div style={{ display: 'flex', gap: 12, flexWrap: 'wrap', justifyContent: 'space-between', marginBottom: 16 }}>
      <Space wrap>
        <Input aria-label="搜索能力" placeholder="按意图搜索能力，如：解析依赖图" value={query} onChange={(event) => setQuery(event.target.value)} onPressEnter={search} style={{ width: 300, maxWidth: '100%' }} />
        <Select aria-label="搜索结果数量" value={k} options={K_OPTIONS} onChange={setK} style={{ width: 100 }} />
        <Button loading={searching} onClick={search}>搜索</Button>
        {hits !== null && <Button onClick={() => resetSearch()}>显示全部</Button>}
      </Space>
      <Space><Button onClick={refresh} loading={loading || catalogLoading}>刷新</Button><EditButton type="primary" onClick={() => setEditor({ entry: null })}>新建能力</EditButton></Space>
    </div>
    {error && <Alert type="error" showIcon title={error} style={{ marginBottom: 12 }} />}
    {catalogError && <Alert type="error" showIcon title={`获取执行绑定失败: ${catalogError}`} action={<Button onClick={onRefreshCatalog}>重试</Button>} style={{ marginBottom: 12 }} />}
    <Table className="brain-capability-table" rowKey={(row) => row.capability.id} size="middle" tableLayout="fixed" columns={columns} dataSource={data}
      loading={loading || searching} scroll={{ x: hits ? 1110 : 1020 }} pagination={{ current: currentPage, pageSize, showSizeChanger: true, pageSizeOptions: [10, 20, 50, 100],
        onChange: (page, size) => { setPageSize(size); setCurrent(size === pageSize ? page : 1); }, showTotal: (total) => `共 ${total} 项` }}
      onRow={(row) => ({ onClick: () => edit(row), style: { cursor: 'pointer' } })}
      locale={{ emptyText: hits !== null ? '没有匹配的能力' : '暂无能力，点击「新建能力」添加' }} />
    {editor && <CapabilityEditor key={editor.entry?.capability.id || 'create'} entry={editor.entry} onClose={() => setEditor(null)}
      onSaved={() => { setEditor(null); resetSearch(hits !== null); refresh(); onNotice?.(ok('能力已保存')); }} />}
  </>;
}
