// Step record drawer: opened from the run canvas when a step is selected.
// Header = the node-side step receipt (GET /api/dag/runs/:rid/steps/:step),
// body = the step's live node-side record (StepPanel). The run-wide logs
// stay reachable through `运行日志` (onOpenRunLogs → dag/run LogsDrawer).
import { Alert, Button, Descriptions, Drawer, Space, Spin, Typography } from 'antd';
import { useCallback, useEffect, useState } from 'react';
import { apiGet } from '../../api.js';
import { absTime } from '../../format.js';
import { StatusTag } from '../../ui/statusTag.jsx';
import { STEP_KIND_LABEL } from './model.js';
import { StepPanel } from './stepPanel.jsx';

// absTime(null) would read as 1970 — missing timestamps render as '—'.
const stamp = (value) => (value === null || value === undefined ? '—' : absTime(value));

export function StepDrawer({ runId, step, specKind, onClose, onOpenRunLogs }) {
  const [receipt, setReceipt] = useState(null);
  const [receiptError, setReceiptError] = useState('');
  const [finished, setFinished] = useState(null);
  const [nonce, setNonce] = useState(0);
  const refresh = useCallback(() => setNonce((value) => value + 1), []);
  useEffect(() => { setFinished(null); }, [runId, step]);
  useEffect(() => {
    const controller = new AbortController();
    setReceipt(null);
    setReceiptError('');
    apiGet('/api/dag/runs/' + encodeURIComponent(runId) + '/steps/' + encodeURIComponent(step), { signal: controller.signal })
      .then((value) => { if (!controller.signal.aborted) setReceipt(value); })
      .catch((e) => { if (!controller.signal.aborted) setReceiptError(e.message || '加载步骤回执失败'); });
    return () => controller.abort();
  }, [runId, step, nonce]);
  // Terminal receipt from the stream: refetch the step receipt so status,
  // timestamps and output settle without a manual refresh.
  const handleFinished = useCallback((value) => { setFinished(value); refresh(); }, [refresh]);
  const kind = receipt?.kind || specKind;
  const status = receipt?.status || finished?.status;
  return <Drawer open title={'步骤 ' + (receipt?.name || step)} placement="right" size="75vw"
    rootClassName="dag-logs-drawer" styles={{ wrapper: { maxWidth: '100vw' } }} onClose={onClose}
    extra={<Space>
      <Button size="small" onClick={refresh}>刷新</Button>
      <Button size="small" onClick={onOpenRunLogs}>运行日志</Button>
      <Button size="small" onClick={onClose}>关闭</Button>
    </Space>}>
    <div className="execution-logs">
      {receiptError && <Alert type="error" title={receiptError} action={<Button size="small" onClick={refresh}>重试</Button>} />}
      {!receipt && !receiptError && <Spin size="small" />}
      {receipt && <>
        <Descriptions size="small" column={2} items={[
          { key: 'kind', label: '类型', children: STEP_KIND_LABEL[kind] || kind || '—' },
          { key: 'status', label: '状态', children: status ? <StatusTag status={status} /> : '—' },
          { key: 'started', label: '开始', children: stamp(receipt.started_at_ms) },
          { key: 'finishedAt', label: '结束', children: stamp(receipt.finished_at_ms) },
          {
            key: 'session', label: '会话', span: 2,
            children: receipt.session_id ? <Typography.Text copyable>{receipt.session_id}</Typography.Text> : '—',
          },
        ]} />
        {receipt.error && <Alert type="error" title={receipt.error} />}
      </>}
      <StepPanel runId={runId} step={step} kind={kind} onFinished={handleFinished} />
    </div>
  </Drawer>;
}
