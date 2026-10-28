import { useCallback, useEffect, useRef, useState } from 'react';
import { apiGet } from '../api.js';
import { usePoll } from '../ui/usePoll.js';

export function useNodes() {
  const [nodes, setNodes] = useState([]);
  const [error, setError] = useState('');
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => { alive.current = false; };
  }, []);
  const load = useCallback(async () => {
    try {
      const result = await apiGet('/api/nodes');
      if (alive.current) { setNodes(result.nodes || []); setError(''); }
    } catch (e) {
      // 失败保留旧节点，只更新错误文案：清空会让共享方（调度、启动、总览）闪没。
      if (alive.current) setError('获取节点失败: ' + e.message);
    }
  }, []);
  usePoll({ first: load, poll: load, intervalMs: 5000 });
  return { nodes, error };
}
