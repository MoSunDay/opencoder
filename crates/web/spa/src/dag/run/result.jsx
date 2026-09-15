import { Alert, Button, Spin } from 'antd';
import { useState } from 'react';
import { DagProcess } from '../process.jsx';
import { useDagProgress } from './useDagProgress.js';
import { LogsDrawer } from './logsDrawer.jsx';
import './run.css';

export function DagRunResult({ id, spec, status, onStatus }) {
  const [selected, setSelected] = useState(null);
  const [open, setOpen] = useState(false);
  const progress = useDagProgress({ id, status, onStatus });
  return <div className="dag-run-result">
    {progress.error && <Alert type="error" showIcon title={progress.error}
      action={<Button size="small" onClick={progress.retry}>重试</Button>} />}
    {progress.snapshot ? <DagProcess spec={spec} snapshot={progress.snapshot} selectedId={selected}
      onSelect={(name) => { setSelected(name); setOpen(true); }} /> : !progress.error && <Spin tip="加载执行结果…"><div style={{ height: 400 }} /></Spin>}
    {open && <LogsDrawer id={id} status={progress.snapshot?.execution_status || status}
      steps={(spec?.steps || []).map((step) => step.name)} step={selected || ''}
      onStepChange={(name) => setSelected(name || null)} onClose={() => setOpen(false)} />}
  </div>;
}
