// @vitest-environment jsdom
import '../../test/setup-dom.js';
import {afterEach,expect,it,vi} from 'vitest';
import {cleanup,fireEvent,render,screen,waitFor} from '@testing-library/react';
import {HistoryPanel} from './history.jsx';
import {readReview,eventPayload} from './api.js';
vi.mock('./api.js',()=>({readReview:vi.fn(),eventPayload:vi.fn(async(_,e)=>e.payload)}));
afterEach(()=>{cleanup();vi.clearAllMocks();});
it('reads sparse history backward with one bounded request per page and selects the latest dispatch',async()=>{
  const dispatch=seq=>({seq,kind:'todos_dispatched',payload:{assignments:[{todo_id:'a',context:{seq}}]}});
  readReview.mockResolvedValueOnce({events:[dispatch(900),dispatch(800)],more:true,next_before_seq:800})
    .mockResolvedValueOnce({events:[dispatch(1)],more:false,next_before_seq:null});
  const onContext=vi.fn();
  render(<HistoryPanel id="todos-1" todoId="a" onContext={onContext} generation={1}/>);
  await screen.findByText('#900');
  expect(onContext.mock.calls[0][0].dispatch_seq).toBe(900);
  expect(readReview).toHaveBeenCalledTimes(1);
  fireEvent.click(screen.getByText('更早记录'));
  await screen.findByText('#1');
  await waitFor(()=>expect(screen.getByText('更早记录').closest('button').disabled).toBe(true));
  expect(readReview).toHaveBeenLastCalledWith('todos-1',{section:'history',before_seq:800});
  expect(eventPayload).toHaveBeenCalledTimes(3);
  expect(screen.getByText('#900')).toBeTruthy();
});
