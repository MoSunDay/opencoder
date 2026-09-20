import { newId } from '../../../fleet/model.js';
import { Alert, Button, Empty, Space, Tabs, Typography } from 'antd';
import { forwardRef, useImperativeHandle, useState } from 'react';
import { apiPost } from '../../../api.js';
import { PlanCanvas } from './canvas.jsx';
import { useDraft } from './editor/draft.js';
import { appendAction, checkDraftPlan, removeAction, repairPlan, submission } from './editor/model.js';
import { InstanceEditor } from './editor/instance.jsx';
import { RouteEditor } from './editor/route.jsx';
import { JsonField } from './editor/fields.jsx';
import { PublishDialog } from './editor/publish.jsx';
export const PlanEditor = forwardRef(function PlanEditor({ version, cacheKey = `oc:brain:v2:${version?.id || 'new'}`, capabilities, onSaved, onClose }, ref) {
  const { draft, setDraft, error: cacheError, persist, retry, clear, discard } = useDraft(cacheKey, version);
  const [error, setError] = useState(''); const [publishing, setPublishing] = useState(false); const [busy, setBusy] = useState(false);
  const close = () => { if (!busy && (!draft || persist())) onClose(); };
  useImperativeHandle(ref, () => ({ close }));
  if (!draft) return <Alert type="error" title="无法读取浏览器草稿" description={cacheError} action={<Space><Button onClick={retry}>重试读取</Button><Button danger onClick={discard}>丢弃缓存并重新开始</Button><Button onClick={onClose}>关闭</Button></Space>} />;
  const plan = draft.version.plan;
  if (plan.schema_version !== 2) return <Alert type="warning" title="旧计划只读，请新建 v2 计划" />;
  const instance = plan.instances.find((i) => i.id === draft.selected);
  const route = plan.routes.find((r) => `route:${r.id}` === draft.selected);
  const setPlan = (next) => setDraft((d) => ({ ...d, version: { ...d.version, plan: next } }));
  const select = (selected) => setDraft((d) => ({ ...d, selected }));
  const invalid = Object.values(draft.raw).some((f) => f.error);
  const save = async (metadata) => { setBusy(true); setError(''); try { if (!persist()) return; const value = submission({ ...draft, metadata }, capabilities); await apiPost('/api/brain/plan-defs/validate', value.plan); const result = await apiPost('/api/brain/plan-defs', value); clear(); onSaved(result); } catch (e) { setError(e.message); } finally { setBusy(false); } };
  return <div className="brain-plan-editor"><div className="brain-editor-toolbar"><Space><Button onClick={close}>关闭画布</Button><Typography.Text>草稿保存在此浏览器</Typography.Text></Space><Space>
    <Button disabled={busy || plan.instances.length > 0} onClick={() => { try { const next = repairPlan(capabilities); setPlan(next); select('fix'); } catch (e) { setError(e.message); } }}>修复—复测循环示例</Button><Button type="primary" disabled={invalid || busy || !!cacheError} onClick={() => setPublishing(true)}>提交计划</Button></Space></div>
    {((error && !publishing) || cacheError) && <Alert type="error" title={cacheError || error} action={cacheError && <Button onClick={persist}>重试缓存</Button>} />}
    <Tabs items={[{ key: 'canvas', label: 'input → 实例 → output → 路由', children: <div className="brain-plan-workspace"><aside className="brain-entity-library"><Typography.Text strong>已注册能力</Typography.Text>
      {!capabilities.length && <Empty description="请先注册能力" />}{capabilities.filter((c) => c.target).map((entity) => <div className="brain-entity-card" key={entity.id}><strong>{entity.summary || '缺少能力描述'}</strong><small>{entity.kind} · {entity.target}</small><Button size="small" disabled={invalid || !entity.summary?.trim()} onClick={() => { const id = newId('instance'); setPlan(appendAction(plan, entity, id)); select(id); }}>添加实例</Button></div>)}
      <Button onClick={() => { const id = newId('route'); setPlan({ ...plan, routes: [...plan.routes, { id, description: '说明如何选择下一实例', outputs: [], targets: [], exits: [] }] }); select(`route:${id}`); }}>添加路由</Button>
      {plan.routes.map((r) => <Button block key={r.id} onClick={() => select(`route:${r.id}`)}>{r.id}</Button>)}
    </aside><PlanCanvas plan={plan} mode="ontology" selected={draft.selected} positions={draft.positions} viewport={draft.viewport} onSelect={select} onPositions={(positions) => setDraft((d) => ({ ...d, positions: { ...d.positions, ...positions } }))} onViewport={(viewport) => setDraft((d) => ({ ...d, viewport }))} />
    <aside className="brain-inspector">{instance ? <><Space><strong>实例</strong><Button danger size="small" onClick={() => { setPlan(removeAction(plan, instance.id)); select(null); }}>删除实例</Button></Space><InstanceEditor instance={instance} plan={plan} capabilities={capabilities} onChange={setPlan} /></> : route ? <><strong>局部语义路由</strong><RouteEditor route={route} plan={plan} onChange={setPlan} /></> : <Typography.Paragraph>选择实例编辑输入与多个输出；选择路由连接输出、配置相邻实例与结束出口。汇合只等待已激活分支，回流创建新轮次。</Typography.Paragraph>}</aside></div> },
    { key: 'source', label: '完整契约', children: <JsonField label="计划 JSON" field="plan" value={plan} raw={draft.raw.plan} onRaw={(key, value) => setDraft((d) => ({ ...d, raw: { ...d.raw, [key]: value } }))} onChange={(value) => setPlan(checkDraftPlan(value))} /> }]} />
    {publishing && <PublishDialog metadata={draft.metadata} busy={busy} error={error || cacheError} onChange={(metadata) => setDraft((d) => ({ ...d, metadata }))} onSubmit={save} onClose={() => setPublishing(false)} />}
  </div>;
});
