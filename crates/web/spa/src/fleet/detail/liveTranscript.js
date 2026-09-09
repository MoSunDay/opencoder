import { useEffect, useRef, useState } from 'react';
import { apiGet } from '../../api.js';
import { openStream } from '../../sse.js';
import { initialExecutionTranscript, reduceExecutionFrame } from './transcript.js';

export function useExecutionTranscript({ id, enabled, status, revision, onFrame, onSettled, onError }) {
  const [state, setState] = useState(initialExecutionTranscript);
  const [caughtUp, setCaughtUp] = useState(false);
  const cursor = useRef(0);
  const callbacks = useRef({ onFrame, onSettled, onError });
  callbacks.current = { onFrame, onSettled, onError };
  useEffect(() => {
    cursor.current = 0;
    setState(initialExecutionTranscript());
    setCaughtUp(false);
  }, [id]);
  useEffect(() => {
    if (!id || !enabled) return undefined;
    let cancelled = false;
    let stream;
    (async () => {
      try {
        const { head_seq: seq } = await apiGet(`/api/executions/${encodeURIComponent(id)}/events-page?after=9223372036854775807`);
        if (cancelled) return;
        if (!Number.isSafeInteger(seq) || seq < 0) throw new Error('节点未返回有效的事件回放位置');
        setCaughtUp(cursor.current >= seq);
        stream = openStream({
          path: `/api/executions/${encodeURIComponent(id)}/events`,
          after: cursor.current,
          executionHistory: true,
          onResync: async () => cursor.current,
          onFrame: (frame) => {
            if (cancelled) return;
            cursor.current = Math.max(cursor.current, frame.seq || 0);
            setState((old) => reduceExecutionFrame(old, frame, Date.now()));
            if (cursor.current >= seq) setCaughtUp(true);
            callbacks.current.onFrame?.(frame);
            if (cursor.current >= seq && ['done', 'error', 'transcript_reset'].includes(frame.event)) callbacks.current.onSettled?.();
          },
          onStatus: (value) => {
            if (cancelled) return;
            if (value === 'closed') callbacks.current.onSettled?.();
            if (value === 'failed') callbacks.current.onError?.('节点事件流连接失败，请刷新重试');
          },
        });
      } catch (error) {
        if (!cancelled) callbacks.current.onError?.(error.message);
      }
    })();
    return () => { cancelled = true; stream?.abort(); };
  }, [id, enabled, status, revision]);
  return { state, caughtUp };
}
