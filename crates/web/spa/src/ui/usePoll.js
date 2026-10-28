// usePoll.js — 可见性感知的轮询 hook（纯函数式，无 class）。
//
// 全仓轮询的共同契约：挂载即首拉（first，通常负责响亮的 loading 遮罩），
// 之后按 intervalMs 静默轮询（poll，不闪遮罩）。document.hidden 时挂起
// 定时器（后台标签页不再空转），visibilitychange 回前台立即补拉一次并
// 重启完整间隔。回调里自己决定错误处理：轮询失败请保留旧数据，只更新
// 错误文案——清空列表会让共享的消费方跟着闪没。
//
// 回调身份变化不重置定时器（ref 持有最新值）；deps 变化（如筛选值）
// 重新执行 first 并重置节奏。

import { useEffect, useRef } from 'react';

export function usePoll({ first, poll, intervalMs, deps = [] }) {
  const firstRef = useRef(first);
  const pollRef = useRef(poll);
  useEffect(() => {
    firstRef.current = first;
    pollRef.current = poll;
  });
  useEffect(() => {
    let timer = null;
    const stop = () => { if (timer !== null) { clearInterval(timer); timer = null; } };
    const runPoll = () => { pollRef.current?.(); };
    const start = () => { stop(); timer = setInterval(runPoll, intervalMs); };
    const onVisibility = () => {
      if (document.hidden) { stop(); return; }
      runPoll(); // 回前台立即补拉，再重启完整间隔。
      start();
    };
    firstRef.current?.();
    if (!document.hidden) start();
    document.addEventListener('visibilitychange', onVisibility);
    return () => {
      stop();
      document.removeEventListener('visibilitychange', onVisibility);
    };
    // deps 是调用方声明的重挂载键（如筛选值），与 intervalMs 一起重置节奏。
  }, [intervalMs, ...deps]);
}
