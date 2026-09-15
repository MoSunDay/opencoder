import { apiGet } from '../../api.js';

export function reviewPath(id,query={}) {
  const params=new URLSearchParams(Object.entries(query).filter(([,v])=>v!==undefined&&v!==null));
  return `/api/todo/workflows/${encodeURIComponent(id)}/review?${params}`;
}

export function bytesFromBase64(value) {return Uint8Array.from(atob(value),c=>c.charCodeAt(0));}

export async function readReview(id,query={},opts={}) {
  let value=await apiGet(reviewPath(id,query),opts);
  if(value?.encoding!=='json-base64') return value;
  const parts=[];const etag=value.etag;let offset=0;
  for(;;){
    const bytes=bytesFromBase64(value.bytes_b64||'');
    if(value.etag!==etag||value.offset!==offset||value.next_offset!==offset+bytes.length||value.next_offset>value.total_bytes)
      throw new Error('Review 分段数据不一致');
    parts.push(bytes);offset=value.next_offset;
    if(value.eof)break;
    if(!bytes.length)throw new Error('Review 分段读取未推进');
    value=await apiGet(reviewPath(id,{...query,offset,etag}),opts);
  }
  const output=new Uint8Array(offset);let cursor=0;
  for(const bytes of parts){output.set(bytes,cursor);cursor+=bytes.length;}
  return JSON.parse(new TextDecoder().decode(output));
}

export async function readOverview(id,opts={}) {
  for(let attempt=0;attempt<3;attempt++) {
    try {
      const first=await readReview(id,{section:'overview'},opts);
      if(first?.initializing)return first;
      if(!first?.workflow||!Array.isArray(first.nodes))throw new Error('Review 快照格式异常');
      const nodes=[...first.nodes];let cursor=first.next_ordinal;
      while(cursor!==null&&cursor!==undefined){
        const next=await readReview(id,{section:'overview',after_ordinal:cursor,generation:first.workflow.generation},opts);
        if(next.workflow?.generation!==first.workflow.generation)throw new Error('Review 分页版本不一致');
        if(next.next_ordinal!==null&&next.next_ordinal<=cursor)throw new Error('Review 分页未推进');
        nodes.push(...next.nodes);cursor=next.next_ordinal;
      }
      if(nodes.length!==first.total||new Set(nodes.map(n=>n.id)).size!==first.total)throw new Error('Review 节点不完整');
      return {...first,nodes};
    }catch(e){if(e.status!==409||attempt===2)throw e;}
  }
}

export async function eventPayload(id,event,opts={},sessionId) {
  const payload=sessionId?event.data:event.payload;
  if(!payload?.omitted)return payload;
  const parts=[];let offset=0;
  for(;;){
    const value=sessionId
      ?await readReview(id,{section:'session_event_payload',session_id:sessionId,after_seq:event.seq,offset},opts)
      :await apiGet(`/api/executions/${encodeURIComponent(id)}/events/${event.seq}/payload?offset=${offset}`,opts);
    const bytes=bytesFromBase64(value.bytes_b64||'');
    if(value.offset!==offset||value.next_offset!==offset+bytes.length)throw new Error('事件分段数据不一致');
    parts.push(bytes);offset=value.next_offset;if(value.eof)break;
    if(!bytes.length)throw new Error('事件分段读取未推进');
  }
  const joined=new Uint8Array(offset);let position=0;for(const bytes of parts){joined.set(bytes,position);position+=bytes.length;}
  return JSON.parse(new TextDecoder().decode(joined));
}
