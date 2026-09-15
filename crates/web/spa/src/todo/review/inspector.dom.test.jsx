// @vitest-environment jsdom
import '../../test/setup-dom.js';
import {useEffect} from 'react';
import {afterEach,expect,it,vi} from 'vitest';
import {cleanup,fireEvent,render,screen,waitFor} from '@testing-library/react';
import {NodeInspector} from './inspector.jsx';
vi.mock('./api.js',()=>({readReview:vi.fn(async()=>({todo:{title:'Task',agent:'act',acceptance:{criteria:'tested'}},state:{status:'running',attempt:2}}))}));
vi.mock('./history.jsx',()=>({HistoryPanel({generation,onContext,onSelectContext}){
  useEffect(()=>{onContext({dispatch_seq:generation,world_epoch:generation,context:{text:`latest-${generation}`}});},[generation]);
  return <button onClick={()=>{onContext({dispatch_seq:1,world_epoch:0,context:{text:'historical-context'}},true);onSelectContext();}}>review old attempt</button>;
}}));
afterEach(cleanup);
it('keeps a selected historical attempt while live generations advance',async()=>{
  const view=render(<NodeInspector id="todos-1" todoId="a" generation={2} onSession={()=>{}}/>);
  await screen.findByText('Task');
  fireEvent.click(screen.getByText('review old attempt'));
  await screen.findByText(/historical-context/);
  view.rerender(<NodeInspector id="todos-1" todoId="a" generation={3} onSession={()=>{}}/>);
  await waitFor(()=>expect(screen.getByText(/historical-context/)).toBeTruthy());
  fireEvent.click(screen.getByText('查看最新派发'));
  expect(screen.getByText(/latest-3/)).toBeTruthy();
});
