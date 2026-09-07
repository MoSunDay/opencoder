import { useEffect, useState } from 'react';
import { apiGet } from '../api.js';

export function useNodes() {
  const [nodes, setNodes] = useState([]);
  const [error, setError] = useState('');
  useEffect(() => {
    let active = true;
    const load = async () => {
      try {
        const result = await apiGet('/api/nodes');
        if (active) { setNodes(result.nodes || []); setError(''); }
      } catch (e) {
        if (active) { setNodes([]); setError('获取节点失败: ' + e.message); }
      }
    };
    load(); const timer = setInterval(load, 5000);
    return () => { active = false; clearInterval(timer); };
  }, []);
  return { nodes, error };
}
