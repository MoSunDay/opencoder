import {Alert,Button,Descriptions,Empty,Space,Spin,Tabs,Tag,Typography} from 'antd';
import {useEffect,useRef,useState} from 'react';
import {StatusTag} from '../../ui/statusTag.jsx';
import {readReview} from './api.js';
import {HistoryPanel} from './history.jsx';

export function NodeInspector({id,todoId,generation,onSession}) {
  const [detail,setDetail]=useState(null);const [attempt,setAttempt]=useState(null);
  const [error,setError]=useState('');const [tab,setTab]=useState('result');
  const pinned=useRef(false);const latest=useRef(null);
  const receiveContext=(value,explicit=false)=>{
    if(explicit)pinned.current=true;else latest.current=value;
    if(explicit||!pinned.current)setAttempt(value);
  };
  useEffect(()=>{
    if(!todoId)return;
    let alive=true;
    readReview(id,{section:'node',todo_id:todoId}).then(value=>{if(alive){setDetail(value);setError('');}}).catch(e=>{if(alive)setError(e.message);});
    return()=>{alive=false;};
  },[id,todoId,generation]);
  if(!todoId)return <Empty description="选择 TODO 查看上下文、结果和历史尝试" image={Empty.PRESENTED_IMAGE_SIMPLE} />;
  if(!detail)return error?<Alert type="error" title={error}/>:<Spin />;
  const {todo,state}=detail;const candidate=state?.candidate;
  return <div className="todo-review-inspector">
    <Typography.Title level={5}>{todo.title}</Typography.Title>
    {error&&<Alert type="error" title="节点详情刷新失败" description={error} />}
    <Space wrap><StatusTag status={state.status}/><Tag>{todo.agent}</Tag><Tag>尝试 {state.attempt}</Tag>
      {state.active_session_id&&<Button size="small" onClick={()=>onSession(state.active_session_id)}>当前会话</Button>}</Space>
    <Tabs activeKey={tab} onChange={setTab} items={[
      {key:'result',label:'结果与验收',children:<>
        <Descriptions column={1} size="small" items={[
          {key:'background',label:'背景',children:todo.requirement_background},
          {key:'instruction',label:'执行说明',children:todo.instructions},
          {key:'criteria',label:'验收标准',children:todo.acceptance.criteria},
          {key:'generation',label:'通过版本',children:state.accepted_generation??'尚未通过'},
        ]}/>
        {!!todo.acceptance.required_tool_calls?.length&&<><strong>必需工具调用</strong><pre className="todo-review-json">{JSON.stringify(todo.acceptance.required_tool_calls,null,2)}</pre></>}
        {candidate?<><Typography.Paragraph strong>{candidate.summary}</Typography.Paragraph>
          <pre className="todo-review-json">{candidate.result||'未返回结果正文'}</pre>
          <Typography.Paragraph>验证：{candidate.verification}</Typography.Paragraph>
          <strong>证据引用</strong><ul>{candidate.evidence_refs.map((ref,i)=><li key={i}><Typography.Text copyable>{ref}</Typography.Text></li>)}</ul>
          {candidate.recovery_context?.summary&&<Typography.Paragraph>恢复说明：{candidate.recovery_context.summary}</Typography.Paragraph>}
        </>:<Empty description="当前轮次尚无候选结果" image={Empty.PRESENTED_IMAGE_SIMPLE}/>}
        {state.last_error&&<Alert type="error" title={state.last_error}/>}<Button type="link" onClick={()=>setTab('history')}>查看父 Agent 验收理由与工具门禁</Button>
      </>},
      {key:'context',label:'派发上下文',children:attempt?<>
        <p>派发 #{attempt.dispatch_seq} · 轮次 {attempt.world_epoch} · 尝试 {attempt.attempt} · {attempt.context_mode}</p>
        <Button size="small" onClick={()=>onSession(attempt.session_id)}>该次执行会话</Button>
        {pinned.current&&<Button size="small" onClick={()=>{pinned.current=false;setAttempt(latest.current);}}>查看最新派发</Button>}
        <pre className="todo-review-json">{JSON.stringify(attempt.context,null,2)}</pre>
      </>:<Alert type="info" title="尚无已记录的派发上下文" description="旧运行没有保存的细节不会补写；可在历史尝试中查看更早记录。"/>},
      {key:'history',label:'历史尝试',forceRender:true,children:<HistoryPanel key={`${id}:${todoId}`} id={id} todoId={todoId} generation={generation} onSession={onSession} onContext={receiveContext} onSelectContext={()=>setTab('context')}/>},
    ]}/>
  </div>;
}
