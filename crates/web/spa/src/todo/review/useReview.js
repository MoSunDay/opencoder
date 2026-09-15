import {useCallback,useEffect,useRef,useState} from 'react';
import {openStream} from '../../sse.js';
import {readOverview} from './api.js';
import {applyFrame} from './model.js';

export function useReview(id) {
  const [snapshot,setSnapshot]=useState(null);
  const [error,setError]=useState('');const [lastSuccess,setLastSuccess]=useState(0);
  const [now,setNow]=useState(Date.now());const [loading,setLoading]=useState(true);
  const epoch=useRef(0);const pending=useRef(false);
  const refresh=useCallback(async()=>{
    if(pending.current)return;
    pending.current=true;const current=epoch.current;
    try{
      const result=await readOverview(id,{signal:AbortSignal.timeout(10000)});
      if(current!==epoch.current)return;
      setSnapshot(prev=>prev?.workflow?.generation>result?.workflow?.generation?prev:result);
      setError('');setLastSuccess(Date.now());
    }catch(e){if(current===epoch.current)setError(e.message);}
    finally{if(current===epoch.current){pending.current=false;setLoading(false);}}
  },[id]);
  useEffect(()=>{
    epoch.current++;pending.current=false;setSnapshot(null);setLoading(true);setLastSuccess(0);setError('');refresh();
    const timer=setInterval(()=>{setNow(Date.now());refresh();},3000);
    return()=>{epoch.current++;clearInterval(timer);};
  },[refresh]);
  const anchor=snapshot?.workflow?`${snapshot.workflow.id}:${snapshot.workflow.world_epoch}:${snapshot.execution_status}`:'';
  const latest=useRef(snapshot);latest.current=snapshot;
  useEffect(()=>{
    if(!anchor)return;
    const stream=openStream({path:`/api/todo/workflows/${encodeURIComponent(id)}/events`,after:latest.current.head_seq,
      onFrame:frame=>{
        if(frame.event==='error'){setError(frame.data?.error||'事件流中断');return;}
        if(frame.event==='stream_end'){refresh();return;}
        setSnapshot(prev=>applyFrame(prev,frame));
        if(frame.data?.omitted||!frame.data?.items||frame.event.startsWith('workflow_'))refresh();
      },onStatus:state=>{if(state==='failed')setError('事件流连接失败');}});
    return()=>stream.abort();
  },[id,anchor,refresh]);
  return {snapshot,error,lastSuccess,loading,refresh,stale:!lastSuccess||now-lastSuccess>10000};
}
