import { EditButton } from '../../ui/permissions.jsx';
import { Alert, Button, Drawer, Empty, Input, Select, Space, Table, Tag, Typography } from 'antd';
import { useEffect, useRef, useState } from 'react';
import { apiGet } from '../../api.js';
import { useStore } from '../../store.js';
import { draftKey } from './scheduler/draft.js';
import { PlanEditor, PlanPreview } from './scheduler/editor.jsx';

export function Plans({ plans, capabilities, reload, onRun }) {
  const { identity, base } = useStore(); const editorRef = useRef(null);
  const owner = `${base || location.origin}:${identity?.name || 'anonymous'}`;
  const [editor, setEditor] = useState(null); const [view, setView] = useState(null); const [versions, setVersions] = useState([]);
  const [diff, setDiff] = useState(null); const [error, setError] = useState(''); const [search, setSearch] = useState('');
  const request = useRef(0); const [loading, setLoading] = useState(false);
  useEffect(() => () => { request.current += 1; }, []);
  const open = async (p, mode = 'edit') => {
    const current = ++request.current;
    setLoading(true); setError('');
    try {
      const version = await apiGet(`/api/brain/plan-defs/${encodeURIComponent(p.id)}/versions/${p.latest_version}`);
      if (current !== request.current) return;
      if (mode === 'edit') {
        if (![4, 5, 6, 7].includes(version.plan.schema_version)) throw new Error('不支持的计划版本');
        setEditor(version);
      } else {
        const history = await apiGet(`/api/brain/plan-defs/${encodeURIComponent(p.id)}/versions`);
        if (current !== request.current) return;
        setView(version); setDiff(null); setVersions(history.versions);
      }
    } catch (error) { if (current === request.current) setError(error.message); }
    finally { if (current === request.current) setLoading(false); }
  };
  const compare = async () => { try { setDiff(await apiGet(`/api/brain/plan-defs/${encodeURIComponent(view.id)}/diff?from=${view.version - 1}&to=${view.version}`)); } catch (error) { setError(error.message); } };
  return <><Space wrap style={{ marginBottom: 16 }}><Input.Search placeholder="搜索计划名称" value={search} onChange={(event) => setSearch(event.target.value)} /><EditButton type="primary" onClick={() => { request.current += 1; setLoading(false); setError(''); setEditor({ creating: true }); }}>新建计划</EditButton></Space>
    {error && <Alert type="error" showIcon title={error} />}
    <Table rowKey="id" loading={loading} scroll={{ x: 700 }} pagination={{ pageSize: 10 }} dataSource={plans.filter((p) => `${p.title} ${p.id}`.toLowerCase().includes(search.toLowerCase()))} columns={[
      { title: '计划', dataIndex: 'title', render: (title, p) => <Button type="link" onClick={() => open(p, p.schema_version === 7 ? 'edit' : 'history')}>{title}</Button> },
      { title: '版本', dataIndex: 'latest_version', render: (v, p) => <Space>v{v}{p.schema_version !== 7 && <Tag>历史只读</Tag>}</Space> },
      { title: '操作', render: (_, p) => <Space>{[4, 5, 6, 7].includes(p.schema_version) && <Button size="small" onClick={() => open(p)}>{p.schema_version === 7 ? '修改' : '转换为新版里程碑'}</Button>}<Button size="small" onClick={() => open(p, 'history')}>历史版本</Button><EditButton disabled={p.schema_version !== 7} size="small" onClick={() => onRun(`${p.id}@${p.latest_version}`)}>执行</EditButton></Space> },
    ]} locale={{ emptyText: <Empty description="创建计划并关联能力，按轮观察执行" /> }} />
    <Drawer open={!!view} onClose={() => setView(null)} title={`历史版本 · ${view?.plan.title || ''}`} size="85vw">{view && <><Space wrap>
      <Select value={view.version} options={versions.map((v) => ({ value: v.version, label: `v${v.version} · ${v.changelog}` }))} onChange={(version) => { setView(versions.find((v) => v.version === version)); setDiff(null); }} />
      <Button disabled={!versions.length || versions[versions.length - 1].version <= 1} onClick={async () => { try { const page = await apiGet(`/api/brain/plan-defs/${encodeURIComponent(view.id)}/versions?before=${versions[versions.length - 1].version}`); setVersions([...versions, ...page.versions]); } catch (error) { setError(error.message); } }}>更早版本</Button>
      {view.plan.schema_version === 7 ? <EditButton onClick={() => { onRun(`${view.id}@${view.version}`); setView(null); }}>执行此版本</EditButton> : <Tag>历史版本只读</Tag>}
      <Button disabled={view.version < 2} onClick={compare}>对比上一版本</Button>
    </Space><Typography.Paragraph>{view.plan.objective}</Typography.Paragraph>
      <PlanPreview plan={view.plan} capabilities={capabilities} />
      {diff && <pre className="brain-json">{JSON.stringify(diff, null, 2)}</pre>}
    </>}</Drawer>
    <Drawer className="brain-plan-drawer" title={editor?.creating ? '新建计划' : '修改计划'} placement="right" size="100%" open={!!editor} onClose={() => editorRef.current?.close()} destroyOnHidden styles={{ body: { padding: 0 } }}>
      {editor && <PlanEditor ref={editorRef} key={draftKey(owner, editor.creating ? null : editor)} cacheKey={draftKey(owner, editor.creating ? null : editor)} version={editor.creating ? undefined : editor} capabilities={capabilities} onClose={() => setEditor(null)} onSaved={async () => { await reload(); setEditor(null); }} />}
    </Drawer>
  </>;
}
