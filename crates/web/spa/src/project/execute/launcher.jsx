import { Alert, Select, Spin } from 'antd';
import { useEffect, useState } from 'react';
import { apiGet } from '../../api.js';
import { ChatPanel } from '../../chat.jsx';
import { DefsTab } from '../../dag/defsTab.jsx';
import { FleetTeamsPanel } from '../../fleet/teams.jsx';
import { TodoPanel } from '../../todoPanel.jsx';
import { Launch } from '../../brain/workbench/launch.jsx';

export const CAPABILITIES = [
  { value: 'operator', label: 'Operator' },
  { value: 'agent', label: 'Agent' },
  { value: 'team', label: 'Team' },
  { value: 'dag', label: 'DAG 工作流' },
  { value: 'todos', label: 'TODO 工作流' },
  { value: 'brain', label: '大脑调度' },
];

function BrainLaunch({ onCreated, prompt }) {
  const [capabilities, setCapabilities] = useState([]);
  const [plans, setPlans] = useState([]);
  const [plan, setPlan] = useState(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  useEffect(() => {
    Promise.all([apiGet('/api/brain/library'), apiGet('/api/brain/plan-defs')])
      .then(([library, definitions]) => { setCapabilities(library.capabilities || []); setPlans(definitions.plans || []); })
      .catch((failure) => setError(failure.message))
      .finally(() => setLoading(false));
  }, []);
  if (error) return <Alert type="error" title={error} />;
  if (loading) return <Spin />;
  return <div>
    <Select aria-label="大脑计划" placeholder="选择计划" value={plan} onChange={setPlan} style={{ width: 350, marginBottom: 16 }}
      options={plans.filter((item) => item.schema_version === 7).map((item) => ({ value: `${item.id}@${item.latest_version}`, label: item.title }))} />
    {plan && <Launch key={plan} onCreated={onCreated} capabilities={capabilities} initialPlan={plan} initialPrompt={prompt} />}
  </div>;
}

export function CapabilityLauncher({ kind, onCreated, onNotice, prompt }) {
  return <div>
    {kind === 'brain' && <BrainLaunch onCreated={onCreated} prompt={prompt} />}
    {kind === 'dag' && <DefsTab onNotice={onNotice} onDispatched={onCreated} initialPrompt={prompt} />}
    {kind === 'todos' && <TodoPanel onNotice={onNotice} onCreated={onCreated} initialPrompt={prompt} />}
    {kind === 'team' && <FleetTeamsPanel onNotice={onNotice} onCreated={onCreated} initialPrompt={prompt} />}
    {(kind === 'agent' || kind === 'operator') && <ChatPanel onNotice={onNotice} onCreated={onCreated} initialPrompt={prompt} launchKind={kind} />}
  </div>;
}
