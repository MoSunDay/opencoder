import { Alert, Button, Drawer, Empty, Input, Select, Space, Table, Tag, Typography } from 'antd';
import { useRef, useState } from 'react';
import { apiGet, apiPost } from '../../api.js';
import { useStore } from '../../store.js';
import { draftKey } from './editor/draft.js';
import { PlanEditor } from './editor.jsx';
import { PlanCanvas } from './canvas.jsx';
export function Plans({ plans, capabilities, reload, onRun }) {
  const { identity, base } = useStore(); const editorRef = useRef(null);
  const owner = `${base || location.origin}:${identity?.name || 'anonymous'}`;
  const [editor, setEditor] = useState(null); const [view, setView] = useState(null); const [versions, setVersions] = useState([]); const [diff, setDiff] = useState(null); const [error, setError] = useState(''); const [search, setSearch] = useState('');
  const open = async (p, edit = false) => { try { const version = await apiGet(`/api/brain/plan-defs/${encodeURIComponent(p.id)}/versions/${p.latest_version}`); if (edit) setEditor({ ...version, version: p.latest_version + 1 }); else { setView(version); setDiff(null); const history = await apiGet(`/api/brain/plan-defs/${encodeURIComponent(p.id)}/versions`); setVersions(history.versions); } } catch (e) { setError(e.message); } };
  const stable = async () => { try { await apiPost(`/api/brain/plan-defs/${encodeURIComponent(view.id)}/stable`, { version: view.version }); await reload(); } catch (e) { setError(e.message); } };
  return <><Space style={{ marginBottom: 16 }}><Input.Search placeholder="搜索计划名称" value={search} onChange={(e) => setSearch(e.target.value)} /><Button type="primary" onClick={() => setEditor({ creating: true })}>新建计划</Button></Space>
    {error && <Alert type="error" showIcon title={error} />}<Table rowKey="id" pagination={{ pageSize: 10 }} dataSource={plans.filter((p) => `${p.title} ${p.id}`.toLowerCase().includes(search.toLowerCase()))} columns={[
      { title: '计划', dataIndex: 'title', render: (title, p) => <Button type="link" onClick={() => open(p)}>{title}</Button> }, { title: '最新版本', dataIndex: 'latest_version', render: (v) => `v${v}` },
      { title: '稳定版本', dataIndex: 'stable_version', render: (v) => v ? <Tag color="green">v{v} 稳定</Tag> : <Tag>草稿</Tag> },
      { title: '操作', render: (_, p) => <Space><Button size="small" onClick={() => open(p, true)}>创建下一版本</Button><Button size="small" onClick={() => onRun(`${p.id}@${p.latest_version}`)}>执行</Button></Space> },
    ]} locale={{ emptyText: <Empty description="创建固定计划，或让一次动态规划沉淀为可复用版本" /> }} />
    <Drawer open={!!view} onClose={() => setView(null)} title={view?.plan.title} size="85vw">{view && <><Space wrap><Select value={view.version} options={versions.map((v) => ({ value: v.version, label: `v${v.version} · ${v.changelog}` }))} onChange={(version) => { setView(versions.find((v) => v.version === version)); setDiff(null); }} /><Button size="small" disabled={!versions.length || versions.at(-1).version <= 1} onClick={async () => { try { const page = await apiGet(`/api/brain/plan-defs/${encodeURIComponent(view.id)}/versions?before=${versions.at(-1).version}`); setVersions([...versions, ...page.versions]); } catch (e) { setError(e.message); } }}>更早版本</Button><Tag>{view.confidence.level} 置信</Tag>{view.tags.map((t) => <Tag key={t}>{t}</Tag>)}<Button onClick={stable}>标记此版本稳定</Button><Button onClick={() => onRun(`${view.id}@${view.version}`)}>执行此版本</Button><Button disabled={view.version < 2} onClick={async () => { try { setDiff(await apiGet(`/api/brain/plan-defs/${encodeURIComponent(view.id)}/diff?from=${view.version - 1}&to=${view.version}`)); } catch (e) { setError(e.message); } }}>对比上一版本</Button></Space>
      <Typography.Paragraph>{view.changelog}</Typography.Paragraph><Typography.Paragraph type="secondary">{view.confidence.reason}</Typography.Paragraph><PlanCanvas plan={view.plan} mode="ontology" />
      {diff && <pre className="brain-json">{JSON.stringify(diff, null, 2)}</pre>}
    </>}</Drawer>
    <Drawer className="brain-plan-drawer" title={editor?.creating ? '新建计划' : '编辑计划新版本'} placement="right" size="100%" open={!!editor} onClose={() => editorRef.current?.close()} destroyOnHidden styles={{ body: { padding: 0 } }}>{editor && <PlanEditor ref={editorRef} key={draftKey(owner, editor.creating ? null : editor)} cacheKey={draftKey(owner, editor.creating ? null : editor)} version={editor.creating ? undefined : editor} capabilities={capabilities} onClose={() => setEditor(null)} onSaved={async () => { setEditor(null); await reload(); }} />}</Drawer>
  </>;
}
