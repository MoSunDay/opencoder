import {Alert,Button,Select,Space,Spin} from 'antd';
import {useEffect,useMemo,useRef,useState} from 'react';
import {FileWorkspace} from '../../../ui/files/workspace.jsx';
import {eventPayload,readReview} from '../api.js';
import {pathTodo,reviewFiles} from './model.js';

export function ReviewFiles({id,snapshot,onTodo,onSession,onError}) {
  const [definition,setDefinition]=useState({});const [events,setEvents]=useState([]);
  const [selected,setSelected]=useState('definition/objective.md');
  const [before,setBefore]=useState(null);const [busy,setBusy]=useState(false);
  const [error,setError]=useState('');const [definitionError,setDefinitionError]=useState('');const [filter,setFilter]=useState('');
  const alive=useRef(true);const pending=useRef(false);const queued=useRef(false);const callbacks=useRef({});callbacks.current={onError};
  const load=async(cursor)=>{
    if(pending.current){if(!cursor)queued.current=true;return;}
    pending.current=true;setBusy(true);
    try{
      const page=await readReview(id,{section:'history',before_seq:cursor},{signal:AbortSignal.timeout(10000)});
      const rows=await Promise.all(page.events.map(async event=>({...event,payload:await eventPayload(id,event,{signal:AbortSignal.timeout(10000)})})));
      if(!alive.current)return;
      setEvents(previous=>[...new Map([...previous,...rows].map(event=>[event.seq,event])).values()]);
      setBefore(previous=>cursor || previous===null ? page.next_before_seq : previous);
      setError('');
    }catch(e){if(alive.current)setError(e.message);}
    finally{pending.current=false;if(alive.current){setBusy(false);if(queued.current){queued.current=false;load();}}}
  };
  const loadDefinition=async()=>{
    try {
      const result=await readReview(id,{section:'files'},{signal:AbortSignal.timeout(10000)});
      if(!result.files || !Object.keys(result.files).length)throw new Error('运行定义缺少文件');
      if(alive.current){setDefinition(result.files);setDefinitionError('');}
    }catch(e){if(alive.current)setDefinitionError(e.message);}
  };
  useEffect(()=>{
    alive.current=true;loadDefinition();
    return()=>{alive.current=false;};
  },[id]);
  useEffect(()=>{load();},[id,snapshot?.head_seq]);
  useEffect(()=>{callbacks.current.onError?.(definitionError||error);},[definitionError,error]);
  const projection=useMemo(()=>reviewFiles(definition,snapshot,events),[definition,snapshot,events]);
  const files=useMemo(()=>Object.fromEntries(Object.entries(projection.files).filter(([path])=>{
    const todo=pathTodo(path);return !filter || !todo || snapshot.nodes.find(node=>node.id===todo)?.status===filter;
  })),[projection,filter,snapshot]);
  const select=path=>{setSelected(path);onTodo(pathTodo(path));};
  return <>
    {(definitionError||error)&&<Alert type="error" title="读取运行文件失败" description={definitionError||error}/>}
    <Space wrap style={{margin:'8px 0'}}>
      <Select aria-label="筛选任务状态" value={filter} onChange={setFilter} style={{minWidth:150}}
        options={[{value:'',label:'全部状态'},...Array.from(new Set((snapshot?.nodes||[]).map(node=>node.status))).map(status=>({value:status,label:status}))]}/>
      <Select aria-label="选择 TODO" placeholder="选择 TODO" showSearch optionFilterProp="label" style={{minWidth:200}} value={pathTodo(selected)||undefined}
        onChange={todo=>select(`process/todos/${todo}/status.json`)} options={(snapshot?.nodes||[]).filter(node=>!filter||node.status===filter).map(node=>({value:node.id,label:`${node.id} · ${node.title} · ${node.status}`}))}/>
      <Button loading={busy} onClick={()=>{load();loadDefinition();}}>刷新过程记录</Button>
      <Button disabled={!before||busy} onClick={()=>load(before)}>更早记录</Button>
      {projection.sessions[selected]&&<Button onClick={()=>onSession(projection.sessions[selected])}>查看该次会话</Button>}
    </Space>
    {!Object.keys(definition).length&&!error&&!definitionError?<Spin/>:<FileWorkspace files={files} selected={selected} onSelect={select} readOnly/>}
  </>;
}
