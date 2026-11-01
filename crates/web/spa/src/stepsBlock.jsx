// stepsBlock.jsx — THREE-LEVEL drill-down ladder for `steps` turns.
// reduce.js folds every non-task tool call into {kind:'steps',
// role:'assistant', steps:[{thinking, calls:[toolCall,…]}]} — the SPA port
// of the TUI's ChatBlock::StepGroup ladder (crates/tui/src/chat_steps.rs).
// Interaction model (collapsed content stays OUT of the DOM until drilled):
//   L0  ❯ 2 Steps [running|error]  ← TURN Collapse (ghost, auto/user
//         managed — see LIVE disclosure below); the ONLY thing a collapsed
//         steps bubble renders; running from
//           reasoning/tool activity until Say begins, then red on any error.
//         With the turn's own Say: the header becomes the Say row
//           `❯ N Steps: {first-line preview}` and the running tag
//           rides THERE (driven by sayActive, the sayStreaming ladder flag)
//           until the sub-turn's ladder ends.
//   L1    ❯ Step(1) [error]        ← per-step Collapse (auto/user managed);
//         red tag when any of THIS step's calls failed
//   L2      💭 Thinking (text)      ← thinking renders DIRECTLY (mono grey
//         paragraph + label line) — no ghost collapse inside a step
//          ❯ 1 Function call       ← calls-AGGREGATE Collapse (ghost)
//   L3        🔧 bash · 1.2s        ← per-call Collapse (input/result)
// Clicking L0 reveals the step rows, clicking a step reveals its thinking
// and calls aggregate, opening that aggregate reveals call rows, and clicking
// one call reveals only that call's result. LIVE disclosure
// (steps/useLadderDisclosure.js — TUI chat_step_live.rs parity): while the
// turn runs the ladder AUTO-OPENS at the running position (L0 + the LAST
// step + its calls aggregate); when running retires (Say begins / the turn
// settles) every auto level collapses back to the `N Steps` summary row,
// and a step born while still running moves the auto slot so the finished
// step above folds by itself. USER PRIORITY: clicking any level records an
// open/closed override that wins over the auto state for the component's
// lifetime (a user-closed level never reopens from streaming; a user-opened
// old step stays open when the auto slot moves on). Per-call rows (L3) stay
// uncontrolled, but expanding one PINS its enclosing `step:N` + `calls:N`
// open via overrides (TUI Call-row `user_touched` parity). Ctrl+L / ⤒ 收起
// remount the bubbles (epoch key), so the
// overrides reset; if the turn is still running the auto-open re-applies
// afterwards (accepted divergence from the TUI's per-turn `user_touched`
// pinning).
// Only the Say's single-line preview enters this component (inside the
// header label above); the Say BODY never does: reduce.js keeps it as a
// sibling segment and bubbleItems.js places both segments in the same visual
// Turn, transcript.jsx renders the full text below this component.
// ThinkContent stays exported for
// transcript.jsx's independent `think` turns (history/defense only — the
// live path streams reasoning straight into steps); ToolContent
// is shared by the L3 rows and flat `tool` turns (task handles). The
// import is one-way (transcript → stepsBlock), no cycle.

import { Collapse, Tag, Typography } from 'antd';
import { fmtDuration } from './format.js';
import { sayPresentation } from './transcript/markdown.js';
import { useLadderDisclosure } from './steps/useLadderDisclosure.js';
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
/// padding now provides). Used both as the L3 row inside the ladder and for
/// flat `tool` turns (task handles) — semantics unchanged. While the call
/// is still running (`output === null`) a small processing `running` tag
/// rides the label (TUI per-call spinner parity); it disappears once the
/// output lands. The collapse stays UNCONTROLLED; an optional `onUserExpand`
/// callback is merely NOTIFIED when the row opens — only the ladder L3 site
/// passes it (to pin the enclosing levels), flat `tool` turns never do.
export function ToolContent({ turn, onUserExpand }) {
  const dur = fmtDuration(turn.durationMs);
  return (
    <Collapse
      expandIcon={disclosureIcon}
      size="small"
      onChange={onUserExpand
        ? (keys) => { if (keys.includes('tool')) onUserExpand(); }
        : undefined}
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

/// One step row (L1 → L2): label `❯ Step(k)` (+ red error tag when any of
/// the step's calls failed). Children: the step's thinking rendered
/// directly, followed by a calls aggregation whose children are the
/// individually collapsible ToolContent rows. Both the step and its calls
/// aggregate are CONTROLLED through the shared disclosure controller
/// (effective = user override ?? auto running position); only the call rows
/// below stay uncontrolled — but expanding one notifies this component,
/// which pins the enclosing `step:N` + `calls:N` open so the settle can
/// never hide the drilled call.
function StepCollapse({ step, index, ladder }) {
  const k = index + 1;
  const stepKey = 'step:' + k;
  const callsKey = 'calls:' + k;
  const list = (step && Array.isArray(step.calls)) ? step.calls : [];
  const failed = list.some((c) => c && c.isError);
  const thinking = step && typeof step.thinking === 'string' ? step.thinking : '';
  return (
    <Collapse
      expandIcon={disclosureIcon}
      size="small"
      ghost
      activeKey={ladder.isOpen(stepKey) ? [stepKey] : []}
      onChange={(keys) => ladder.setOpen(stepKey, keys.includes(stepKey))}
      items={[{
        key: stepKey,
        label: (
          <span style={{ fontFamily: MONO_VAR, fontSize: 12 }}>
            Step({k})
            {failed ? <Tag color="red" style={{ marginLeft: 8 }}>error</Tag> : null}
          </span>
        ),
        children: (
          <>
            {thinking ? <StepThinking text={thinking} /> : null}
            {list.length ? (
              <Collapse
                expandIcon={disclosureIcon}
                size="small"
                ghost
                activeKey={ladder.isOpen(callsKey) ? [callsKey] : []}
                onChange={(keys) => ladder.setOpen(callsKey, keys.includes(callsKey))}
                items={[{
                  key: callsKey,
                  label: (
                    <span style={{ fontFamily: MONO_VAR, fontSize: 12 }}>
                      {list.length} Function call{list.length === 1 ? '' : 's'}
                    </span>
                  ),
                  children: list.map((call, ci) => (
                    <ToolContent
                      key={(call && call.id) || 'call:' + ci}
                      turn={call}
                      onUserExpand={() => {
                        // A drilled call result pins its enclosing ladder
                        // levels open (TUI Call-row `user_touched` parity):
                        // the settle empties the auto keys, but these
                        // overrides keep the step + calls aggregate up.
                        ladder.setOpen(stepKey, true);
                        ladder.setOpen(callsKey, true);
                      }}
                    />
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

/// Say header preview: imported from sayText.js and shared with the
/// body-side first-line dedup (transcript.jsx) — one concatenation basis for
/// both sides, so the header preview and the skipped body line can never
/// drift apart. `image:true` marker rows are presentation, NOT Say text (TUI
/// parity: Image blocks never close a turn) and never feed the preview; an
/// empty/whitespace Say leaves the preview empty (bare colon).

/// Turn row (L0): ONE ghost Collapse for the whole turn — default CLOSED, so
/// a collapsed steps bubble shows only this row (the ladder renders on
/// drill-down). Two label modes:
///   * no Say yet: `❯ N Step(s)`; `running` while `progressActive` stays
///     true from reasoning/tool activity until Say begins (older hand-built
///     turns without that field fall back to the open-call test);
///   * the turn's own Say: `❯ N Steps: {first-line preview}` — the
///     running tag MOVES onto the Say row (driven by `sayActive`, the
///     sayStreaming ladder flag from the reducer) so the hint survives the
///     Say until the sub-turn's ladder really ends; `error` still shows once
///     running settles. Clicking the Say row still drills the SAME Collapse,
///     now controlled through the ladder controller (user override wins).
/// Say text: only the first non-blank line (preview) rides in the label —
/// the full Say body renders outside, as this component's sibling (see the
/// header comment). Disclosure follows the live policy in the header
/// comment: auto-open at the running position while `running`, auto-collapse
/// on settle, user overrides win for the component's lifetime.
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
  // The ONLY disclosure brain: auto keys + user overrides. Disclosure keys
  // off the ladder's OWN pre-Say activity (TUI parity: Say's first chunk
  // settles the ladder even while the running tag rides the Say row), so
  // `sayActive` must NOT reopen it.
  const ladderRunning = !hasSay && (typeof turn.progressActive === 'boolean'
    ? turn.progressActive === true
    : openCall);
  const ladder = useLadderDisclosure(ladderRunning, steps.length);
  const errored = calls.some((c) => c && c.isError);
  const headingPreview = preview ?? sayPresentation(say, turn.sayActive === true).preview;
  return (
    <Collapse
      expandIcon={disclosureIcon}
      size="small"
      ghost
      activeKey={ladder.isOpen('steps') ? ['steps'] : []}
      onChange={(keys) => ladder.setOpen('steps', keys.includes('steps'))}
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
          <StepCollapse key={'step:' + (i + 1)} step={step} index={i} ladder={ladder} />
        )),
      }]}
    />
  );
}
