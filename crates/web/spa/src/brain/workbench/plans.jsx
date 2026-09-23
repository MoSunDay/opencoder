import { Alert, Button, Drawer, Empty, Input, Select, Space, Table, Tag, Typography } from 'antd';
import { useRef, useState } from 'react';
import { apiGet } from '../../api.js';
import { useStore } from '../../store.js';
import { draftKey } from './scheduler/draft.js';
import { PlanEditor, PlanPreview } from './scheduler/editor.jsx';

export function Plans({ plans, capabilities, reload, onRun }) {
  const { identity, base } = useStore(); const editorRef = useRef(null);
  const owner = `${base || location.origin}:${identity?.name || 'anonymous'}`;
  const [editor, setEditor] = useState(null); const [view, setView] = useState(null); const [versions, setVersions] = useState([]);
  const [diff, setDiff] = useState(null); const [error, setError] = useState(''); const [search, setSearch] = useState('');
  const open = async (p, edit = false) => {
    try {
      const version = await apiGet(`/api/brain/plan-defs/${encodeURIComponent(p.id)}/versions/${p.latest_version}`);
      if (edit) { if (![4, 5].includes(version.plan.schema_version)) throw new Error('不支持的计划版本'); setEditor(version); }
      else { setView(version); setDiff(null); const history = await apiGet(`/api/brain/plan-defs/${encodeURIComponent(p.id)}/versions`); setVersions(history.versions); }
      setError('');
    } catch (error) { setError(error.message); }
  };
  const compare = async () => { try { setDiff(await apiGet(`/api/brain/plan-defs/${encodeURIComponent(view.id)}/diff?from=${view.version - 1}&to=${view.version}`)); } catch (error) { setError(error.message); } };
  return <><Space style={{ marginBottom: 16 }}><Input.Search placeholder="搜索计划名称" value={search} onChange={(event) => setSearch(event.target.value)} /><Button type="primary" onClick={() => setEditor({ creating: true })}>新建计划</Button></Space>
    {error && <Alert type="error" showIcon title={error} />}
    <Table rowKey="id" pagination={{ pageSize: 10 }} dataSource={plans.filter((p) => `${p.title} ${p.id}`.toLowerCase().includes(search.toLowerCase()))} columns={[
      { title: '计划', dataIndex: 'title', render: (title, p) => <Button type="link" onClick={() => open(p)}>{title}</Button> },
      { title: '版本', dataIndex: 'latest_version', render: (v, p) => <Space>v{v}{p.schema_version !== 5 && <Tag>{p.schema_version === 4 ? "历史 DAG" : "不支持"}</Tag>}</Space> },
      { title: '操作', render: (_, p) => [4, 5].includes(p.schema_version) && <Space><Button size="small" onClick={() => open(p, true)}>{p.schema_version === 4 ? "转换为里程碑版本" : "创建下一版本"}</Button><Button disabled={p.schema_version !== 5} size="small" onClick={() => onRun(`${p.id}@${p.latest_version}`)}>执行</Button></Space> },
    ]} locale={{ emptyText: <Empty description="创建计划并关联能力，按轮观察执行" /> }} />
    <Drawer open={!!view} onClose={() => setView(null)} title={view?.plan.title} size="85vw">{view && <><Space wrap>
      <Select value={view.version} options={versions.map((v) => ({ value: v.version, label: `v${v.version} · ${v.changelog}` }))} onChange={(version) => { setView(versions.find((v) => v.version === version)); setDiff(null); }} />
      <Button disabled={!versions.length || versions.at(-1).version <= 1} onClick={async () => { try { const page = await apiGet(`/api/brain/plan-defs/${encodeURIComponent(view.id)}/versions?before=${versions.at(-1).version}`); setVersions([...versions, ...page.versions]); } catch (error) { setError(error.message); } }}>更早版本</Button>
      {view.plan.schema_version === 5 ? <Button onClick={() => { onRun(`${view.id}@${view.version}`); setView(null); }}>执行此版本</Button> : <Tag>不支持的计划版本</Tag>}
      <Button disabled={view.version < 2} onClick={compare}>对比上一版本</Button>
    </Space><Typography.Paragraph>{view.plan.objective}</Typography.Paragraph>
      <PlanPreview plan={view.plan} />
      {diff && <pre className="brain-json">{JSON.stringify(diff, null, 2)}</pre>}
    </>}</Drawer>
    <Drawer className="brain-plan-drawer" title={editor?.creating ? '新建计划' : '编辑计划新版本'} placement="right" size="100%" open={!!editor} onClose={() => editorRef.current?.close()} destroyOnHidden styles={{ body: { padding: 0 } }}>
      {editor && <PlanEditor ref={editorRef} key={draftKey(owner, editor.creating ? null : editor)} cacheKey={draftKey(owner, editor.creating ? null : editor)} version={editor.creating ? undefined : editor} capabilities={capabilities} onClose={() => setEditor(null)} onSaved={async () => { await reload(); setEditor(null); }} />}
    </Drawer>
  </>;
}
