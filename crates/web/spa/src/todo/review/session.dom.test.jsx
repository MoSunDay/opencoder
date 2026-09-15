// @vitest-environment jsdom
import '../../test/setup-dom.js';
import {afterEach,beforeEach,expect,it,vi} from 'vitest';
import {act,cleanup,render} from '@testing-library/react';
import {SessionReview} from './session.jsx';
import {apiGet} from '../../api.js';
vi.mock('../../api.js',()=>({apiGet:vi.fn()}));
vi.mock('../../transcript.jsx',()=>({TranscriptView:({turns})=><div data-testid="transcript">{turns.length}</div>}));
const flush=async()=>{for(let i=0;i<5;i++)await act(async()=>{await Promise.resolve();});};
beforeEach(()=>{vi.useFakeTimers();vi.clearAllMocks();});
afterEach(()=>{cleanup();vi.useRealTimers();});
it('polls after the last complete message and does not reload the transcript from zero',async()=>{
  const bytes=Buffer.from(JSON.stringify([{kind:'text',text:'recorded decision'}]));
  apiGet.mockResolvedValueOnce({chunks:[{seq:7,offset:0,next_offset:bytes.length,total_bytes:bytes.length,eof:true,encoding:'base64',bytes_b64:bytes.toString('base64'),role:'assistant',created_at:1}],more:false})
    .mockResolvedValue({chunks:[],more:false});
  render(<SessionReview id="todos-1" sessionId="parent-1" onClose={()=>{}}/>);
  await flush();
  await act(async()=>{vi.advanceTimersByTime(3000);});await flush();
  expect(apiGet).toHaveBeenCalledTimes(2);
  expect(apiGet.mock.calls[1][0]).toContain('after_seq=7&message_offset=0');
  await act(async()=>{vi.advanceTimersByTime(3000);});await flush();
  expect(apiGet.mock.calls[2][0]).toContain('after_seq=7&message_offset=0');
});
