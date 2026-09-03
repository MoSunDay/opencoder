// stepsBlock.jsx — THREE-LEVEL drill-down ladder for `steps` turns.
// reduce.js folds every non-task tool call into {kind:'steps',
// role:'assistant', steps:[{thinking, calls:[toolCall,…]}]} — the SPA port
// of the TUI's ChatBlock::StepGroup ladder (crates/tui/src/chat_steps.rs).
// Interaction model (collapsed content stays OUT of the DOM until drilled):
//   L0  ❯ 2 Steps [running|error]  ← TURN Collapse (ghost, default CLOSED)
//         — the ONLY thing a collapsed steps bubble renders; running from
//           reasoning/tool activity until Say begins, then red on any error
//   L1    ❯ Step(1) [error]        ← per-step Collapse (default closed);
//         red tag when any of THIS step's calls failed
//   L2      💭 Thinking (text)      ← thinking renders DIRECTLY (mono grey
//         paragraph + label line) — no ghost collapse inside a step
//          ❯ 1 Function call       ← calls-AGGREGATE Collapse (ghost)
//   L3        🔧 bash · 1.2s        ← per-call Collapse (input/result)
// Clicking L0 reveals the step rows, clicking a step reveals its thinking
// and calls aggregate, opening that aggregate reveals call rows, and clicking
// one call reveals only that call's result. Every level starts closed,
// including while streaming. Ctrl+L / ⤒ 收起 remount
// the bubbles (epoch key), resetting them to the stable `N Steps + Say`
// turn summary.
// Say text never enters this component: reduce.js keeps it as a sibling
// segment and bubbleItems.js places both segments in the same visual Turn.
// ThinkContent stays exported for
// transcript.jsx's independent `think` turns (history/defense only — the
// live path streams reasoning straight into steps); ToolContent
// is shared by the L3 rows and flat `tool` turns (task handles). The
// import is one-way (transcript → stepsBlock), no cycle.

import { Collapse, Tag, Typography } from 'antd';
import { fmtDuration } from './format.js';

const { Text, Paragraph } = Typography;

// TUI-flavoured monospace carried over from the old TextTurn/ToolTurn.
const MONO = 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace';

/// Reasoning row for standalone `think` turns (pure-text rounds): ghost
/// collapse, renders even with empty text so a reasoning-only frame still
/// leaves a visible trace (old ThinkTurn feel). NOT used inside steps —
/// step thinking renders directly (see StepCollapse).
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
/// padding now provides). Used both as the L3 row inside the ladder and for
/// flat `tool` turns (task handles) — semantics unchanged.
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

/// Step thinking, L2: a small `💭 Thinking` label line + the text DIRECTLY
/// as a mono grey paragraph — no collapse of its own (drilling into the
/// step already paid the click; burying the text one level deeper would
/// hide the round's reasoning behind a fourth click).
function StepThinking({ text }) {
  return (
    <div style={{ marginBottom: 4 }}>
      <div style={{ fontFamily: MONO, fontSize: 12, color: '#8c8c8c' }}>💭 Thinking</div>
      <Paragraph
        style={{
          fontFamily: MONO, fontSize: 12,
          whiteSpace: 'pre-wrap', color: '#8c8c8c', marginBottom: 0,
        }}
      >
        {text}
      </Paragraph>
    </div>
  );
}

/// One step row (L1 → L2): label `❯ Step(k)` (+ red error tag when any of
/// the step's calls failed). Children: the step's thinking rendered
/// directly, followed by a calls aggregation whose children are the
/// individually collapsible ToolContent rows.
function StepCollapse({ step, index }) {
  const k = index + 1;
  const list = (step && Array.isArray(step.calls)) ? step.calls : [];
  const failed = list.some((c) => c && c.isError);
  const thinking = step && typeof step.thinking === 'string' ? step.thinking : '';
  return (
    <Collapse
      size="small"
      ghost
      items={[{
        key: 'step:' + k,
        label: (
          <span style={{ fontFamily: MONO, fontSize: 12 }}>
            ❯ Step({k})
            {failed ? <Tag color="red" style={{ marginLeft: 8 }}>error</Tag> : null}
          </span>
        ),
        children: (
          <>
            {thinking ? <StepThinking text={thinking} /> : null}
            {list.length ? (
              <Collapse
                size="small"
                ghost
                items={[{
                  key: 'calls:' + k,
                  label: (
                    <span style={{ fontFamily: MONO, fontSize: 12 }}>
                      ❯ {list.length} Function call{list.length === 1 ? '' : 's'}
                    </span>
                  ),
                  children: list.map((call, ci) => (
                    <ToolContent key={(call && call.id) || 'call:' + ci} turn={call} />
                  )),
                }]}
              />
            ) : null}
          </>
        ),
      }]}
    />
  );
}

/// Turn row (L0): ONE ghost Collapse for the whole turn — label `❯ N
/// Step(s)` + running/error tag; default CLOSED, so a collapsed steps bubble
/// shows only this row (the ladder renders on drill-down). `running` while
/// `progressActive` stays true from reasoning/tool activity until Say begins;
/// `error` appears only once progress settles. Older hand-built turns without
/// that field fall back to the open-call test. It remains closed by default
/// while streaming, so disclosure state never jumps as frames arrive.
export function StepsContent({ turn }) {
  const steps = turn && Array.isArray(turn.steps) ? turn.steps : [];
  const calls = steps.flatMap((s) => ((s && Array.isArray(s.calls)) ? s.calls : []));
  const openCall = calls.some((c) => c && c.output === null);
  const running = typeof turn.progressActive === 'boolean'
    ? turn.progressActive
    : openCall;
  const errored = calls.some((c) => c && c.isError);
  return (
    <Collapse
      size="small"
      ghost
      items={[{
        key: 'steps',
        label: (
          <span style={{ fontFamily: MONO, fontSize: 12 }}>
            ❯ {steps.length} Step{steps.length === 1 ? '' : 's'}
            {running ? <Tag color="processing" style={{ marginLeft: 12 }}>running</Tag> : null}
            {!running && errored ? <Tag color="red" style={{ marginLeft: 12 }}>error</Tag> : null}
          </span>
        ),
        children: steps.map((step, i) => (
          <StepCollapse key={'step:' + (i + 1)} step={step} index={i} />
        )),
      }]}
    />
  );
}
