// stepsBlock.jsx — THREE-LEVEL drill-down ladder for `steps` turns.
// reduce.js folds every non-task tool call into {kind:'steps',
// role:'assistant', steps:[{thinking, calls:[toolCall,…]}]} — the SPA port
// of the TUI's ChatBlock::StepGroup ladder (crates/tui/src/chat_steps.rs).
// Interaction model — USER-CONTROLLED ladder, FOLDED by default (Say stays
// the primary output; the ladder is a drill-down, not the reading surface):
//   L0  ❯ 2 Steps [running|error]  ← TURN Collapse (ghost), CLOSED by
//         default and the ONLY thing a folded steps bubble renders; running
//         from reasoning/tool activity until Say begins, then red on any
//         error. Open state lives in StepsContent's own `stepsOpen` useState
//         (no auto effects, no overrides), so a user-opened ladder survives
//         streaming rerenders.
//         With the turn's own Say: the header becomes the Say row
//           `❯ N Steps: {first-line preview}` and the running tag
//           rides THERE (driven by sayActive, the sayStreaming ladder flag)
//           until the sub-turn's ladder ends.
//   LIVE PREVIEW — while `ladderRunning` and L0 is still folded, the LAST
//         step renders directly below the summary row: `Step(k)` header +
//         its thinking + its call rows (the same ToolContent rows as L3, so
//         the per-call `running` tag/duration/error stream in place). When
//         running retires (Say begins / the turn settles) the preview
//         disappears and the step lives only inside the folded ladder.
//         Opening L0 hides the preview (nothing is shown twice).
//   L1    ❯ Step(1) [error]        ← per-step Collapse (uncontrolled);
//         red tag when any of THIS step's calls failed
//   L2      💭 Thinking (text)      ← thinking renders DIRECTLY (mono grey
//         paragraph + label line) — no ghost collapse inside a step
//          ❯ 1 Function call       ← calls-AGGREGATE Collapse (ghost,
//         uncontrolled)
//   L3        🔧 bash · 1.2s        ← per-call Collapse (input/result)
// Clicking L0 reveals the step rows, clicking a step reveals its thinking
// and calls aggregate, opening that aggregate reveals call rows, and clicking
// one call reveals only that call's result. Ctrl+L / ⤒ 收起 remount the
// bubbles (epoch key in transcript.jsx), which resets the user-open state
// together with the uncontrolled levels below it.
// Only the Say's single-line preview enters this component (inside the
// header label above); the Say BODY never does: reduce.js keeps it as a
// sibling segment and bubbleItems.js places both segments in the same visual
// Turn, transcript.jsx renders the full text below this component.
// ThinkContent stays exported for
// transcript.jsx's independent `think` turns (history/defense only — the
// live path streams reasoning straight into steps); ToolContent is shared
// by the L3 rows, the live preview, and flat `tool` turns (task handles).
// The import is one-way (transcript → stepsBlock), no cycle.

import { useState } from 'react';
import { Collapse, Tag, Typography } from 'antd';
import { fmtDuration } from './format.js';
import { sayPresentation } from './transcript/markdown.js';
import { MONO_VAR } from './ui/mono.js';

const { Text, Paragraph } = Typography;

// One disclosure glyph, changing with the actual state (TUI ▸ / ❯).
// A literal glyph in the label plus antd's default icon produced two arrows.
const disclosureIcon = ({ isActive }) => <span aria-hidden="true">{isActive ? '❯' : '▸'}</span>;

/// Reasoning row for standalone `think` turns (pure-text rounds): ghost
/// collapse, renders even with empty text so a reasoning-only frame still
/// leaves a visible trace (old ThinkTurn feel). NOT used inside steps —
/// step thinking renders directly (see StepCollapse).
export function ThinkContent({ turn }) {
  return (
    <Collapse
      expandIcon={disclosureIcon}
      size="small"
      ghost
      items={[{
        key: 'think',
        label: <span style={{ fontSize: 12 }}>💭 Thinking</span>,
        children: (
          <Paragraph
            style={{
              fontFamily: MONO_VAR, fontSize: 12,
              whiteSpace: 'pre-wrap', color: 'var(--oc-text-tertiary)', marginBottom: 0,
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
/// padding now provides). Used as the L3 row inside the ladder, the flat
/// rows of the live preview, and for flat `tool` turns (task handles) —
/// semantics unchanged. While the call is still running (`output === null`)
/// a small processing `running` tag rides the label (TUI per-call spinner
/// parity); it disappears once the output lands. The collapse stays
/// UNCONTROLLED at every call site: its disclosure is pure local state.
export function ToolContent({ turn }) {
  const dur = fmtDuration(turn.durationMs);
  return (
    <Collapse
      expandIcon={disclosureIcon}
      size="small"
      items={[{
        key: 'tool',
        label: (
          <span style={{ fontFamily: MONO_VAR, fontSize: 12 }}>
            🔧 {turn.name || 'tool'}
            {dur ? <Text type="secondary"> · {dur}</Text> : null}
            {turn.isError ? <Tag color="red" style={{ marginLeft: 8 }}>error</Tag> : null}
            {turn.output === null ? <Tag color="processing" style={{ marginLeft: 8 }}>running</Tag> : null}
          </span>
        ),
        children: (
          <>
            {turn.input ? (
              <Paragraph style={{ fontFamily: MONO_VAR, fontSize: 12, whiteSpace: 'pre-wrap', marginBottom: 4 }}>
                <Text type="secondary">input:</Text>
                {'\n'}
                {turn.input}
              </Paragraph>
            ) : null}
            {turn.output ? (
              <Paragraph style={{ fontFamily: MONO_VAR, fontSize: 12, whiteSpace: 'pre-wrap', marginBottom: 0 }}>
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
      <div style={{ fontFamily: MONO_VAR, fontSize: 12, color: 'var(--oc-text-tertiary)' }}>💭 Thinking</div>
      <Paragraph
        style={{
          fontFamily: MONO_VAR, fontSize: 12,
          whiteSpace: 'pre-wrap', color: 'var(--oc-text-tertiary)', marginBottom: 0,
        }}
      >
        {text}
      </Paragraph>
    </div>
  );
}

/// The call rows shared by the L2 aggregate and the live preview: one
/// uncontrolled ToolContent per call (input/result on drill-down, plus the
/// per-call `running` tag while `output === null`).
function CallRows({ calls }) {
  return calls.map((call, ci) => (
    <ToolContent key={(call && call.id) || 'call:' + ci} turn={call} />
  ));
}

/// The step BODY shared by the L1 row and the live preview: the step's
/// thinking rendered directly, followed by its calls. Inside the ladder the
/// calls sit behind the `N Function call(s)` aggregate (closed until
/// clicked); the preview passes `flatCalls` so the running call row is
/// visible immediately. Every collapse here is uncontrolled.
function StepBody({ step, flatCalls = false }) {
  const list = (step && Array.isArray(step.calls)) ? step.calls : [];
  const thinking = step && typeof step.thinking === 'string' ? step.thinking : '';
  return (
    <>
      {thinking ? <StepThinking text={thinking} /> : null}
      {list.length ? (flatCalls ? (
        <CallRows calls={list} />
      ) : (
        <Collapse
          expandIcon={disclosureIcon}
          size="small"
          ghost
          items={[{
            key: 'calls',
            label: (
              <span style={{ fontFamily: MONO_VAR, fontSize: 12 }}>
                {list.length} Function call{list.length === 1 ? '' : 's'}
              </span>
            ),
            children: <CallRows calls={list} />,
          }]}
        />
      )) : null}
    </>
  );
}

/// One step row (L1 → L2): label `❯ Step(k)` (+ red error tag when any of
/// the step's calls failed), children = the shared StepBody. Uncontrolled:
/// the user opens it and streaming rerenders leave the state alone.
function StepCollapse({ step, index }) {
  const k = index + 1;
  const list = (step && Array.isArray(step.calls)) ? step.calls : [];
  const failed = list.some((c) => c && c.isError);
  return (
    <Collapse
      expandIcon={disclosureIcon}
      size="small"
      ghost
      items={[{
        key: 'step:' + k,
        label: (
          <span style={{ fontFamily: MONO_VAR, fontSize: 12 }}>
            Step({k})
            {failed ? <Tag color="red" style={{ marginLeft: 8 }}>error</Tag> : null}
          </span>
        ),
        children: <StepBody step={step} />,
      }]}
    />
  );
}

/// LIVE preview under the folded L0 summary: the LAST step's header + body
/// (thinking + its call rows laid out flat, so the running call and its
/// `running` tag stream in place). The 12px left inset matches antd's
/// collapse body padding, keeping the preview aligned with the drilled rows;
/// no separators — the preview simply disappears once the ladder settles or
/// the user opens L0.
function StepPreview({ step, index }) {
  const k = index + 1;
  const list = (step && Array.isArray(step.calls)) ? step.calls : [];
  const failed = list.some((c) => c && c.isError);
  return (
    <div style={{ marginLeft: 12, marginBottom: 8 }}>
      <div style={{ fontFamily: MONO_VAR, fontSize: 12, marginBottom: 4 }}>
        Step({k})
        {failed ? <Tag color="red" style={{ marginLeft: 8 }}>error</Tag> : null}
      </div>
      <StepBody step={step} flatCalls />
    </div>
  );
}

/// Say header preview: imported from sayText.js and shared with the
/// body-side first-line dedup (transcript.jsx) — one concatenation basis for
/// both sides, so the header preview and the skipped body line can never
/// drift apart. `image:true` marker rows are presentation, NOT Say text (TUI
/// parity: Image blocks never close a turn) and never feed the preview; an
/// empty/whitespace Say leaves the preview empty (bare colon).

/// Turn row (L0): ONE ghost Collapse for the whole turn — default CLOSED,
/// so a collapsed steps bubble shows only this row plus the live preview
/// while the ladder runs (see header comment). The L0 open state is plain
/// local UI state (`stepsOpen`), NOT an auto-managed policy. Two label modes:
///   * no Say yet: `❯ N Step(s)`; `running` while `progressActive` stays
///     true from reasoning/tool activity until Say begins (older hand-built
///     turns without that field fall back to the open-call test);
///   * the turn's own Say: `❯ N Steps: {first-line preview}` — the
///     running tag MOVES onto the Say row (driven by `sayActive`, the
///     sayStreaming ladder flag from the reducer) so the hint survives the
///     Say until the sub-turn's ladder really ends; `error` still shows once
///     running settles. Clicking the Say row drills the SAME Collapse.
/// Say text: only the first non-blank line (preview) rides in the label —
/// the full Say body renders outside, as this component's sibling (see the
/// header comment).
export function StepsContent({ turn, preview }) {
  const steps = turn && Array.isArray(turn.steps) ? turn.steps : [];
  const calls = steps.flatMap((s) => ((s && Array.isArray(s.calls)) ? s.calls : []));
  const openCall = calls.some((c) => c && c.output === null);
  const say = turn && Array.isArray(turn.say) ? turn.say : [];
  // Same Say test bubbleItems.js uses for its progressActive freeze (image
  // markers count — they ride in the say segment like any non-step part).
  const hasSay = say.some((part) => (
    part && part.kind === 'text' && !part.image && typeof part.text === 'string' && part.text.trim().length > 0
  ));
  const running = hasSay
    ? turn.sayActive === true
    : (typeof turn.progressActive === 'boolean' ? turn.progressActive : openCall);
  // The live preview rides the ladder's OWN pre-Say activity (TUI parity:
  // Say's first chunk settles the ladder even while the running tag rides the
  // Say row), so `sayActive` must NOT keep the preview alive.
  const ladderRunning = !hasSay && (typeof turn.progressActive === 'boolean'
    ? turn.progressActive === true
    : openCall);
  // L0 is the ONLY controlled level and its state is plain local UI state:
  // folded by default, survives streaming rerenders, resets when Ctrl+L /
  // ⤒ 收起 remounts the bubble (epoch key in transcript.jsx). No auto
  // effects, no overrides; L1/L2/L3 stay uncontrolled.
  const [stepsOpen, setStepsOpen] = useState(false);
  const errored = calls.some((c) => c && c.isError);
  const headingPreview = preview ?? sayPresentation(say, turn.sayActive === true).preview;
  return (
    <>
      <Collapse
        expandIcon={disclosureIcon}
        size="small"
        ghost
        activeKey={stepsOpen ? ['steps'] : []}
        onChange={(keys) => setStepsOpen(keys.includes('steps'))}
        items={[{
          key: 'steps',
          label: (
            <span style={{ fontFamily: MONO_VAR, fontSize: 12 }}>
              {`${steps.length} Step${steps.length === 1 ? '' : 's'}${hasSay && headingPreview ? ': ' + headingPreview : ''}`}
              {running ? <Tag color="processing" style={{ marginLeft: 12 }}>running</Tag> : null}
              {!running && errored ? <Tag color="red" style={{ marginLeft: 12 }}>error</Tag> : null}
            </span>
          ),
          children: steps.map((step, i) => (
            <StepCollapse key={'step:' + (i + 1)} step={step} index={i} />
          )),
        }]}
      />
      {ladderRunning && !stepsOpen && steps.length > 0 ? (
        <StepPreview step={steps[steps.length - 1]} index={steps.length - 1} />
      ) : null}
    </>
  );
}
