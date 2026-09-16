import {Alert,Button,Collapse,Empty,Select,Space,Spin,Typography} from 'antd';
import {useEffect,useRef,useState} from 'react';
import {Markdown} from '../../../project/markdown.jsx';
import {StatusTag} from '../../../ui/statusTag.jsx';
import {readReview} from '../api.js';
import {taskSessions} from './model.js';
import {KeptPane} from './pane.jsx';
import {SessionConversation} from './session.jsx';

function useTask(id,node,snapshot,active) {
  const [detail,setDetail]=useState(null);const [error,setError]=useState('');const [retry,setRetry]=useState(0);
  const loaded=useRef('');
  const version=`${snapshot.workflow.generation}/${snapshot.head_seq}/${retry}`;
  useEffect(()=>{
    if(!active||loaded.current===version)return undefined;
    let alive=true;const controller=new AbortController();
    setError('');
    readReview(id,{section:'node',todo_id:node.id},{signal:AbortSignal.any([controller.signal,AbortSignal.timeout(10000)])})
      .then(value=>{if(!value?.todo||!value?.state)throw new Error('任务详情响应格式异常');if(alive){loaded.current=version;setDetail(value);}})
      .catch(cause=>{if(alive)setError(cause.message);});
    return()=>{alive=false;controller.abort();};
  },[id,node.id,version,active]);
  return {detail,error,retry:()=>setRetry(value=>value+1)};
}

export function TaskConversation({id,node,snapshot,active,onSelect}) {
  const {detail,error,retry}=useTask(id,node,snapshot,active);
  const {sessions,current}=taskSessions(node,detail);
  const [chosen,setChosen]=useState('');const sessionId=chosen||current;
  const [visited,setVisited]=useState([]);
  useEffect(()=>{if(sessionId)setVisited(previous=>previous.includes(sessionId)?previous:[...previous,sessionId]);},[sessionId]);
  const mounted=[...new Set([...visited,sessionId].filter(Boolean))];
  const candidate=detail?.state?.candidate;
  return <div className="todo-task-conversation">
    <div className="todo-conversation-heading"><div><Typography.Text type="secondary">{node.id}</Typography.Text><Typography.Title level={5}>{node.title}</Typography.Title></div><StatusTag status={node.status}/></div>
    {error&&<Alert type="error" title="读取任务详情失败" description={error} action={<Button onClick={retry}>重试</Button>}/>}
    {node.last_error&&<Alert type="error" title={node.last_error}/>}
    <div className="todo-task-context">
      {!!node.depends_on?.length&&<Space wrap><Typography.Text type="secondary">前置 TODO</Typography.Text>{node.depends_on.map(todo=><Button key={todo} size="small" type="link" onClick={()=>onSelect(todo)}>{todo}</Button>)}</Space>}
      {detail&&<Collapse size="small" items={[
        ...(candidate&&sessionId===current&&sessionId===taskSessions({},detail).current?[{key:'result',label:'执行结果',children:<>
          <Markdown text={candidate.summary}/>{candidate.result&&<Markdown text={candidate.result}/>}
          {candidate.verification&&<><Typography.Text strong>验证</Typography.Text><Markdown text={candidate.verification}/></>}
          {!!candidate.evidence_refs?.length&&<pre className="todo-review-json">{candidate.evidence_refs.join('\n')}</pre>}
        </>}]:[]),
        {key:'definition',label:'任务要求',children:<pre className="todo-review-json">{JSON.stringify(detail.todo,null,2)}</pre>},
      ]}/>}
      {sessions.length>1&&<Select aria-label="执行会话" value={chosen||'__current__'} onChange={value=>setChosen(value==='__current__'?'':value)}
        options={[{value:'__current__',label:'当前会话'},...sessions.map((session,index)=>({value:session,label:`会话 ${index+1}${session===current?' · 当前':''}`}))]} />}
    </div>
    {mounted.map(session=><KeptPane key={session} active={active&&session===sessionId} label={`TODO ${node.id} 的对话`}>
      <SessionConversation id={id} sessionId={session} active={active&&session===sessionId}/>
    </KeptPane>)}
    {!sessionId&&(detail?<Empty description="该 TODO 尚未开始执行"/>:!error?<Spin/>:null)}
  </div>;
}
