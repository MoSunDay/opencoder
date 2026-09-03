// stepsBlock.jsx — step-ladder rendering for `steps` turns. reduce.js folds
// every non-task tool call into {kind:'steps', role:'assistant',
// steps:[{thinking, calls:[toolCall,…]}]} — the SPA port of the TUI's
// ChatBlock::StepGroup ladder (crates/tui/src/chat_steps.rs). Visual shape:
//   ≡ 2 steps  [running]     ← static marker line (NOT a collapse)
//     ❯ Step(1) · 2 calls    ← per-step Collapse, default closed
//         💭 Thinking        ← the step's thinking (only inside an open step)
//         🔧 bash · 1.2s     ← per-call Collapse rows (input/output inside)
// There is NO group-level Collapse — every step row renders immediately, so
// the ladder's height (and the running/error state) is always visible.
// ThinkContent / ToolContent moved here VERBATIM from transcript.jsx; the
// import is one-way (transcript → stepsBlock), no cycle.

import { Collapse, Tag, Typography } from 'antd';
import { fmtDuration } from './format.js';

const { Text, Paragraph } = Typography;

// TUI-flavoured monospace carried over from the old TextTurn/ToolTurn.
const MONO = 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace';

/// Reasoning row: ghost collapse, renders even with empty text so a
/// reasoning-only frame still leaves a visible trace (old ThinkTurn feel).
export function ThinkContent({ turn }) {
  return (
    <Collapse
      size="small"
      ghost
      items={[{
        key: 'think',
        label: <span style={{ fontSize: 12 }}>💭 Thinking</span>,
        children: (
          <Paragraph
            style={{
              fontFamily: MONO, fontSize: 12,
              whiteSpace: 'pre-wrap', color: '#8c8c8c', marginBottom: 0,
            }}
          >
            {turn.text || ''}
          </Paragraph>
        ),
      }]}
    />
  );
}

/// Tool row: 🔧 name · duration, red error tag, input/output paragraphs
/// inside the collapse (old ToolTurn, minus the outer margins the bubble
/// padding now provides).
export function ToolContent({ turn }) {
  const dur = fmtDuration(turn.durationMs);
  return (
    <Collapse
      size="small"
      items={[{
        key: 'tool',
        label: (
          <span style={{ fontFamily: MONO, fontSize: 12 }}>
            🔧 {turn.name || 'tool'}
            {dur ? <Text type="secondary"> · {dur}</Text> : null}
            {turn.isError ? <Tag color="red" style={{ marginLeft: 8 }}>error</Tag> : null}
          </span>
        ),
        children: (
          <>
            {turn.input ? (
              <Paragraph style={{ fontFamily: MONO, fontSize: 12, whiteSpace: 'pre-wrap', marginBottom: 4 }}>
                <Text type="secondary">input:</Text>
                {'\n'}
                {turn.input}
              </Paragraph>
            ) : null}
            {turn.output ? (
              <Paragraph style={{ fontFamily: MONO, fontSize: 12, whiteSpace: 'pre-wrap', marginBottom: 0 }}>
                <Text type="secondary">output:</Text>
                {'\n'}
                {turn.output}
              </Paragraph>
            ) : null}
          </>
        ),
      }]}
    />
  );
}

/// Step ladder: marker row (plain div, never clickable) + one small ghost
/// Collapse per step. `running` while any call is open (`output === null`);
/// `error` on the marker only once nothing is running (a finished round can
/// still fail). antd's own collapse arrow communicates open state, the label
/// itself stays a static `❯ Step(k)`.
export function StepsContent({ turn }) {
  const steps = turn && Array.isArray(turn.steps) ? turn.steps : [];
  const calls = steps.flatMap((s) => ((s && Array.isArray(s.calls)) ? s.calls : []));
  const running = calls.some((c) => c && c.output === null);
  const errored = calls.some((c) => c && c.isError);
  return (
    <div>
      <div style={{ fontFamily: MONO, fontSize: 12, display: 'flex', alignItems: 'center', gap: 8 }}>
        <span>≡ {steps.length} step{steps.length === 1 ? '' : 's'}</span>
        {running ? <Tag color="processing">running</Tag> : null}
        {!running && errored ? <Tag color="red">error</Tag> : null}
      </div>
      {steps.map((step, k) => {
        const list = (step && Array.isArray(step.calls)) ? step.calls : [];
        const n = list.length;
        return (
          <Collapse
            key={'step:' + k}
            size="small"
            ghost
            items={[{
              key: 'step:' + k,
              label: (
                <span style={{ fontFamily: MONO, fontSize: 12 }}>
                  ❯ Step({k + 1})
                  <Text type="secondary"> · {n} call{n === 1 ? '' : 's'}</Text>
                  {list.some((c) => c && c.isError)
                    ? <Tag color="red" style={{ marginLeft: 8 }}>error</Tag>
                    : null}
                </span>
              ),
              children: (
                <>
                  {step && step.thinking ? <ThinkContent turn={{ text: step.thinking }} /> : null}
                  {list.map((call, ci) => (
                    <ToolContent key={(call && call.id) || 'call:' + ci} turn={call} />
                  ))}
                </>
              ),
            }]}
          />
        );
      })}
    </div>
  );
}
