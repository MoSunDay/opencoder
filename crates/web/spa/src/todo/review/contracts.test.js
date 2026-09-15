import {beforeEach,describe,expect,it,vi} from 'vitest';
import {applyFrame} from './model.js';
import {readOverview,readReview,eventPayload} from './api.js';
import {apiGet} from '../../api.js';
vi.mock('../../api.js',()=>({apiGet:vi.fn()}));
beforeEach(()=>vi.clearAllMocks());
const snapshot={workflow:{id:'todos-1',generation:2,world_epoch:0,status:'running'},head_seq:5,nodes:[{id:'a',status:'pending',attempt:0}],total:1};
const frame={seq:6,event:'todos_dispatched',data:{generation:3,world_epoch:0,workflow_status:'running',items:[{todo_id:'a',status:'running',attempt:1,active_session_id:'session-a'}]}};
describe('TODO review consistency',()=>{
  it('batch dispatch applies exact states once and rejects late frames',()=>{
    const next=applyFrame(snapshot,frame);
    expect(next.nodes[0]).toMatchObject({status:'running',attempt:1,active_session_id:'session-a'});
    expect(applyFrame(next,frame)).toBe(next);
    expect(applyFrame(next,{...frame,seq:7,data:{...frame.data,generation:1}})).toBe(next);
    const failed=applyFrame(next,{...frame,seq:8,data:{...frame.data,generation:4,items:[{todo_id:'a',status:'failed',attempt:3}]}});
    expect(failed.nodes[0].status).toBe('failed');
  });
  it('reads all nodes across pages and retries conflicting generations',async()=>{
    apiGet.mockResolvedValueOnce({...snapshot,total:2,next_ordinal:1})
      .mockRejectedValueOnce(Object.assign(new Error('changed'),{status:409}))
      .mockResolvedValueOnce({...snapshot,total:2,next_ordinal:1})
      .mockResolvedValueOnce({...snapshot,nodes:[{id:'b'}],total:2,next_ordinal:null});
    expect((await readOverview('todos-1')).nodes.map(n=>n.id)).toEqual(['a','b']);
    expect(apiGet).toHaveBeenCalledTimes(4);
  });
  it('reassembles UTF-8 chunks and refuses missing bytes or mismatched identities',async()=>{
    const bytes=new TextEncoder().encode(JSON.stringify({text:'任务结果'.repeat(20000)}));
    const part=(start,end)=>({encoding:'json-base64',etag:'same',offset:start,next_offset:end,total_bytes:bytes.length,eof:end===bytes.length,bytes_b64:Buffer.from(bytes.slice(start,end)).toString('base64')});
    apiGet.mockResolvedValueOnce(part(0,65536)).mockResolvedValueOnce(part(65536,bytes.length));
    expect((await readReview('todos-1',{section:'node'})).text).toHaveLength(80000);
    apiGet.mockResolvedValueOnce(part(0,65536)).mockResolvedValueOnce({...part(65536,bytes.length),etag:'changed'});
    await expect(readReview('todos-1')).rejects.toThrow('不一致');
  });
  it('loads omitted session event bodies through the workflow membership boundary',async()=>{
    const bytes=Buffer.from(JSON.stringify({output:'完整工具结果'}));
    apiGet.mockResolvedValueOnce({offset:0,next_offset:bytes.length,eof:true,bytes_b64:bytes.toString('base64')});
    expect(await eventPayload('todos-1',{seq:9,data:{omitted:true}},{},'session-a')).toEqual({output:'完整工具结果'});
    expect(apiGet.mock.calls[0][0]).toContain('section=session_event_payload&session_id=session-a');
  });
});
