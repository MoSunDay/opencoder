import {useEffect,useRef,useState} from 'react';
import {appendMessagePage} from '../../../fleet/model.js';
import {eventPayload,readReview} from '../api.js';

export function useSession(id,sessionId,active,tab) {
  const [data,setData]=useState({messages:null,events:null,messageError:'',eventError:'',messageBusy:false,eventBusy:false});
  const current=useRef(data);const epoch=useRef(0);const pending=useRef({});const tail=useRef(0);const eventCursor=useRef(0);
  current.current=data;
  useEffect(()=>{pending.current={};return()=>{epoch.current++;};},[]);
  const publish=patch=>{current.current={...current.current,...patch};setData(current.current);};
  const load=async(kind,reset=false,silent=false)=>{
    if(pending.current[kind])return;
    pending.current[kind]=true;const owner=epoch.current;
    const prefix=kind==='messages'?'message':'event';
    publish({[`${prefix}Busy`]:!silent,[`${prefix}Error`]:''});
    try {
      const previous=current.current.messages;
      const cursor=reset?{seq:0,offset:0}:previous?.nextCursor || {seq:tail.current,offset:0};
      const query=kind==='messages'?{section:kind,session_id:sessionId,after_seq:cursor.seq,message_offset:cursor.offset}
        :{section:'session_events',session_id:sessionId,after_seq:reset?0:eventCursor.current};
      const page=await readReview(id,query,{signal:AbortSignal.timeout(10000)});
      if(kind==='messages') {
        if(!Array.isArray(page?.chunks))throw new Error('会话消息响应格式异常');
        if(page.more && (!page.next_cursor || page.next_cursor.seq===cursor.seq&&page.next_cursor.offset===cursor.offset))throw new Error('会话消息分页未推进');
        const next=appendMessagePage(reset?null:previous,page,reset?undefined:previous?.large?.at(-1)?.tail);
        if(owner!==epoch.current)return;
        if(reset)tail.current=0;
        const last=page.chunks.at(-1);if(last?.eof)tail.current=last.seq;
        if(reset||!previous||page.chunks.length)publish({messages:next});
      }else{
        if(!Array.isArray(page?.events))throw new Error('会话事件响应格式异常');
        if(page.more&&(!page.events.length||page.events.at(-1).seq<=(reset?0:eventCursor.current)))throw new Error('会话事件分页未推进');
        const events=await Promise.all(page.events.map(async event=>({...event,data:await eventPayload(id,event,{signal:AbortSignal.timeout(10000)},sessionId)})));
        if(owner!==epoch.current)return;
        eventCursor.current=events.at(-1)?.seq || (reset?0:eventCursor.current);
        publish({events:[...new Map([...(reset?[]:current.current.events||[]),...events].map(event=>[event.seq,event])).values()],moreEvents:!!page.more});
      }
    }catch(error){if(owner===epoch.current)publish({[`${prefix}Error`]:error.message});}
    finally{if(owner===epoch.current){pending.current[kind]=false;publish({[`${prefix}Busy`]:false});}}
  };
  const latest=useRef();latest.current={load,tab};
  useEffect(()=>{
    if(!active)return undefined;
    const update=(initial=false)=>{
      const kind=latest.current.tab==='events'?'events':'messages';
      const value=current.current;
      const empty=value[kind]===null;
      if(initial&&empty || !initial&&(kind==='messages'?!value.messages?.more:!value.moreEvents))latest.current.load(kind,false,!empty);
    };
    update(true);
    const timer=setInterval(()=>update(),3000);
    return()=>clearInterval(timer);
  },[active,tab,id,sessionId]);
  useEffect(()=>{
    const messages=data.messages;
    const hasSay=messages?.messages?.some(message=>message.role==='assistant'&&message.blocks?.some(block=>block.kind==='text'&&block.text?.trim()))
      || messages?.large?.some(message=>message.role==='assistant');
    if(active&&tab==='say'&&messages?.more&&!hasSay&&!data.messageBusy&&!data.messageError)latest.current.load('messages');
  },[active,tab,data.messages,data.messageBusy,data.messageError]);
  return {...data,load};
}
