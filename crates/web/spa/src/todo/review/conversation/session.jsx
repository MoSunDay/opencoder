import {Alert,Button,Collapse,Empty,Space,Spin,Tabs} from 'antd';
import {useMemo,useState} from 'react';
import {TranscriptView} from '../../../transcript.jsx';
import {sessionPresentation} from './model.js';
import {useSession} from './useSession.js';

export function SessionConversation({id,sessionId,active=true}) {
  const [tab,setTab]=useState('say');
  const {messages,events,messageBusy,eventBusy,messageError,eventError,moreEvents,load}=useSession(id,sessionId,active,tab);
  const presentation=useMemo(()=>sessionPresentation(messages?.messages),[messages]);
  const say=<>
    {messageError&&<Alert type="error" title="读取会话失败" description={messageError} action={<Button onClick={()=>load('messages')}>重试</Button>}/>}
    {messages===null ? messageBusy?<Spin/>:null : <>
      <TranscriptView turns={presentation.turns} active={active&&tab==='say'} autoScroll={false} emptyText="暂无 Say，执行过程就绪后会自动显示"/>
      {!!messages.large?.length&&<Collapse items={messages.large.map(chunk=>({key:`${chunk.seq}-${chunk.start}`,label:`长消息片段 · ${chunk.role==='assistant'?'Agent':'输入'}`,children:<pre className="todo-review-json">{chunk.text}</pre>}))}/>}
      {messages.partial&&<Alert type="info" title="这条消息尚未读取完整"/>}
      {messages.trimmed&&<Alert type="info" title="当前显示最近的消息，可从头读取早期记录"/>}
      {(presentation.inputs.length>0||presentation.raw.length>0)&&<Collapse className="todo-conversation-context" items={[
        ...(presentation.inputs.length?[{key:'inputs',label:'输入与上下文',children:<TranscriptView turns={presentation.inputs} active={false} autoScroll={false}/>}]:[]),
        ...(presentation.raw.length?[{key:'raw',label:'原始回复',children:presentation.raw.map(item=><pre key={item.key} className="todo-review-json">{item.text}</pre>)}]:[]),
      ]}/>}
    </>}
    <Space wrap className="todo-conversation-pagination">
      {messages?.more&&<Button loading={messageBusy} onClick={()=>load('messages')}>继续读取消息</Button>}
      <Button size="small" loading={messageBusy} onClick={()=>load('messages',true)}>从头读取</Button>
    </Space>
  </>;
  return <section className="todo-conversation" aria-label="执行对话" data-session-id={sessionId}>
    <Tabs activeKey={tab} onChange={setTab} destroyOnHidden={false} items={[
      {key:'say',label:'Say',children:say},
      {key:'events',label:'执行事件',children:<>
        {eventError&&<Alert type="error" title="读取执行事件失败" description={eventError}/>}
        {events===null&&eventBusy?<Spin/>:events?.length?<Collapse items={events.map(event=>({key:event.seq,label:`#${event.seq} ${event.kind}`,children:<pre className="todo-review-json">{JSON.stringify(event.data,null,2)}</pre>}))}/>:<Empty description="暂无执行事件"/>}
        <Space wrap className="todo-conversation-pagination"><Button loading={eventBusy} onClick={()=>load('events',true)}>刷新事件</Button>
          {moreEvents&&<Button loading={eventBusy} onClick={()=>load('events')}>继续读取事件</Button>}</Space>
      </>},
    ]}/>
  </section>;
}
