import { Drawer } from 'antd';
import { ExecutionLogs } from '../../ui/executionEvents/executionLogs.jsx';
import { useExecutionEvents } from '../../ui/executionEvents/useExecutionEvents.js';

export function LogsDrawer({ id, status, steps, step, onStepChange, onClose }) {
  const logs = useExecutionEvents({ id, status, hydrate: true });
  return <Drawer open title="实时日志" placement="right" size="75vw" rootClassName="dag-logs-drawer"
    styles={{ wrapper: { maxWidth: '100vw' } }} onClose={onClose}>
    <ExecutionLogs id={id} {...logs} steps={steps} step={step} onStepChange={onStepChange} />
  </Drawer>;
}
