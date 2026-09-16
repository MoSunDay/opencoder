// One step's live record body: picks the agent transcript or the wasm log
// view by step kind, surfaces stream failures with a reconnect action and
// reports the terminal `step_finished` receipt upward (the drawer refetches
// the step receipt on it).
import { Alert, Button } from 'antd';
import { useEffect, useRef } from 'react';
import { isAgentKind } from './model.js';
import { useStepStream } from './useStepStream.js';
import { WasmLogs } from './wasmLogs.jsx';
import { AgentTranscript } from './agentTranscript.jsx';

export function StepPanel({ runId, step, kind, onFinished }) {
  const stream = useStepStream({ runId, step });
  const notify = useRef(onFinished);
  notify.current = onFinished;
  const finished = stream.finished;
  useEffect(() => {
    if (finished) notify.current?.(finished);
  }, [finished]);
  return <>
    {(stream.error || stream.connection === 'failed') && (
      <Alert type="error" title={stream.error || '步骤记录连接失败，请重新连接'}
        action={<Button size="small" onClick={stream.retry}>重新连接</Button>} />
    )}
    {isAgentKind(kind)
      ? <AgentTranscript transcript={stream.transcript} finished={stream.finished} connection={stream.connection} />
      : <WasmLogs frames={stream.frames} trimmed={stream.trimmed} connection={stream.connection} />}
  </>;
}
