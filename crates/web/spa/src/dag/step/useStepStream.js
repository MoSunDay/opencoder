// Live subscription to one DAG step's node-side event stream. The frame
// window (appendLog) and the TUI transcript fold (reduceExecutionFrame) are
// updated in the SAME onFrame so the wasm log view and the agent transcript
// can never drift apart; agent-session frames flow straight into the shared
// execution reducer, and the unknown kinds (step_output/step_finished) hit
// its default branch and pass through untouched.
import { useCallback, useEffect, useRef, useState } from 'react';
import { openStream } from '../../sse.js';
import { appendLog, initialLogs } from '../../ui/executionEvents/model.js';
import { initialExecutionTranscript, reduceExecutionFrame } from '../../fleet/detail/transcript.js';

export function useStepStream({ runId, step, index, enabled = true }) {
  const [connection, setConnection] = useState('connecting');
  const [error, setError] = useState('');
  const [logs, setLogs] = useState(initialLogs);
  const [transcript, setTranscript] = useState(initialExecutionTranscript);
  const [finished, setFinished] = useState(null);
  const state = useRef(initialLogs());
  const cursorRef = useRef(0);
  const [retryNonce, setRetryNonce] = useState(0);

  useEffect(() => {
    if (!runId || !step || !enabled) return undefined;
    const controller = new AbortController();
    // Reset the whole window on run/step change and on retry: the stream
    // replays from seq 0, so a stale fold would double-apply history.
    state.current = initialLogs();
    cursorRef.current = 0;
    setLogs(state.current);
    setTranscript(initialExecutionTranscript());
    setFinished(null);
    setError('');
    setConnection('connecting');
    const stream = openStream({
      path: '/api/dag/runs/' + encodeURIComponent(runId) + '/steps/' + encodeURIComponent(step) + (index === undefined ? '' : '/instances/' + index) + '/events',
      after: cursorRef.current,
      executionHistory: true,
      requireEnd: true,
      onResync: async () => cursorRef.current,
      onStatus: (next) => {
        // Keep a terminal 'failed' (max attempts reached) — the transport's
        // final 'closed' report must not mask it (useExecutionEvents parity).
        if (!controller.signal.aborted) setConnection((old) => (old === 'failed' && next === 'closed' ? old : next));
      },
      onFrame: (frame) => {
        if (controller.signal.aborted) return;
        const next = appendLog(state.current, frame);
        if (next === state.current) return; // seq'd replay duplicate
        state.current = next;
        cursorRef.current = next.cursor;
        setLogs(next);
        setTranscript((old) => reduceExecutionFrame(old, frame, Date.now()));
        if (frame.event === 'step_finished') setFinished(frame.data || null);
      },
    });
    return () => { controller.abort(); stream?.abort(); };
  }, [runId, step, index, enabled, retryNonce]);

  const retry = useCallback(() => setRetryNonce((value) => value + 1), []);
  return { frames: logs.frames, cursor: logs.cursor, trimmed: logs.trimmed, transcript, finished, connection, error, retry };
}
