import { Alert, Button, Spin, Typography } from 'antd';
import { useEffect, useState } from 'react';
import { apiGet } from '../../../../api.js';
import { ExecutionInputs } from '../../../../ui/runContent/execution.jsx';
import { DataValue } from '../../../../ui/runContent/value.jsx';

export function Inputs({ id, view, enabled }) {
  const [state, setState] = useState({ detail: null, error: '', loading: true });
  const [retry, setRetry] = useState(0);
  useEffect(() => {
    if (!enabled) return undefined;
    const controller = new AbortController();
    setState((old) => ({ ...old, loading: true, error: '' }));
    apiGet(`/api/executions/${encodeURIComponent(id)}`, { signal: controller.signal }).then((detail) => {
      if (!controller.signal.aborted) setState({ detail, error: '', loading: false });
    }).catch((error) => { if (!controller.signal.aborted) setState((old) => ({ ...old, loading: false, error: error.message })); });
    return () => controller.abort();
  }, [id, enabled, retry, view.run.last_event_seq]);
  const visits = (view.events || []).filter((event) => event.event_type === 'layer_started');
  return <div className="run-content-stack">
    {state.error && <Alert type="error" showIcon title={state.error} action={<Button aria-label="重试" onClick={() => setRetry(retry + 1)}>重试</Button>} />}
    {state.loading && !state.detail ? <Spin /> : state.detail && <ExecutionInputs detail={state.detail} id={id} />}
    <section><Typography.Title level={5}>各次派发的输入绑定</Typography.Title>
      {visits.map((visit) => <details className="brain-binding" key={visit.activation}><summary>第 {visit.round} 轮 · 第 {visit.layer} 层 · 第 {visit.activation} 次派发</summary>
        <DataValue value={visit.assignments || []} id={id} /></details>)}</section>
  </div>;
}
