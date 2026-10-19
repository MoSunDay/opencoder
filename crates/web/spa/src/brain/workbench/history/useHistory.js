import { useCallback, useEffect, useRef, useState } from 'react';
import { apiGet } from '../../../api.js';

export function useHistory() {
  const [state, setState] = useState({ rows: [], page: 0, next: null, loading: true, error: '' });
  const cursors = useRef([null]);
  const request = useRef(null);
  const page = useRef(0);
  const load = useCallback(async (target = page.current) => {
    request.current?.abort();
    const controller = new AbortController(); request.current = controller;
    setState((old) => ({ ...old, loading: true, error: '' }));
    const query = new URLSearchParams({ limit: '20' });
    const cursor = cursors.current[target];
    if (cursor) { query.set('cursor_created_at', cursor.created_at); query.set('cursor_id', cursor.id); }
    try {
      const result = await apiGet(`/api/brain/runs?${query}`, { signal: controller.signal });
      if (controller.signal.aborted) return;
      if (!Array.isArray(result.runs)) throw new Error('运行历史响应格式错误');
      page.current = target;
      cursors.current = cursors.current.slice(0, target + 1).concat(result.next_cursor || null);
      setState({ rows: result.runs, page: target, next: result.next_cursor, loading: false, error: '' });
    } catch (error) {
      if (!controller.signal.aborted) setState((old) => ({ ...old, loading: false, error: error.message }));
    }
  }, []);
  useEffect(() => { load(0); return () => request.current?.abort(); }, [load]);
  return { ...state, load };
}
