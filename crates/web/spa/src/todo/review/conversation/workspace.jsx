import {Empty,Input,Typography} from 'antd';
import {useState} from 'react';
import {StatusTag} from '../../../ui/statusTag.jsx';
import {KeptPane} from './pane.jsx';
import {SessionConversation} from './session.jsx';
import {TaskConversation} from './task.jsx';

export function ConversationWorkspace({id,snapshot,selected,onSelect,active=true}) {
  const [visited,setVisited]=useState([]);const [query,setQuery]=useState('');
  const select=todo=>{if(todo)setVisited(previous=>previous.includes(todo)?previous:[...previous,todo]);onSelect(todo);};
  const nodes=snapshot.nodes||[];
  const mounted=[...new Set([...visited,selected].filter(Boolean))];
  const visible=nodes.filter(node=>`${node.id} ${node.title}`.toLowerCase().includes(query.trim().toLowerCase()));
  return <div className="todo-conversation-workspace">
    <nav className="todo-conversation-nav" aria-label="父 Agent 与 TODO">
      <button type="button" aria-label="父 Agent" className="todo-conversation-nav-item" aria-current={!selected?'page':undefined} onClick={()=>select('')}><strong>父 Agent</strong><span>调度与验收</span></button>
      <div className="todo-task-list-heading"><Typography.Text strong>TODO 清单</Typography.Text><Typography.Text type="secondary">{nodes.length}</Typography.Text></div>
      <Input aria-label="搜索 TODO" placeholder="搜索 TODO" value={query} onChange={event=>setQuery(event.target.value)} allowClear/>
      <div className="todo-task-list">{visible.map(node=><button key={node.id} type="button" data-todo-id={node.id} className="todo-conversation-nav-item" aria-current={selected===node.id?'page':undefined} onClick={()=>select(node.id)}>
        <span className="todo-task-list-meta"><span>{node.id}</span><StatusTag status={node.status}/></span><strong>{node.title}</strong>
      </button>)}{!visible.length&&<Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={nodes.length?'没有匹配的 TODO':'暂无 TODO'}/>}</div>
    </nav>
    <div className="todo-conversation-main">
      <KeptPane active={active&&!selected} label="父 Agent 的对话">
        <div className="todo-conversation-heading"><Typography.Title level={5}>父 Agent</Typography.Title><StatusTag status={snapshot.workflow.status}/></div>
        {snapshot.workflow.parent_session_id?<SessionConversation key={snapshot.workflow.parent_session_id} id={id} sessionId={snapshot.workflow.parent_session_id} active={active&&!selected}/>:<Empty description="父 Agent 会话尚未就绪"/>}
      </KeptPane>
      {mounted.map(todo=>{const node=nodes.find(item=>item.id===todo);return node?<div key={todo} hidden={selected!==todo} className="todo-task-pane">
        <TaskConversation id={id} node={node} snapshot={snapshot} active={active&&selected===todo} onSelect={select}/>
      </div>:null;})}
    </div>
  </div>;
}
