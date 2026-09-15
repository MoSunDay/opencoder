import {Alert,Button,Empty,Space,Spin,Tag,Typography} from 'antd';
import {useState} from 'react';
import {apiPost} from '../../api.js';
import {StatusTag} from '../../ui/statusTag.jsx';
import {useReview} from './useReview.js';
import {EVENT_LABELS} from './model.js';
import {ReviewFiles} from './files/workspace.jsx';
import {SessionReview} from './session.jsx';
import {RerunDialog} from './rerun.jsx';
import './workbench.css';

export function TodoWorkbench({id,onMutated,showControls=true}) {
  const {snapshot,error,lastSuccess,loading,refresh,stale}=useReview(id);
  const [selected,setSelected]=useState('');const [fileError,setFileError]=useState('');
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
  return <section className="todo-workbench" aria-label="TODO 运行工作台">
    <div className="todo-workbench-toolbar"><Space wrap>
      <Button onClick={refresh} loading={loading}>刷新状态</Button>
      {showControls&&<><Button disabled={disabled||!['pending','running','idle'].includes(current)} onClick={()=>control('interrupt')}>中断（可恢复）</Button>
        <Button disabled={disabled||!['interrupted','error'].includes(current)||['completed','failed'].includes(wf?.status)} onClick={()=>control('resume')}>在原节点恢复</Button>
        <Button danger disabled={disabled||['done','error','cancelled'].includes(current)||!wf} onClick={()=>control('cancel')}>取消（终止）</Button></>}
      <Button type="primary" disabled={disabled||!selected||current==='pending'||current==='cancelling'} onClick={()=>setRerun(selected)}>从选中任务重跑</Button>
    </Space><Typography.Text type={stale?'danger':'secondary'}>{lastSuccess?`${stale?'数据已陈旧 · ':''}更新于 ${new Date(lastSuccess).toLocaleTimeString()}`:'等待状态同步'}</Typography.Text></div>
    {(error||actionError)&&<Alert type="error" showIcon title={actionError?'执行操作失败':'状态同步失败'} description={actionError||error}/>}
    {loading&&!snapshot?<Spin/>:snapshot?.initializing?<Alert type="info" title="工作流正在初始化" description="受理记录已保存，等待父 Agent 和 TODO 状态就绪。"/>:!wf?<Empty description="无法读取工作流"/>:<>
      <div className="todo-parent-heading"><div><Typography.Title level={4}>{wf.name}</Typography.Title><Typography.Paragraph>{wf.objective}</Typography.Paragraph></div>
        <Space wrap><StatusTag status={wf.status}/><Tag>轮次 {wf.world_epoch}</Tag><Tag>{progress.passed}/{progress.total} 已通过</Tag><Button onClick={()=>{setSelected('');setSessionId(wf.parent_session_id);}}>父 Agent 会话</Button></Space>
        <div className="todo-parent-decision"><strong>父 Agent · workflow</strong><span>{EVENT_LABELS[snapshot.latest_event?.kind]||snapshot.latest_event?.kind||'等待调度'}</span><span>{snapshot.latest_event?.payload?.reason||wf.terminal_reason||''}</span></div>
      </div>
      {!!snapshot.controls?.length&&<Space wrap>{snapshot.controls.slice(-3).map(c=><Tag key={c.request_id} color={c.phase==='failed'?'error':c.phase==='stopping'?'processing':'success'} title={c.error||c.request_id}>{c.todo_id} 重跑 · {c.phase==='stopping'?'等待停止':c.phase==='queued'?'已重新排队':`失败：${c.error}`}</Tag>)}</Space>}
      <ReviewFiles key={id} id={id} snapshot={snapshot} onTodo={setSelected} onSession={setSessionId} onError={setFileError}/>
    </>}
    <SessionReview id={id} sessionId={sessionId} onClose={()=>setSessionId('')}/>
    <RerunDialog id={id} todoId={rerun} snapshot={snapshot} onClose={()=>setRerun('')} onAccepted={notify}/>
  </section>;
}
