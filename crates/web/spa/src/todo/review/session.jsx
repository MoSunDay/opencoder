import {Alert,Button,Collapse,Drawer,Empty,Space,Spin,Tabs} from 'antd';
import {useEffect,useMemo,useRef,useState} from 'react';
import {appendMessagePage} from '../../fleet/model.js';
import {TranscriptView} from '../../transcript.jsx';
import {turnsFromMessages} from '../../reduce.js';
import {eventPayload,readReview} from './api.js';

export function SessionReview({id,sessionId,onClose}) {
  return <Drawer title={`会话 Review · ${sessionId}`} placement="right" size="85vw" open={!!sessionId} onClose={onClose} destroyOnHidden>
    {sessionId&&<SessionBody key={sessionId} id={id} sessionId={sessionId} />}
  </Drawer>;
}

function SessionBody({id,sessionId}) {
  const [messages,setMessages]=useState(null);const [events,setEvents]=useState([]);
  const [cursor,setCursor]=useState(0);const [more,setMore]=useState(true);
  const alive=useRef(true);const latest=useRef({});const reading=useRef(false);const tail=useRef(0);
  const [tab,setTab]=useState('messages');
  const [busy,setBusy]=useState(false);const [error,setError]=useState('');
  const loadMessages=async(reset=false,silent=false)=>{
    if(reading.current)return;reading.current=true;
    if(!silent)setBusy(true);setError('');
    try{
      const next=reset?null:messages?.nextCursor||{seq:tail.current,offset:0};
      const page=await readReview(id,{section:'messages',session_id:sessionId,after_seq:next?.seq||0,message_offset:next?.offset||0});
      const leading=!reset&&messages?.large?.at(-1)?.tail||new Uint8Array();
      if(!alive.current)return;
      if(reset)tail.current=0;
      const last=page.chunks?.at(-1);if(last?.eof)tail.current=last.seq;
      if(reset||page.chunks?.length)setMessages(appendMessagePage(reset?null:messages,page,leading));
    }catch(e){if(alive.current)setError(e.message);}finally{reading.current=false;if(alive.current)setBusy(false);}
  };
  const loadEvents=async(reset=false,silent=false)=>{
    if(reading.current)return;reading.current=true;
    if(!silent)setBusy(true);setError('');
    try{const page=await readReview(id,{section:'session_events',session_id:sessionId,after_seq:reset?0:cursor});
      for(const event of page.events)event.data=await eventPayload(id,event,{},sessionId);
      if(!alive.current)return;
      setEvents(previous=>[...new Map([...(reset?[]:previous),...page.events].map(e=>[e.seq,e])).values()]);setCursor(page.events.at(-1)?.seq||cursor);setMore(!!page.more);
    }catch(e){if(alive.current)setError(e.message);}finally{reading.current=false;if(alive.current)setBusy(false);}
  };
  latest.current={busy,loadMessages,loadEvents,messages,tab,more};
  useEffect(()=>{alive.current=true;loadMessages(true);
    const timer=setInterval(()=>{const value=latest.current;if(value.busy)return;if(value.tab==='messages'&&value.messages&&!value.messages.more)value.loadMessages(false,true);else if(value.tab==='events'&&!value.more)value.loadEvents(false,true);},3000);
    return()=>{alive.current=false;clearInterval(timer);};
  },[id,sessionId]);
  const turns=useMemo(()=>turnsFromMessages(messages?.messages||[]),[messages]);
  return <>
    {error&&<Alert type="error" title="读取会话失败" description={error} />}
    <Tabs activeKey={tab} onChange={key=>{setTab(key);if(key==='events'&&!events.length)loadEvents(true);}} items={[
      {key:'messages',label:'会话消息',children:<>
        <Space><Button onClick={()=>loadMessages(true)} loading={busy}>从头刷新</Button><Button disabled={!messages?.more||busy} onClick={()=>loadMessages()}>继续读取</Button></Space>
        {busy&&!messages?<Spin />:null}<TranscriptView turns={turns} />
        {messages?.trimmed&&<Alert type="info" title="较早消息已释放，可从头刷新查看" />}
        {messages?.large?.map(chunk=><pre key={`${chunk.seq}-${chunk.start}`} className="todo-review-json">{chunk.text}</pre>)}
        {messages?.partial&&<Alert type="info" title="当前消息尚未读取完整，请继续读取" />}
        {!busy&&!messages?.messages?.length&&!messages?.partial&&!messages?.large?.length&&<Empty description="暂无消息" />}
      </>},
      {key:'events',label:'工具与执行事件',children:<><Space><Button loading={busy} onClick={()=>loadEvents(true)}>刷新事件</Button><Button disabled={!more||busy} onClick={()=>loadEvents()}>继续读取事件</Button></Space>
        <Collapse items={events.map(event=>({key:event.seq,label:`#${event.seq} ${event.kind}`,children:<pre className="todo-review-json">{JSON.stringify(event.data,null,2)}</pre>}))} />
      </>},
    ]} />
  </>;
}
