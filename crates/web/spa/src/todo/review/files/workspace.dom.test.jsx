// @vitest-environment jsdom
import '../../../test/setup-dom.js';
import {afterEach,expect,it,vi} from 'vitest';
import {cleanup,fireEvent,render,screen,waitFor} from '@testing-library/react';
import {ReviewFiles} from './workspace.jsx';
import {readReview,eventPayload} from '../api.js';
vi.mock('../api.js',()=>({readReview:vi.fn(),eventPayload:vi.fn(async(_,event)=>event.payload)}));
vi.mock('../../../ui/files/workspace.jsx',()=>({FileWorkspace:({files,selected,onSelect})=><div>
  <output data-testid="content">{files[selected]}</output>
  {Object.keys(files).map(path=><button key={path} onClick={()=>onSelect(path)}>{path}</button>)}
</div>}));
afterEach(()=>{cleanup();vi.clearAllMocks();});
const snapshot={head_seq:10,workflow:{id:'wf',parent_session_id:'parent'},nodes:[{id:'a',title:'A',status:'passed'}]};
const dispatch={seq:1,kind:'todos_dispatched',payload:{assignments:[{todo_id:'a',session_id:'child',context:{objective:'original context'}}]}};
const result={seq:9,kind:'todo_candidate_ready',payload:{todo_id:'a',candidate:{result:'complete result'}}};
const props={id:'wf',snapshot,onTodo:vi.fn(),onSession:vi.fn(),onError:vi.fn()};

it('loads older attempts and retains the selected context when live history refreshes',async()=>{
  readReview.mockImplementation(async(_,query)=>query.section==='files'?{files:{'objective.md':'frozen'}}:query.before_seq?{events:[dispatch],next_before_seq:null}:{events:[result],next_before_seq:9});
  const view=render(<ReviewFiles {...props}/>);
  await screen.findByText('process/todos/a/records/000000000009-result.json');
  fireEvent.click(screen.getByText('更早记录'));
  const context='process/todos/a/attempts/000000000001/context.json';
  fireEvent.click(await screen.findByText(context));
  expect(screen.getByTestId('content').textContent).toContain('original context');
  expect(props.onTodo).toHaveBeenLastCalledWith('a');
  view.rerender(<ReviewFiles {...props} snapshot={{...snapshot,head_seq:11}}/>);
  await waitFor(()=>expect(readReview.mock.calls.filter(c=>c[1].section==='history')).toHaveLength(3));
  expect(screen.getByTestId('content').textContent).toContain('original context');
  fireEvent.click(screen.getByText('process/todos/a/attempts/000000000001/session.json'));
  fireEvent.click(screen.getByText('查看该次会话'));
  expect(props.onSession).toHaveBeenCalledWith('child');
});

it('definition read failures remain visible until a successful retry',async()=>{
  let failed=true;
  readReview.mockImplementation(async(_,query)=>{
    if(query.section==='files'){if(failed)throw new Error('definition unavailable');return {files:{'objective.md':'frozen'}};}
    return {events:[],next_before_seq:null};
  });
  render(<ReviewFiles {...props}/>);
  await screen.findByText('definition unavailable');
  expect(props.onError).toHaveBeenLastCalledWith('definition unavailable');
  failed=false;fireEvent.click(screen.getByText('刷新过程记录'));
  await screen.findByText('definition/objective.md');
  await waitFor(()=>expect(screen.queryByText('definition unavailable')).toBeNull());
  expect(props.onError).toHaveBeenLastCalledWith('');
});

it('hydrates omitted history payloads before displaying complete result files',async()=>{
  readReview.mockImplementation(async(_,query)=>query.section==='files'?{files:{'objective.md':'frozen'}}:{events:[{...result,payload:{omitted:true}}],next_before_seq:null});
  eventPayload.mockResolvedValueOnce({todo_id:'a',candidate:{result:'完整结果'.repeat(20000)}});
  render(<ReviewFiles {...props}/>);
  fireEvent.click(await screen.findByText('process/todos/a/records/000000000009-result.json'));
  expect(screen.getByTestId('content').textContent).toContain('完整结果'.repeat(20000));
});

it('fetches the newest history after a snapshot changes during an in-flight request',async()=>{
  let finish;let calls=0;
  readReview.mockImplementation(async(_,query)=>{
    if(query.section==='files')return {files:{'objective.md':'frozen'}};
    if(++calls===1)return new Promise(resolve=>{finish=resolve;});
    return {events:[dispatch,result],next_before_seq:null};
  });
  const view=render(<ReviewFiles {...props}/>);
  await screen.findByText('definition/objective.md');
  view.rerender(<ReviewFiles {...props} snapshot={{...snapshot,head_seq:11}}/>);
  finish({events:[],next_before_seq:null});
  await screen.findByText('process/todos/a/attempts/000000000001/000000000009-result.json');
  expect(calls).toBe(2);
});
