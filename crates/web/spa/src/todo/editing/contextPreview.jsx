import { Alert, Button, Collapse, Typography } from 'antd';
import { useEffect, useRef, useState } from 'react';
import { apiPost } from '../../api.js';

export function ContextPreview({ spec, todoId }) {
  const [context,setContext]=useState(null);
  const [error,setError]=useState('');
  const [loading,setLoading]=useState(false);
  const epoch=useRef(0);
  useEffect(()=>{epoch.current++;setContext(null);setError('');setLoading(false);return()=>{epoch.current++;};},[spec,todoId]);
  const load=async()=>{
    const current=epoch.current;
    setLoading(true);setError('');
    try {const next=await apiPost('/api/todo/context-preview',{spec,todo_id:todoId});if(current===epoch.current)setContext(next);}
    catch(e){if(current===epoch.current)setError(e.message);} finally{if(current===epoch.current)setLoading(false);}
  };
  return <div style={{marginBottom:12}}>
    <Button block loading={loading} disabled={!spec} onClick={load}>预览派发上下文</Button>
    {error && <Alert type="error" title={error} />}
    {context && <Collapse defaultActiveKey={['context']} size="small" items={[{key:'context',label:'派发上下文',children:<>
      <Typography.Text type="secondary">依赖结果、证据和恢复信息将在运行时填入。子任务过程日志独立保存。</Typography.Text>
      <pre className="oc-todo-run-pre">{JSON.stringify(context,null,2)}</pre>
    </>}]} />}
  </div>;
}
