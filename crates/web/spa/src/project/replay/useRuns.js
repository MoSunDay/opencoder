import { useCallback, useEffect, useRef, useState } from 'react';
import { apiGet } from '../../api.js';

export function mergeRuns(old, incoming) {
  return [...new Map([...old, ...incoming].map((run) => [run.id, run])).values()]
    .sort((a, b) => b.version - a.version);
}
export function useRuns(todoId, running) {
  const [runs, setRuns] = useState([]);
  const [cursor, setCursor] = useState(null);
  const [error, setError] = useState('');
  const [updated, setUpdated] = useState(null);
  const [busy, setBusy] = useState(false);
  const generation = useRef(0);
  const paging = useRef(false);
  const load = useCallback(async (before = null) => {
    if (!todoId) return;
    const current = generation.current;
    setBusy(true);
    try {
      const suffix = before === null ? '' : `?before_version=${before}`;
      const page = await apiGet(`/api/project/todos/${encodeURIComponent(todoId)}/runs${suffix}`);
      if (current !== generation.current) return;
      setRuns((old) => mergeRuns(old, page?.runs || []));
      if (before !== null || !paging.current) setCursor(page?.next_version ?? null);
      if (before !== null) paging.current = true;
      setUpdated(Date.now()); setError('');
    } catch (e) {
      if (current === generation.current) setError(`获取执行记录失败: ${e.message}`);
    } finally {
      if (current === generation.current) setBusy(false);
    }
  }, [todoId]);
  useEffect(() => {
    generation.current += 1; paging.current = false;
    setRuns([]); setCursor(null); setError(''); setUpdated(null);
    load();
    return () => { generation.current += 1; };
  }, [load]);
  const active = running || runs.some((run) => run.status === 'running');
  useEffect(() => {
    if (!active) return undefined;
    const timer = setInterval(() => load(), 3000);
    return () => clearInterval(timer);
  }, [active, load]);
  return { runs, error, updated, busy, more: cursor !== null, refresh: () => load(), next: () => load(cursor) };
}
