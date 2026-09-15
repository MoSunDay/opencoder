import {Alert,Button,Collapse,Empty,Space,Spin,Tag,Typography} from 'antd';
import {useEffect,useRef,useState} from 'react';
import {eventPayload,readReview} from './api.js';
import {EVENT_LABELS,relatedEvent} from './model.js';

export function HistoryPanel({id,todoId,onSession,onContext,onSelectContext,generation}) {
  const [events,setEvents]=useState([]);const [before,setBefore]=useState(null);
  const [busy,setBusy]=useState(false);const [error,setError]=useState('');
  const epoch=useRef(0);
  const load=async(cursor,reset=false)=>{
    const current=++epoch.current;
    setBusy(true);setError('');
    try{
      const page=await readReview(id,{section:'history',before_seq:cursor});
      const rows=[...page.events];const older=page.next_before_seq;
      for(const event of rows)event.payload=await eventPayload(id,event);
      if(current!==epoch.current)return;
      setEvents(previous=>[...new Map([...(reset?[]:previous),...rows].map(e=>[e.seq,e])).values()].sort((a,b)=>b.seq-a.seq));
      setBefore(older);
      if(reset&&onContext){const dispatch=[...rows].sort((a,b)=>b.seq-a.seq).find(e=>e.payload.assignments?.some(a=>a.todo_id===todoId));
        onContext(dispatch?{...dispatch.payload.assignments.find(a=>a.todo_id===todoId),dispatch_seq:dispatch.seq,world_epoch:dispatch.payload.world_epoch}:null);}
    }catch(e){if(current===epoch.current)setError(e.message);}finally{if(current===epoch.current)setBusy(false);}
  };
  useEffect(()=>{load(undefined,true);return()=>{epoch.current++;};},[id,todoId,generation]);
  const rows=events.filter(e=>relatedEvent(e,todoId));
  return <div className="todo-history">
    <Space><Button size="small" loading={busy} onClick={()=>load(undefined,true)}>刷新记录</Button>
      <Button size="small" disabled={!before||busy} onClick={()=>load(before)}>更早记录</Button></Space>
    {error&&<Alert type="error" title="读取历史失败" description={error} />}
    {busy&&!rows.length?<Spin />:!rows.length?<Empty description="尚无过程记录" image={Empty.PRESENTED_IMAGE_SIMPLE} />:null}
    <Collapse size="small" items={rows.map(event=>({key:event.seq,label:<span><Tag>#{event.seq}</Tag>{EVENT_LABELS[event.kind]||event.kind}</span>,children:<>
      {event.payload.reason&&<Typography.Paragraph>{event.payload.reason}</Typography.Paragraph>}
      {(event.payload.assignments||[]).filter(a=>!todoId||a.todo_id===todoId).map(a=><Space key={a.todo_id} wrap>
        <Tag>{a.todo_id} · 尝试 {a.attempt} · {a.context_mode}</Tag>
        <Button size="small" onClick={()=>onSession?.(a.session_id)}>查看该次会话</Button>
        {onContext&&<Button size="small" onClick={()=>{onContext({...a,dispatch_seq:event.seq,world_epoch:event.payload.world_epoch},true);onSelectContext?.();}}>查看该次上下文</Button>}
      </Space>)}
      <pre className="todo-review-json">{JSON.stringify(event.payload,null,2)}</pre>
    </>}))} />
  </div>;
}
