import { useCallback, useEffect, useRef, useState } from 'react';
import { apiGet } from '../../api.js';

export function useComments(todoId, active) {
  const path = `/api/project/todos/${encodeURIComponent(todoId)}/comments`;
  const [rows, setRows] = useState([]);
  const [loading, setLoading] = useState(false);
  const [older, setOlder] = useState(false);
  const [error, setError] = useState('');
  const [revision, setRevision] = useState(0);
  const [added, setAdded] = useState(0);
  const records = useRef([]);
  const mutation = useRef(0);
  const polling = useRef(false);
  const requests = useRef(new Set());
  const generation = useRef(0);
  const merge = useCallback((items) => {
    mutation.current += 1;
    const combined = new Map(records.current.map((row) => [row.seq, row]));
    items.forEach((row) => combined.set(row.seq, row));
    records.current = [...combined.values()].sort((a, b) => a.seq - b.seq);
    setRows(records.current);
  }, []);
  const remove = useCallback((seq) => {
    mutation.current += 1;
    records.current = records.current.filter((row) => row.seq !== seq);
    setRows(records.current);
  }, []);
  useEffect(() => { records.current = []; setRows([]); setOlder(false); setAdded(0); }, [path]);
  const read = useCallback(async (query = '') => {
    const request = new AbortController(); requests.current.add(request);
    try {
      const page = await apiGet(path + query, { signal: request.signal });
      if (request.signal.aborted) return null;
      if (!Array.isArray(page.comments) || typeof page.has_more !== 'boolean'
        || page.comments.some((row) => !Number.isSafeInteger(row.seq) || typeof row.body_md !== 'string')) throw new Error('评论列表格式错误');
      setError(''); return page;
    } catch (failure) { if (!request.signal.aborted) setError(failure.message); return null; }
    finally { requests.current.delete(request); }
  }, [path]);
  const loadOlder = async () => {
    if (polling.current || !records.current.length) return;
    polling.current = true; setLoading(true);
    const epoch = generation.current;
    try {
      const page = await read(`?before_seq=${records.current[0].seq}`);
      if (page && epoch === generation.current) { merge(page.comments); setOlder(page.has_more); }
    } finally { if (epoch === generation.current) { polling.current = false; setLoading(false); } }
  };
  useEffect(() => {
    if (!active) return;
    const epoch = ++generation.current;
    let disposed = false;
    let timer;
    const poll = async () => {
      if (polling.current) { timer = setTimeout(poll, 5000); return; }
      polling.current = true;
      if (!records.current.length) setLoading(true);
      const version = mutation.current;
      const first = records.current[0]?.seq;
      const last = records.current.at(-1)?.seq;
      try {
        let page = await read();
        if (!page || disposed) return;
        let snapshot = page.comments;
        while (page.has_more && first && snapshot[0]?.seq > first && !disposed) {
          const before = snapshot[0].seq;
          page = await read(`?before_seq=${before}`);
          if (!page || disposed) return;
          if (!page.comments.length || page.comments[0].seq >= before) throw new Error('评论分页未前进');
          snapshot = [...page.comments, ...snapshot];
        }
        if (!disposed && version === mutation.current) {
          const count = last ? snapshot.filter((row) => row.seq > last).length : 0;
          records.current = snapshot; setRows(snapshot); setOlder(page.has_more);
          if (count) setAdded((old) => old + count);
        }
      } catch (failure) { if (!disposed) setError(failure.message); }
      finally {
        if (epoch === generation.current) polling.current = false;
        if (!disposed) { setLoading(false); timer = setTimeout(poll, 5000); }
      }
    };
    poll();
    return () => { disposed = true; generation.current += 1; polling.current = false; clearTimeout(timer); requests.current.forEach((request) => request.abort()); };
  }, [active, revision, read, merge]);
  return { rows, loading, older, error, added, loadOlder, merge, remove, reload: () => setRevision((v) => v + 1) };
}
