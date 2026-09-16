import { Alert, Button, Spin } from 'antd';
import { useState } from 'react';
import { DagProcess } from '../process.jsx';
import { StepDrawer } from '../step/stepDrawer.jsx';
import { useDagProgress } from './useDagProgress.js';
import { LogsDrawer } from './logsDrawer.jsx';
import './run.css';

export function DagRunResult({ id, spec, status, onStatus }) {
  const [selected, setSelected] = useState(null);
  const [open, setOpen] = useState(false);
  // Run-wide logs stay reachable from the step drawer (onOpenRunLogs).
  const [logsOpen, setLogsOpen] = useState(false);
  const progress = useDagProgress({ id, status, onStatus });
  // Spec-side step kind: the drawer's receipt wins once loaded, this is the
  // first-paint fallback so the panel picks the right view immediately.
  const specKind = (spec?.steps || []).find((step) => step.name === selected)?.kind?.type || '';
  return <div className="dag-run-result">
    {progress.error && <Alert type="error" showIcon title={progress.error}
      action={<Button size="small" onClick={progress.retry}>重试</Button>} />}
    {progress.snapshot ? <DagProcess spec={spec} snapshot={progress.snapshot} selectedId={selected}
      onSelect={(name) => { setSelected(name); setOpen(true); }} /> : !progress.error && <Spin tip="加载执行结果…"><div style={{ height: 400 }} /></Spin>}
    {open && selected && <StepDrawer runId={id} step={selected} specKind={specKind}
      onClose={() => setOpen(false)} onOpenRunLogs={() => setLogsOpen(true)} />}
    {logsOpen && <LogsDrawer id={id} status={progress.snapshot?.execution_status || status}
      steps={(spec?.steps || []).map((step) => step.name)} step={selected || ''}
      onStepChange={(name) => setSelected(name || null)} onClose={() => setLogsOpen(false)} />}
  </div>;
}
