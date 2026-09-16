import {Button, Card, Space, Tabs, Tag, Timeline, Typography} from 'antd';
import {useCallback, useEffect, useState} from 'react';
import {apiGet} from './api.js';
import {REF_FIELDS, resolvedNames} from './agentsItems.js';
import {useEvent} from './ui/editing/useEvent.js';
import {AgentHarnessFields} from './harness/agentFields.jsx';
import {CATEGORIES} from './agents/resourceModel.js';
import {useResources} from './agents/useResources.js';
import {ResourceTab} from './agents/resourceTab.jsx';
const {Text} = Typography;

/// Meta tab：引用变更历史（field / from → to / at）+ references 汇总。
function MetaTab({ meta }) {
  const hist = (meta && meta.history) || [];
  const refs = (meta && meta.references) || {};
  return (
    <div>
      <Card size="small" title="引用变更历史">
        {hist.length === 0 ? <Text type="secondary">暂无变更</Text> : (
          <Timeline
            items={hist.map((h, i) => ({
              key: String(i),
              content: (
                <Space wrap size={4}>
                  <Tag>{h.field || '-'}</Tag>
                  <Text style={{ fontSize: 12 }}>{h.from || '—'} → {h.to || '—'}</Text>
                  <Text type="secondary" style={{ fontSize: 12 }}>{h.at || '-'}</Text>
                </Space>
              ),
            }))}
          />
        )}
      </Card>
      <Card size="small" title="解析快照（references）" style={{ marginTop: 12 }}>
        {REF_FIELDS.map(({ field, cat }) => (
          <div key={field} style={{ marginBottom: 4 }}>
            <Text type="secondary" style={{ fontSize: 12, width: 64, display: 'inline-block' }}>{field}</Text>
            {resolvedNames(refs, field).map((n) => <Tag key={n}>{n}</Tag>)}
            {resolvedNames(refs, field).length === 0 ? <Text type="secondary">—</Text> : null}
          </div>
        ))}
      </Card>
    </div>
  );
}

function AgentDetailSession({name,onNotice:noticeCallback,onChanged,onDirtyChange}) {
  const onNotice = useEvent(noticeCallback); const changed = useEvent(onChanged);
  const [meta,setMeta] = useState(null); const [error,setError] = useState('');
  const loadMeta = useCallback(async () => {
    try {
      const result = await apiGet(`/api/agents/${encodeURIComponent(name)}/meta`);
      if (!result?.meta) throw new Error('卡片响应不完整');
      setMeta({...result.meta,builtin:!!result.builtin}); setError('');
    } catch (error) { setError(error.message); }
  },[name]);
  useEffect(() => { loadMeta(); },[loadMeta]);
  const onSaved = useCallback(() => { loadMeta(); changed?.(); },[loadMeta,changed]);
  const resources = useResources(name,onSaved,onDirtyChange);
  return <div>
    <Space style={{marginBottom:16}}>
      <Button size="small" disabled={resources.busy} onClick={() => {resources.refresh(); loadMeta();}}>刷新</Button>
    </Space>
    {error && <Text type="danger">卡片读取失败：{error}</Text>}
    {meta && <AgentHarnessFields meta={meta} onNotice={onNotice} onSaved={onSaved}/>}
    <Tabs defaultActiveKey="prompts" items={[
      ...CATEGORIES.map(({cat,label}) => ({key:cat,label,children:<ResourceTab cat={cat} label={label}
        entry={resources.entries[cat]} onEdit={files => resources.edit(cat,files)} onSave={version => resources.save(cat,version)}/>})),
      {key:'meta',label:'Meta',children:<MetaTab meta={meta}/>},
    ]}/>
  </div>;
}
export function AgentDetail(props) { return <AgentDetailSession key={props.name} {...props}/>; }
