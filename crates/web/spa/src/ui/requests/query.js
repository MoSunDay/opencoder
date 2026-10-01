import { useCallback, useEffect, useRef, useState } from 'react';
import { apiGet } from '../../api.js';

export function useJsonQuery(path, decode) {
  const request = useRef(null);
  const active = useRef(false);
  const [state, setState] = useState({ data: null, loading: true, error: '' });
  const reload = useCallback(async () => {
    request.current?.abort();
    const controller = new AbortController();
    request.current = controller;
    setState((previous) => ({ ...previous, loading: true, error: '' }));
    try {
      const data = decode(await apiGet(path, { signal: controller.signal }));
      if (active.current && !controller.signal.aborted) setState({ data, loading: false, error: '' });
    } catch (failure) {
      if (active.current && !controller.signal.aborted) setState((previous) => ({ ...previous, loading: false, error: failure.message }));
    }
  }, [path, decode]);
  useEffect(() => {
    active.current = true;
    setState({ data: null, loading: true, error: '' });
    reload();
    return () => { active.current = false; request.current?.abort(); };
  }, [reload]);
  return { ...state, reload };
}
