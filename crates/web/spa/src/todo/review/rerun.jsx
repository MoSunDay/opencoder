import {Alert,Button,Input,Modal,Space,Tag} from 'antd';
import {useEffect,useRef,useState} from 'react';
import {apiPost} from '../../api.js';
import {newId} from '../../fleet/model.js';
import {readReview} from './api.js';

export function RerunDialog({id,todoId,snapshot,onClose,onAccepted}) {
  const [preview,setPreview]=useState(null);const [reason,setReason]=useState('');
  const [busy,setBusy]=useState(false);const [error,setError]=useState('');const [receipt,setReceipt]=useState(null);
  const request=useRef(null);
  const epoch=useRef(0);
  const refresh=async()=>{
    const current=++epoch.current;
    setBusy(true);setError('');request.current=null;
    try{const node=await readReview(id,{section:'node',todo_id:todoId});if(current===epoch.current)setPreview(node.preview);}
    catch(e){if(current===epoch.current)setError(e.message);}finally{if(current===epoch.current)setBusy(false);}
  };
  useEffect(()=>{setPreview(null);setReason('');setReceipt(null);request.current=null;if(todoId)refresh();return()=>{epoch.current++;};},[id,todoId]);
  const current=(snapshot?.controls||[]).find(c=>c.request_id===receipt?.request_id)||receipt;
  useEffect(()=>{if(current?.phase==='queued'){onAccepted?.();onClose();}},[current?.phase]);
  const submit=async()=>{
    if(!reason.trim()||!preview)return;
    const body=request.current||{request_id:newId('rerun'),todo_id:todoId,reason:reason.trim(),expected_generation:preview.generation};
    request.current=body;setBusy(true);setError('');
    const current=epoch.current;
    try{const result=await apiPost(`/api/todo/workflows/${encodeURIComponent(id)}/rerun`,body);if(current===epoch.current){setReceipt(result);onAccepted?.();}}
    catch(e){if(current===epoch.current)setError(e.message);}finally{if(current===epoch.current)setBusy(false);}
  };
  return <Modal open={!!todoId} title={`从 ${todoId} 重新执行`} onCancel={onClose} footer={<Space>
    <Button onClick={onClose}>关闭</Button><Button disabled={busy||!!receipt} onClick={refresh}>刷新影响预览</Button>
    <Button type="primary" loading={busy} disabled={!preview||!!preview.blockers?.length||!reason.trim()||!!receipt} onClick={submit}>确认暂停并重跑</Button>
  </Space>}>
    <p>保留当前文件、外部操作结果和历史记录。旧执行停止后，父 Agent 将重新派发目标及下游任务。</p>
    {preview&&<><p>重新执行：{preview.affected.map(id=><Tag color="orange" key={id}>{id}</Tag>)}</p>
      <p>保留结果：{preview.preserved.map(id=><Tag key={id}>{id}</Tag>)}</p>
      {!!preview.blockers.length&&<Alert type="error" title={`前置任务尚未通过：${preview.blockers.join('、')}`} />}</>}
    <Input.TextArea aria-label="重跑原因" placeholder="说明为什么需要重新执行" value={reason} disabled={!!request.current} onChange={e=>setReason(e.target.value)} maxLength={4096} />
    {current&&<Alert type={current.phase==='failed'?'error':'info'} title={current.phase==='stopping'?'已受理，等待旧执行停止':current.phase} description={current.error} />}
    {error&&<Alert type="error" title="重跑操作失败" description={error} />}
  </Modal>;
}
