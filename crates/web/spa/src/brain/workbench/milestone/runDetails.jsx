import { ReadingTabs } from '../../../ui/runContent/tabs.jsx';
import { useState } from 'react';
import { Overview } from './details/overview.jsx';
import { Rounds } from './details/rounds.jsx';
import { Events } from './details/events.jsx';
import { Inputs } from './details/inputs.jsx';
import './details/style.css';
export { roundRows } from './details/model.js';

export function RunDetails({ id, view, plan, history, onExecution, visible = true }) {
  const [tab, setTab] = useState('overview');
  return <div className="brain-run-details"><ReadingTabs activeKey={tab} onChange={setTab} items={[
    { key: 'overview', label: '概览', children: <Overview view={view} plan={plan} /> },
    { key: 'executions', label: '执行记录', children: <Rounds view={view} plan={plan} history={history} onExecution={onExecution} /> },
    { key: 'events', label: '调度记录', children: <Events view={view} plan={plan} onExecution={onExecution} /> },
    { key: 'inputs', label: '输入与产物', children: <Inputs id={id} view={view} enabled={visible && tab === 'inputs'} /> },
  ]} /></div>;
}
