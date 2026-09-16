// @vitest-environment jsdom
import '../../../test/setup-dom.js';
import {afterEach,beforeEach,expect,it,vi} from 'vitest';
import {act,cleanup,fireEvent,render,screen} from '@testing-library/react';
import {apiGet} from '../../../api.js';
import {SessionConversation} from './session.jsx';
vi.mock('../../../api.js',()=>({apiGet:vi.fn()}));
const flush=async()=>{for(let i=0;i<10;i++)await act(async()=>{await Promise.resolve();});};
const chunk=(seq,role,text)=>{const bytes=Buffer.from(JSON.stringify([{kind:'text',text}]));return {seq,role,offset:0,next_offset:bytes.length,total_bytes:bytes.length,eof:true,encoding:'base64',bytes_b64:bytes.toString('base64')};};
beforeEach(()=>{vi.useFakeTimers();vi.clearAllMocks();});
afterEach(()=>{cleanup();vi.useRealTimers();});

it('continues past prompt-only pages to the first Say and keeps the original decision available',async()=>{
  const raw=JSON.stringify({operation:'complete',reason:'验证完成'});
  apiGet.mockResolvedValueOnce({chunks:[chunk(1,'user','large context')],more:true,next_cursor:{seq:1,offset:0}})
    .mockResolvedValueOnce({chunks:[chunk(2,'assistant',raw)],more:false}).mockResolvedValue({chunks:[],more:false});
  render(<SessionConversation id="wf" sessionId="parent"/>);await flush();
  expect(apiGet).toHaveBeenCalledTimes(2);expect(apiGet.mock.calls[1][0]).toContain('after_seq=1');
  expect(screen.getByText('工作流完成')).toBeTruthy();expect(screen.getByText('验证完成')).toBeTruthy();
  expect(screen.queryByText('large context')).toBeNull();
  fireEvent.click(screen.getByText('原始回复'));expect(screen.getByText(raw)).toBeTruthy();
  fireEvent.click(screen.getByText('输入与上下文'));expect(screen.getByText('large context')).toBeTruthy();
});

it('reports stalled pagination without looping and retries from the last valid cursor',async()=>{
  apiGet.mockResolvedValueOnce({chunks:[chunk(1,'user','context')],more:true,next_cursor:{seq:1,offset:0}})
    .mockResolvedValueOnce({chunks:[],more:true,next_cursor:{seq:1,offset:0}})
    .mockResolvedValue({chunks:[chunk(2,'assistant','recovered Say')],more:false});
  render(<SessionConversation id="wf" sessionId="parent"/>);await flush();
  expect(screen.getByText('会话消息分页未推进')).toBeTruthy();expect(apiGet).toHaveBeenCalledTimes(2);
  await act(async()=>vi.advanceTimersByTime(6000));await flush();expect(apiGet).toHaveBeenCalledTimes(2);
  fireEvent.click(screen.getByRole('button',{name:/重\s*试/}));await flush();
  expect(apiGet.mock.calls[2][0]).toContain('after_seq=1');expect(screen.getByText('recovered Say')).toBeTruthy();
});

it('does not carry a late response across keyed session changes',async()=>{
  let resolve;
  apiGet.mockImplementation(path=>path.includes('session_id=old')?new Promise(done=>{resolve=done;}):Promise.resolve({chunks:[chunk(1,'assistant','new Say')],more:false}));
  const view=render(<SessionConversation key="old" id="wf" sessionId="old"/>);await flush();
  view.rerender(<SessionConversation key="new" id="wf" sessionId="new"/>);await flush();
  await act(async()=>resolve({chunks:[chunk(1,'assistant','late old Say')],more:false}));await flush();
  expect(screen.getByText('new Say')).toBeTruthy();expect(screen.queryByText('late old Say')).toBeNull();
});
