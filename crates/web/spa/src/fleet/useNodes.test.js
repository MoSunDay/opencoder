// @vitest-environment jsdom
// useNodes.test.js — 节点轮询的行为守卫：5s 间隔不变、失败保留旧节点。
// usePoll 的可见性感知与清理契约见 src/ui/usePoll.test.js。

import '../test/setup-dom.js';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { act, renderHook } from '@testing-library/react';

const { apiGet } = vi.hoisted(() => ({ apiGet: vi.fn() }));
vi.mock('../api.js', () => ({ apiGet }));

import { useNodes } from './useNodes.js';

const flush = async (rounds = 6) => {
  for (let i = 0; i < rounds; i += 1) await act(async () => { await Promise.resolve(); });
};

beforeEach(() => { vi.useFakeTimers(); });
afterEach(() => { vi.useRealTimers(); apiGet.mockReset(); });

it('keeps the last node list when the 5s poll fails, instead of flashing it away', async () => {
  apiGet.mockResolvedValueOnce({ nodes: [{ id: 'n1', name: 'worker', online: true }] })
    .mockRejectedValueOnce(new Error('offline'));
  const { result } = renderHook(() => useNodes());
  await flush();
  expect(result.current.nodes).toHaveLength(1);
  expect(result.current.error).toBe('');
  await act(async () => { vi.advanceTimersByTime(5000); });
  await flush();
  // 失败只更新错误文案；清空节点会让调度/启动/总览三个消费方一起闪没。
  expect(result.current.nodes).toEqual([{ id: 'n1', name: 'worker', online: true }]);
  expect(result.current.error).toBe('获取节点失败: offline');
});
