import { Alert, Button, Space, Spin, Typography } from 'antd';
import { useEffect, useRef, useState } from 'react';
import { apiGet } from '../api.js';
import { absTime } from '../format.js';
import { RunStatusTag, NodeBadge } from './runBits.jsx';
import { ExecutionDetail } from '../fleet/detail.jsx';
import { DagRunResult } from './run/result.jsx';
import { isActive } from './run/model.js';

export function RunDetail({ run, onNotice, onClose, onFinished }) {
  const [detail, setDetail] = useState(null);
  const [error, setError] = useState('');
  const [current, setCurrent] = useState(run);
  const [revision, setRevision] = useState(0);
  const [executionOpen, setExecutionOpen] = useState(false);
  const previous = useRef(run.status);
  useEffect(() => {
    const controller = new AbortController();
    setDetail(null); setError(''); setCurrent(run); previous.current = run.status;
    apiGet('/api/executions/' + encodeURIComponent(run.id), { signal: controller.signal }).then((value) => {
      if (controller.signal.aborted) return;
      const spec = value?.definition?.spec || value?.definition;
      if (!Array.isArray(spec?.steps)) throw new Error('执行缺少工作流定义快照');
      setDetail(value); setCurrent((old) => ({ ...old, ...value.execution, error: value.error }));
    }).catch((e) => { if (!controller.signal.aborted) setError(e.message); });
    return () => controller.abort();
  }, [run.id, revision]);
  const updateStatus = (status, executionError) => {
    if (isActive(previous.current) && !isActive(status)) onFinished?.();
    previous.current = status;
    setCurrent((old) => ({ ...old, status, error: executionError === undefined ? old.error : executionError }));
  };
  return <Space orientation="vertical" size={12} style={{ width: '100%' }}>
    <Space wrap>
      <Button size="small" onClick={onClose}>← 返回运行列表</Button>
      <Button size="small" onClick={() => setExecutionOpen(true)}>执行详情与产物</Button>
      <Typography.Text strong>运行 {current?.name || detail?.definition?.spec?.name || String(run.id).slice(0, 8)}</Typography.Text>
      <RunStatusTag status={current.status} /><NodeBadge nodeId={current.node_id} status={current.status} />
      <Typography.Text type="secondary">创建于 {absTime(current.created_at)}</Typography.Text>
    </Space>
    {current.error && <Alert type="error" showIcon title={current.error} />}
    {error && <Alert type="error" showIcon title={error} action={<Button onClick={() => setRevision((v) => v + 1)}>重试</Button>} />}
    {detail ? <DagRunResult key={run.id} id={run.id} spec={detail.definition.spec || detail.definition}
      status={current.status} onStatus={updateStatus} /> : !error && <Spin />}
    {executionOpen && <ExecutionDetail id={run.id} summary={{ ...current, kind: 'dag' }} onClose={() => setExecutionOpen(false)} onNotice={onNotice} />}
  </Space>;
}
