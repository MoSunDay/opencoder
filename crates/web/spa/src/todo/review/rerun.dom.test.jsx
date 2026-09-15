// @vitest-environment jsdom
import '../../test/setup-dom.js';
import {afterEach,expect,it,vi} from 'vitest';
import {cleanup,fireEvent,render,screen,waitFor} from '@testing-library/react';
import {RerunDialog} from './rerun.jsx';
import {apiGet,apiPost} from '../../api.js';
vi.mock('../../api.js',()=>({apiGet:vi.fn(),apiPost:vi.fn()}));
afterEach(()=>{cleanup();vi.clearAllMocks();});
it('keeps one request identity across a lost response and closes only after durable queuing',async()=>{
  apiGet.mockResolvedValue({preview:{generation:9,affected:['b','c'],preserved:['a'],blockers:[]}});
  apiPost.mockRejectedValueOnce(new Error('connection lost')).mockResolvedValueOnce({request_id:'pending',phase:'stopping'});
  const close=vi.fn(),accepted=vi.fn();
  const view=render(<RerunDialog id="todos-1" todoId="b" snapshot={{controls:[]}} onClose={close} onAccepted={accepted}/>);
  await screen.findByText('重新执行：');
  fireEvent.change(screen.getByLabelText('重跑原因'),{target:{value:'review correction'}});
  fireEvent.click(screen.getByText('确认暂停并重跑'));
  await screen.findByText('connection lost');
  fireEvent.click(screen.getByText('确认暂停并重跑'));
  await screen.findByText('已受理，等待旧执行停止');
  expect(apiPost.mock.calls[0][1]).toEqual(apiPost.mock.calls[1][1]);
  expect(apiPost.mock.calls[0][1]).toMatchObject({todo_id:'b',reason:'review correction',expected_generation:9});
  expect(close).not.toHaveBeenCalled();
  view.rerender(<RerunDialog id="todos-1" todoId="b" snapshot={{controls:[{request_id:'pending',phase:'queued'}]}} onClose={close} onAccepted={accepted}/>);
  await waitFor(()=>expect(close).toHaveBeenCalledOnce());
});
it('unaccepted prerequisites prevent the rerun request',async()=>{
  apiGet.mockResolvedValue({preview:{generation:9,affected:['b'],preserved:['a'],blockers:['a']}});
  render(<RerunDialog id="todos-1" todoId="b" snapshot={{controls:[]}} onClose={()=>{}}/>);
  await screen.findByText('前置任务尚未通过：a');
  fireEvent.change(screen.getByLabelText('重跑原因'),{target:{value:'retry'}});
  expect(screen.getByText('确认暂停并重跑').closest('button').disabled).toBe(true);
  expect(apiPost).not.toHaveBeenCalled();
});
