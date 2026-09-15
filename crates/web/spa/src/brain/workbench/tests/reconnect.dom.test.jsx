// @vitest-environment jsdom
import {act,cleanup,renderHook,waitFor} from '@testing-library/react';
import {afterEach,expect,it,vi} from 'vitest';
import {useBrainRun} from '../useRun.js';
import {apiGet} from '../../../api.js';
import {openStream} from '../../../sse.js';
vi.mock('../../../api.js',()=>({apiGet:vi.fn()}));
vi.mock('../../../sse.js',()=>({openStream:vi.fn(()=>({abort(){}}))}));
afterEach(()=>{cleanup();vi.useRealTimers();vi.clearAllMocks();});
it('reloads the same brain run after an activation stream closes and stops at its terminal state',async()=>{
  apiGet.mockResolvedValueOnce({phase:'running',watermark:1}).mockResolvedValueOnce({phase:'completed',watermark:8});
  const {result}=renderHook(()=>useBrainRun('brain-1'));
  await waitFor(()=>expect(openStream).toHaveBeenCalledTimes(1));
  vi.useFakeTimers();
  act(()=>openStream.mock.calls[0][0].onStatus('closed'));
  await act(async()=>vi.advanceTimersByTimeAsync(2000));
  expect(result.current.run.phase).toBe('completed');
  expect(apiGet).toHaveBeenCalledTimes(2);
  await act(async()=>vi.advanceTimersByTimeAsync(10000));
  expect(openStream).toHaveBeenCalledTimes(1);
  expect(apiGet).toHaveBeenLastCalledWith('/api/brain/runs/brain-1');
});
