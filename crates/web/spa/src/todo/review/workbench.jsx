import {Alert,Button,Dropdown,Empty,Space,Spin,Tabs,Tag,Typography} from 'antd';
import {useState} from 'react';
import {apiPost} from '../../api.js';
import {StatusTag} from '../../ui/statusTag.jsx';
import {useReview} from './useReview.js';
import {ConversationWorkspace} from './conversation/workspace.jsx';
import {ReviewFiles} from './files/workspace.jsx';
import {SessionReview} from './session.jsx';
import {RerunDialog} from './rerun.jsx';
import './workbench.css';

export function TodoWorkbench({id,onMutated,showControls=true}) {
  const {snapshot,error,lastSuccess,loading,refresh,stale}=useReview(id);
  const [selected,setSelected]=useState('');const [fileError,setFileError]=useState('');
  const [tab,setTab]=useState('conversation');const [fileTodo,setFileTodo]=useState('');
  const [sessionId,setSessionId]=useState('');const [rerun,setRerun]=useState('');
  const [actionError,setActionError]=useState('');const [busy,setBusy]=useState(false);
  const wf=snapshot?.workflow;
  const progress={total:snapshot?.nodes?.length||0,passed:snapshot?.nodes?.filter(n=>n.status==='passed').length||0};
  const notify=()=>{refresh();onMutated?.();};
  const control=async(action)=>{
    setBusy(true);setActionError('');
    try{await apiPost(action==='cancel'?`/api/executions/${encodeURIComponent(id)}/commands`:`/api/todo/workflows/${encodeURIComponent(id)}/${action}`,action==='cancel'?{action:'cancel',input:{}}:{});notify();}
    catch(e){setActionError(e.message);}finally{setBusy(false);}
  };
  const current=snapshot?.execution_status;
  const pendingControl=snapshot?.controls?.some(c=>c.phase==='stopping');
  const disabled=stale||!!error||!!fileError||busy||pendingControl;
  const selectedTodo=tab==='conversation'?selected:fileTodo;
  const actions=[
    ...(showControls?[
      {key:'interrupt',label:'中断（可恢复）',disabled:disabled||!['pending','running','idle'].includes(current)},
      {key:'resume',label:'在原节点恢复',disabled:disabled||!['interrupted','error'].includes(current)||['completed','failed'].includes(wf?.status)},
      {key:'cancel',label:'取消（终止）',danger:true,disabled:disabled||['done','error','cancelled'].includes(current)||!wf},
    ]:[]),
    {key:'rerun',label:'从选中任务重跑',disabled:disabled||!selectedTodo||current==='pending'||current==='cancelling'},
  ];
  const runAction=key=>key==='rerun'?setRerun(selectedTodo):control(key);
  return <section className="todo-workbench" aria-label="TODO 运行工作台">
    <div className="todo-workbench-toolbar"><Space wrap>
      <Button onClick={refresh} loading={loading}>刷新状态</Button>
      <Space wrap className="todo-workbench-controls">{actions.map(action=><Button key={action.key} type={action.key==='rerun'?'primary':'default'} danger={action.danger} disabled={action.disabled} onClick={()=>runAction(action.key)}>{action.label}</Button>)}</Space>
      <Dropdown menu={{items:actions,onClick:({key})=>runAction(key)}} trigger={['click']}><Button className="todo-workbench-mobile-actions">运行操作</Button></Dropdown>
    </Space><Typography.Text type={stale?'danger':'secondary'}>{lastSuccess?`${stale?'数据已陈旧 · ':''}更新于 ${new Date(lastSuccess).toLocaleTimeString()}`:'等待状态同步'}</Typography.Text></div>
    {(error||actionError)&&<Alert type="error" showIcon title={actionError?'执行操作失败':'状态同步失败'} description={actionError||error}/>}
    {loading&&!snapshot?<Spin/>:snapshot?.initializing?<Alert type="info" title="工作流正在初始化" description="受理记录已保存，等待父 Agent 和 TODO 状态就绪。"/>:!wf?<Empty description="无法读取工作流"/>:<>
      <div className="todo-parent-heading"><Typography.Title level={4}>{wf.name}</Typography.Title>
        <Space wrap><StatusTag status={wf.status}/><Tag>{progress.passed}/{progress.total} 已通过</Tag></Space>
      </div>
      {!!snapshot.controls?.length&&<Space wrap>{snapshot.controls.slice(-3).map(c=><Tag key={c.request_id} color={c.phase==='failed'?'error':c.phase==='stopping'?'processing':'success'} title={c.error||c.request_id}>{c.todo_id} 重跑 · {c.phase==='stopping'?'等待停止':c.phase==='queued'?'已重新排队':`失败：${c.error}`}</Tag>)}</Space>}
      <Tabs activeKey={tab} onChange={setTab} destroyOnHidden={false} items={[
        {key:'conversation',label:'执行对话',children:<ConversationWorkspace key={id} id={id} snapshot={snapshot} selected={selected} onSelect={setSelected} active={tab==='conversation'&&!sessionId}/>},
        {key:'files',label:'原始记录',children:<ReviewFiles key={id} id={id} snapshot={snapshot} active={tab==='files'} onTodo={setFileTodo} onSession={setSessionId} onError={setFileError}/>},
      ]}/>
    </>}
    <SessionReview id={id} sessionId={sessionId} onClose={()=>setSessionId('')}/>
    <RerunDialog id={id} todoId={rerun} snapshot={snapshot} onClose={()=>setRerun('')} onAccepted={notify}/>
  </section>;
}
