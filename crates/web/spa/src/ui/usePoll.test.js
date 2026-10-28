// @vitest-environment jsdom
// usePoll.test.js — 轮询 hook 契约的行为守卫：首拉、静默轮询、后台挂起、
// 回前台补拉、隐藏挂载不布防、卸载清理、失败不打断节奏、deps 重置。

import '../test/setup-dom.js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, renderHook } from '@testing-library/react';
import { usePoll } from './usePoll.js';

const INTERVAL = 5000;

let hiddenNow = false;
const setHidden = (value) => {
  hiddenNow = value;
  Object.defineProperty(document, 'hidden', { configurable: true, get: () => hiddenNow });
  document.dispatchEvent(new Event('visibilitychange'));
};
const advance = (ms) => act(() => { vi.advanceTimersByTime(ms); });
const flush = () => act(async () => {});

beforeEach(() => {
  vi.useFakeTimers();
  setHidden(false); // 重定义 getter，保证用例顺序无关。
});

afterEach(() => { vi.useRealTimers(); });

describe('usePoll', () => {
  it('mount runs first once, then polls every intervalMs while visible', () => {
    const first = vi.fn();
    const poll = vi.fn();
    renderHook(() => usePoll({ first, poll, intervalMs: INTERVAL }));
    expect(first).toHaveBeenCalledTimes(1);
    expect(poll).not.toHaveBeenCalled();
    advance(INTERVAL);
    expect(poll).toHaveBeenCalledTimes(1);
    advance(INTERVAL);
    expect(poll).toHaveBeenCalledTimes(2);
    expect(first).toHaveBeenCalledTimes(1);
  });

  it('suspends polling while the tab is hidden', () => {
    const first = vi.fn();
    const poll = vi.fn();
    renderHook(() => usePoll({ first, poll, intervalMs: INTERVAL }));
    act(() => setHidden(true));
    advance(INTERVAL * 5);
    expect(poll).not.toHaveBeenCalled();
    expect(first).toHaveBeenCalledTimes(1);
  });

  it('fires one immediate catch-up poll on returning to foreground, then restarts the interval', () => {
    const poll = vi.fn();
    renderHook(() => usePoll({ first: vi.fn(), poll, intervalMs: INTERVAL }));
    act(() => setHidden(true));
    advance(INTERVAL * 3);
    expect(poll).not.toHaveBeenCalled();
    act(() => setHidden(false));
    expect(poll).toHaveBeenCalledTimes(1); // 立即补拉，无需推进时钟
    advance(INTERVAL - 1);
    expect(poll).toHaveBeenCalledTimes(1);
    advance(1);
    expect(poll).toHaveBeenCalledTimes(2);
  });

  it('runs first when mounting hidden but does not arm the timer', () => {
    const first = vi.fn();
    const poll = vi.fn();
    setHidden(true);
    renderHook(() => usePoll({ first, poll, intervalMs: INTERVAL }));
    expect(first).toHaveBeenCalledTimes(1);
    advance(INTERVAL * 4);
    expect(poll).not.toHaveBeenCalled();
  });

  it('clears the timer and removes the listener on unmount', () => {
    const first = vi.fn();
    const poll = vi.fn();
    const { unmount } = renderHook(() => usePoll({ first, poll, intervalMs: INTERVAL }));
    unmount();
    advance(INTERVAL * 3);
    expect(poll).not.toHaveBeenCalled();
    act(() => setHidden(false)); // 监听器已摘除：回前台不再补拉
    expect(poll).not.toHaveBeenCalled();
    expect(first).toHaveBeenCalledTimes(1);
  });

  it('keeps polling on schedule after a failing poll that keeps old data', async () => {
    // 拒绝在回调内部消化（保留旧数据），不产生 unhandled rejection。
    const poll = vi.fn(async () => {
      try { await Promise.reject(new Error('boom')); } catch { /* keep old data */ }
    });
    renderHook(() => usePoll({ first: vi.fn(), poll, intervalMs: INTERVAL }));
    advance(INTERVAL);
    await flush();
    expect(poll).toHaveBeenCalledTimes(1);
    advance(INTERVAL);
    await flush();
    expect(poll).toHaveBeenCalledTimes(2); // 上一轮失败不影响下一轮节奏
  });

  it('re-runs first and resets the rhythm when deps change', () => {
    const first = vi.fn();
    const poll = vi.fn();
    const { rerender } = renderHook(
      ({ dep }) => usePoll({ first, poll, intervalMs: INTERVAL, deps: [dep] }),
      { initialProps: { dep: 1 } },
    );
    expect(first).toHaveBeenCalledTimes(1);
    advance(INTERVAL);
    expect(poll).toHaveBeenCalledTimes(1);
    act(() => rerender({ dep: 2 }));
    expect(first).toHaveBeenCalledTimes(2);
    advance(INTERVAL - 1);
    expect(poll).toHaveBeenCalledTimes(1); // 节奏已随 deps 重置
    advance(1);
    expect(poll).toHaveBeenCalledTimes(2);
  });
});
