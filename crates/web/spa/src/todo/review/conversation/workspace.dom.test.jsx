// @vitest-environment jsdom
import '../../../test/setup-dom.js';
import {useState} from 'react';
import {afterEach,beforeEach,expect,it,vi} from 'vitest';
import {act,cleanup,fireEvent,render,screen,within} from '@testing-library/react';
import {apiGet} from '../../../api.js';
import {ConversationWorkspace} from './workspace.jsx';
vi.mock('../../../api.js',()=>({apiGet:vi.fn()}));

const snapshot={workflow:{id:'wf',name:'Workflow',status:'running',generation:1,parent_session_id:'parent'},head_seq:1,
  nodes:[{id:'t1',title:'完成实现',status:'passed',depends_on:[],active_session_id:'child'},
    {id:'t2',title:'尚未开始',status:'pending',depends_on:['t1']}]};
const page=(name,seq=1)=>({chunks:[
  {role:'user',blocks:[{kind:'text',text:`${name} context`}]},
  {role:'assistant',blocks:[{kind:'reasoning',text:`${name} reasoning`},{kind:'tool_use',id:'call',name:'bash',input:{command:'pwd'}}]},
  {role:'tool',blocks:[{kind:'tool_result',tool_use_id:'call',content:`/${name}/result`}]},
  {role:'assistant',blocks:[{kind:'text',text:`${name} Say`}]}].map((message,index)=>{
    const bytes=Buffer.from(JSON.stringify(message.blocks));return {...message,seq:seq+index,offset:0,next_offset:bytes.length,total_bytes:bytes.length,eof:true,encoding:'base64',bytes_b64:bytes.toString('base64')};
  }),more:false});
const detail={todo:{id:'t1',title:'完成实现'},state:{status:'passed',session_history:['old','child'],candidate:{summary:'交付完成',result:'可用结果',verification:'检查通过'}}};
const flush=async()=>{for(let i=0;i<8;i++)await act(async()=>{await Promise.resolve();});};
function Harness({value=snapshot,active=true}){const [selected,onSelect]=useState('');return <ConversationWorkspace id="wf" snapshot={value} selected={selected} onSelect={onSelect} active={active}/>;}
const select=id=>fireEvent.click(document.querySelector(`[data-todo-id="${id}"]`));
const parent=()=>fireEvent.click(screen.getByRole('button',{name:'父 Agent',exact:true}));
const expand=scope=>{for(const label of ['1 Step','Step(1)','1 Function call','🔧 bash'])fireEvent.click(within(scope).getByText(label));};
let nodeDetail;
beforeEach(()=>{
  vi.useFakeTimers();vi.clearAllMocks();nodeDetail=structuredClone(detail);
  apiGet.mockImplementation(async path=>{
    const params=new URL(path,'http://local').searchParams;
    if(params.get('section')==='node')return params.get('todo_id')==='t2'?{todo:{id:'t2'},state:{session_history:[]}}:nodeDetail;
    if(params.get('section')==='session_events')return {events:[{seq:1,kind:'tool_result',data:{result:'event detail'}}],more:false};
    return Number(params.get('after_seq'))?{chunks:[],more:false}:page(params.get('session_id'));
  });
});
afterEach(()=>{cleanup();vi.useRealTimers();});

it('starts with parent Say, switches in place and retains expanded steps and independent scroll positions',async()=>{
  render(<Harness/>);await flush();
  const parentPane=screen.getByLabelText('父 Agent 的对话');
  expect(within(parentPane).getByText('parent Say')).toBeTruthy();
  expect(screen.queryByText('parent context')).toBeNull();
  expect(screen.queryByText('/parent/result')).toBeNull();expand(parentPane);
  parentPane.scrollTop=210;fireEvent.scroll(parentPane);
  select('t1');await flush();
  const childPane=screen.getByLabelText('TODO t1 的对话');expand(childPane);
  childPane.scrollTop=140;fireEvent.scroll(childPane);
  fireEvent.click(screen.getByText('执行结果'));expect(screen.getByText('可用结果')).toBeTruthy();
  parent();await flush();
  expect(screen.getByLabelText('父 Agent 的对话')).toBe(parentPane);
  expect(parentPane.hidden).toBe(false);expect(parentPane.scrollTop).toBe(210);
  expect(within(parentPane).getByText('/parent/result')).toBeTruthy();
  select('t1');await flush();expect(childPane.scrollTop).toBe(140);
  expect(within(childPane).getByText('/child/result')).toBeTruthy();expect(screen.getByText('可用结果')).toBeTruthy();
  expect(document.querySelector('.ant-drawer')).toBeNull();
  expect(apiGet.mock.calls.filter(([path])=>path.includes('section=messages')&&path.includes('after_seq=0'))).toHaveLength(2);
});

it('polls only the visible conversation with its own cursor and preserves cached Say after failures',async()=>{
  render(<Harness/>);await flush();select('t1');await flush();apiGet.mockClear();
  await act(async()=>vi.advanceTimersByTime(3000));await flush();
  expect(apiGet.mock.calls).toHaveLength(1);expect(apiGet.mock.calls[0][0]).toContain('session_id=child&after_seq=4');
  parent();await flush();apiGet.mockRejectedValueOnce(new Error('network lost'));
  await act(async()=>vi.advanceTimersByTime(3000));await flush();
  expect(screen.getByText('network lost')).toBeTruthy();expect(screen.getByText('parent Say')).toBeTruthy();
  expect(apiGet.mock.calls.at(-1)[0]).toContain('session_id=parent&after_seq=4');
});

it('keeps inactive parent disclosure open when the current TODO is collapsed with the keyboard',async()=>{
  render(<Harness/>);await flush();expand(screen.getByLabelText('父 Agent 的对话'));
  select('t1');await flush();expand(screen.getByLabelText('TODO t1 的对话'));
  fireEvent.keyDown(window,{key:'l',ctrlKey:true});
  expect(screen.queryByText('/child/result')).toBeNull();parent();await flush();
  expect(screen.getByText('/parent/result')).toBeTruthy();
});

it('opens waiting tasks and follows dependencies without losing the TODO filter',async()=>{
  render(<Harness/>);await flush();select('t2');await flush();
  expect(screen.getByText('该 TODO 尚未开始执行')).toBeTruthy();
  fireEvent.change(screen.getByLabelText('搜索 TODO'),{target:{value:'尚未'}});
  expect(document.querySelector('[data-todo-id="t1"]')).toBeNull();
  fireEvent.click(screen.getByRole('button',{name:'t1',exact:true}));await flush();
  expect(screen.getByText('child Say')).toBeTruthy();expect(screen.getByLabelText('搜索 TODO').value).toBe('尚未');
  parent();await flush();expect(screen.getByText('parent Say')).toBeTruthy();
});

it('preserves a chosen historical session as a new run appears and exposes its execution events',async()=>{
  const view=render(<Harness/>);await flush();select('t1');await flush();
  fireEvent.mouseDown(screen.getByRole('combobox',{name:'执行会话'}));
  fireEvent.click(screen.getByText('会话 1'));await flush();
  expect(screen.getByText('old Say')).toBeTruthy();expect(screen.queryByText('执行结果')).toBeNull();
  nodeDetail={...detail,state:{...detail.state,active_session_id:'new',session_history:['old','child','new']}};
  view.rerender(<Harness value={{...snapshot,head_seq:2,nodes:[{...snapshot.nodes[0],active_session_id:'new'},snapshot.nodes[1]]}}/>);await flush();
  const old=document.querySelector('[data-session-id="old"]');
  expect(old.closest('[hidden]')).toBeNull();expect(document.querySelector('[data-session-id="new"]')).toBeNull();
  fireEvent.click(within(old).getByRole('tab',{name:'执行事件'}));await flush();
  fireEvent.click(within(old).getByText('#1 tool_result'));expect(within(old).getByText(/event detail/)).toBeTruthy();
  parent();await flush();select('t1');await flush();
  expect(within(old).getByRole('tab',{name:'执行事件'}).getAttribute('aria-selected')).toBe('true');
});
