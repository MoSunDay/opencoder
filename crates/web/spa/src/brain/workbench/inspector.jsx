import { Alert, Button, Descriptions, Empty, Select, Space, Tabs, Tag, Typography } from 'antd';
import { useEffect, useState } from 'react';
import { apiGet } from '../../api.js';
import { downloadArtifact } from '../../fleet/download.js';
import { ExecutionView } from '../../fleet/detail.jsx';
import { STATES, COLORS } from './model.js';
const JsonView = ({ value }) => <pre className="brain-json">{JSON.stringify(value, null, 2)}</pre>;
export function Inspector({ run, stepId, instanceId, onInstance, onNotice }) {
  const [instance, setInstance] = useState(null); const [error, setError] = useState(''); const [tab, setTab] = useState('overview');
  const step = (run.plan?.plan.schema_version === 2 ? run.plan.plan.instances : run.plan?.plan.steps || []).find((s) => s.id === stepId);
  const [page, setPage] = useState({ instances: [], next_offset: null, total: 0 }); const [offset, setOffset] = useState(0);
  const rows = page.instances;
  useEffect(() => { setOffset(0); }, [stepId]);
  useEffect(() => {
    let alive = true;
    if (stepId) apiGet(`/api/brain/runs/${encodeURIComponent(run.id)}/instances?step=${encodeURIComponent(stepId)}&offset=${offset}`).then((value) => {
      if (!alive) return; setPage(value); if (!instanceId && value.instances.length) onInstance(value.instances[0].id);
    }).catch((e) => { if (alive) setError(e.message); });
    return () => { alive = false; };
  }, [run.id, run.revision, stepId, offset, instanceId, onInstance]);
  useEffect(() => {
    let alive = true; setInstance(null); setError('');
    if (instanceId) apiGet(`/api/brain/runs/${encodeURIComponent(run.id)}/instances/${encodeURIComponent(instanceId)}`).then((value) => { if (alive) setInstance(value); }).catch((e) => { if (alive) setError(e.message); });
    return () => { alive = false; };
  }, [run.id, run.revision, instanceId]);
  const route = run.plan?.plan.routes?.find((r) => `route:${r.id}` === stepId);
  if (route) return <aside className="brain-inspector"><Typography.Title level={5}>路由 {route.id}</Typography.Title><p>{route.description}</p>{Object.values(run.graph?.routes || {}).filter((r) => r.context.route === route.id).map((r) => <div key={r.context.receipt}><strong>读取的 output 与轮次</strong><JsonView value={r.context.outputs} /><strong>选择与判断依据</strong><JsonView value={r.decision || { waiting: '等待局部路由判断' }} /></div>)}</aside>;
  if (!step) return <div className="brain-inspector"><Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="选择步骤查看输入、执行过程和证据" />
    <Typography.Title level={5}>交付物</Typography.Title><JsonView value={run.deliverables} /></div>;
  return <div className="brain-inspector"><Space><Tag>{step.action.kind}</Tag><Typography.Text strong>{step.description || step.label}</Typography.Text></Space>
    <Typography.Paragraph type="secondary">{step.description || step.purpose}</Typography.Paragraph>
    {!!rows.length && <Select style={{ width: '100%', marginBottom: 12 }} value={instanceId} onChange={onInstance} placeholder="选择运行实例" options={rows.map((i) => ({ value: i.id, label: `${i.item_key || i.id} · ${STATES[i.status]}` }))} />}
    {page.total > 100 && <Space><Button size="small" disabled={!offset} onClick={() => { setOffset(Math.max(0, offset - 100)); onInstance(null); }}>上一页实例</Button><span>{offset + 1}–{offset + rows.length} / {page.total}</span><Button size="small" disabled={page.next_offset === null} onClick={() => { setOffset(page.next_offset); onInstance(null); }}>下一页实例</Button></Space>}
    {error && <Alert type="error" title={error} />}
    <Tabs activeKey={tab} onChange={setTab} items={[
      { key: 'overview', label: '概览', children: <><Descriptions column={1} size="small" items={[
        { key: 'status', label: '状态', children: instance ? <Tag color={COLORS[instance.status]}>{STATES[instance.status]}</Tag> : '尚未展开' },
        { key: 'why', label: '当前原因', children: instance?.reason || '—' }, { key: 'attempt', label: '尝试', children: instance?.attempt || 0 },
        { key: 'round', label: '执行轮次', children: instance?.item_key || '—' }, { key: 'node', label: '执行节点', children: instance?.node_id || step.action.node_id || '自动选择' },
        { key: 'target', label: '能力目标', children: step.action.target },
      ]} /><Typography.Title level={5}>动作</Typography.Title><Typography.Paragraph>{step.action.prompt}</Typography.Paragraph>
        <Typography.Title level={5}>依赖与资源</Typography.Title><JsonView value={{ depends_on: step.depends_on, when: step.when, foreach: step.foreach, resources: step.resources }} /></> },
      { key: 'io', label: '输入 / 输出', children: <><Typography.Title level={5}>输入绑定</Typography.Title><JsonView value={step.inputs} /><Typography.Title level={5}>本次输入来源与因果轮次</Typography.Title><JsonView value={run.graph?.visits?.[instanceId]} /><Typography.Title level={5}>本次实际输入</Typography.Title><JsonView value={instance?.inputs} /><Typography.Title level={5}>输出约定</Typography.Title><JsonView value={step.outputs || step.output} /><Typography.Title level={5}>实际输出</Typography.Title><JsonView value={instance?.output?.value} />{Object.values(run.graph?.outputs || {}).filter((o) => o.visit === instanceId).map((o) => <div key={o.id}><strong>{o.output} · 第 {o.round} 轮</strong><p>完成：{assessment(o.value.completion)}</p><p>验证：{assessment(o.value.verification)}</p></div>)}</> },
      { key: 'process', label: '执行过程', children: instance?.execution ? <ExecutionView key={instance.execution.id} executionRef={instance.execution} mode="inline" onNotice={onNotice} managed /> : <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="步骤尚未派发" /> },
      { key: 'evidence', label: '证据', children: <><JsonView value={instance?.output?.evidence || []} />{(instance?.output?.artifacts || []).map((artifact) => <div key={`${artifact.step}/${artifact.file}`}><Typography.Text>{artifact.step} / {artifact.file}</Typography.Text><Typography.Paragraph copyable={{ text: artifact.sha256 }} type="secondary">SHA-256 {artifact.sha256}</Typography.Paragraph><Typography.Text>{artifact.bytes} 字节</Typography.Text> <Button size="small" onClick={async () => { try { await downloadArtifact(artifact.execution.id, artifact.step, artifact.file); } catch (e) { setError(e.message); } }}>下载产物</Button></div>)}</> },
    ]} />
  </div>;
}

function assessment(value) { return value?.passed == null || !value?.evidence?.length ? "未知" : `${value.passed ? "通过" : "未通过"}：${value.evidence.join("；")}`; }
