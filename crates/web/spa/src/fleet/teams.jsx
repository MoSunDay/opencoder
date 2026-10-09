import { Alert, Button, Tabs } from 'antd';
import { useEffect, useState } from 'react';
import { apiGet } from '../api.js';
import { PageShell } from '../shell/pageShell.jsx';
import { ExecutionDetail } from './detail.jsx';
import { TeamEditor } from './teams/editor.jsx';
import { TeamList } from './teams/list.jsx';
import { TeamLaunch } from './teams/launch.jsx';
import { TeamExecutionHistory } from './teams/history.jsx';
import { info, ok } from '../notice.js';

export function FleetTeamsPanel({ onNotice, onCreated, initialPrompt = '' }) {
  const [data, setData] = useState({ teams: [], nodes: [], agents: [] });
  const [tab, setTab] = useState('teams'), [revision, setRevision] = useState(0);
  const [editor, setEditor] = useState(null), [launch, setLaunch] = useState(null);
  const [detail, setDetail] = useState(null), [accepted, setAccepted] = useState(null);
  const [loading, setLoading] = useState(true), [error, setError] = useState('');
  useEffect(() => {
    const controller = new AbortController();
    setLoading(true); setError('');
    Promise.all(['/api/teams', '/api/nodes', '/api/brain/agents'].map((path) => apiGet(path, { signal: controller.signal })))
      .then(([a, b, c]) => { if (!controller.signal.aborted) setData({ teams: a.teams || [], nodes: b.nodes || [], agents: c.agents || [] }); })
      .catch((failure) => { if (!controller.signal.aborted) setError(failure.message); })
      .finally(() => { if (!controller.signal.aborted) setLoading(false); });
    return () => controller.abort();
  }, [revision]);
  const inspect = (run) => { setLaunch(null); setTab('history'); setDetail(run); };
  const created = (run) => {
    setAccepted(run); setLaunch(null); setTab('history');
    onNotice?.(info('Team 已启动，可在执行记录中继续查看讨论过程。'));
    if (onCreated) onCreated(run.id); else setDetail(run);
  };
  return <PageShell page="team">
    {error && <Alert type="error" showIcon title="Team 数据读取失败" description={error}
      action={<Button onClick={() => setRevision((v) => v + 1)}>重试</Button>} />}
    {accepted && <Alert type="success" showIcon closable onClose={() => setAccepted(null)} title="Team 已启动"
      description={`执行编号：${accepted.id}`} action={<Button onClick={() => inspect(accepted)}>查看执行过程</Button>} style={{ marginBottom: 12 }} />}
    <Tabs activeKey={tab} onChange={setTab} destroyOnHidden items={[
      { key: 'teams', label: 'Team 列表', children: <TeamList rows={data.teams} loading={loading}
        onRefresh={() => setRevision((v) => v + 1)} onEdit={(team) => setEditor({ team })} onLaunch={setLaunch} /> },
      { key: 'history', label: '执行记录', children: <TeamExecutionHistory key={accepted?.id || 'history'} nodes={data.nodes} onOpen={setDetail} /> },
    ]} />
    <TeamEditor open={!!editor} team={editor?.team} agents={data.agents} onClose={() => setEditor(null)}
      onSaved={() => { onNotice?.(ok('Team 已保存')); setEditor(null); setRevision((v) => v + 1); }} />
    <TeamLaunch team={launch} nodes={data.nodes} initialPrompt={initialPrompt} onClose={() => setLaunch(null)} onAccepted={created} onHistory={() => { setLaunch(null); setTab('history'); }} />
    {detail && <ExecutionDetail id={detail.id} summary={detail} onClose={() => setDetail(null)} onNotice={onNotice} />}
  </PageShell>;
}
