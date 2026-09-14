import { useCallback, useEffect, useRef, useState } from 'react';
import { openStream } from '../../sse.js';

export function useExecutionEvents({ id, status, onFrame }) {
  const [connection, setConnection] = useState('connecting');
  const [frames, setFrames] = useState([]);
  const cursor = useRef(0);
  const previousId = useRef(id);
  const [retryNonce, setRetryNonce] = useState(0);
  useEffect(() => {
    if (!id) return undefined;
    if (previousId.current !== id) {
      cursor.current = 0;
      setFrames([]);
      previousId.current = id;
    } else if (!cursor.current) setFrames([]);
    setConnection('connecting');
    const stream = openStream({
      path: `/api/executions/${encodeURIComponent(id)}/events`,
      after: cursor.current, executionHistory: true, requireEnd: true,
      onStatus: (next) => setConnection((current) => current === 'failed' && next === 'closed' ? current : next),
      onFrame: (frame) => {
        if (Number.isFinite(frame.seq)) cursor.current = Math.max(cursor.current, frame.seq);
        setFrames((v) => v.some((item) => item.seq === frame.seq)
          ? v
          : [...v.slice(-999), frame]);
        onFrame?.(frame);
      },
    });
    return () => stream.abort();
  }, [id, status, retryNonce]);
  const retry = useCallback(() => setRetryNonce((value) => value + 1), []);
  return { connection, frames, cursor: cursor.current, status, retry };
}
