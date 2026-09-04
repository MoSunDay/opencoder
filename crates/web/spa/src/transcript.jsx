// transcript.jsx — chat transcript on @ant-design/x Bubble.List (T3
// migration of render.jsx). Reduced assistant segments that contain a step
// ladder become one visual Turn bubble:
//   user   → placement end,   variant filled   (❯ avatar, monospace body)
//   ai     → placement start, variant outlined (◉ avatar, monospace body)
//   think  → placement start, variant borderless, ghost 💭 Thinking collapse
//            (standalone turns — pure-text rounds; a tool round's thinking
//            lives INSIDE its step, see below)
//   assistantTurn → placement start, outlined Turn containing collapsed
//            `❯ N Steps [running|error]` + visible Say. Opening the Turn
//            reveals Step rows; opening a Step reveals Thinking + an
//            N-function-calls aggregate; opening it reveals calls, and an
//            individual call reveals its result.
//   tool   → placement start, variant borderless, 🔧 collapse with
//            duration + error tag + input/output paragraphs (flat rows now
//            only for `task` — the subagent handle; renderer lives in
//            stepsBlock.jsx alongside the ladder)
//   sys    → placement start, variant borderless, centered secondary text
//   subagent → placement start, variant borderless, 🤖 fold block with
//            status tag + child replay drill-in (subagentBlock.jsx)
// Assistant Say stays visible at the Turn level — never folded into Steps.
// Collapse-all: Ctrl/Cmd+L (window keydown) or the `⤒ 收起` link bumps an
// epoch key on Bubble.List, remounting every bubble so all Collapses (step
// rows, call rows, subagent blocks) reset closed.
// UsageFooter / StatusTag (moved from render.jsx verbatim in spirit) stay
// below the list; the empty-state hint keeps the old wording contract.

import { useEffect, useState } from 'react';
import { Bubble } from '@ant-design/x';
import { Tag, Typography } from 'antd';
import { isEmptyTranscript, itemsFromTurns, usageLine } from './bubbleItems.js';
import { StepsContent, ThinkContent, ToolContent } from './stepsBlock.jsx';
import { sayBodyParts, sayPreview } from './sayText.js';
import { SubagentContent } from './subagentBlock.jsx';

const { Text, Paragraph } = Typography;

// TUI-flavoured monospace carried over from the old TextTurn/ToolTurn.
const MONO = 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace';

function RoleAvatar({ glyph, color }) {
  return (
    <div style={{
      width: 28,
      height: 28,
      borderRadius: '50%',
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'center',
      background: color + '1a',
      color,
      fontFamily: MONO,
      fontSize: 13,
      fontWeight: 600,
      flexShrink: 0,
      userSelect: 'none',
    }}
    >
      {glyph}
    </div>
  );
}

/// user / ai body: same monospace pre-wrap paragraph the old TextTurn used
/// (the ❯/◉ role markers now live on the bubble avatars).
function TextContent({ turn }) {
  return (
    <Paragraph
      style={{
        fontFamily: MONO,
        fontSize: 13,
        whiteSpace: 'pre-wrap',
        wordBreak: 'break-word',
        marginBottom: 0,
      }}
    >
      {turn.text || ''}
    </Paragraph>
  );
}

/// One visual assistant Turn. `itemsFromTurns` merges all adjacent step
/// segments into one ladder and keeps the closing Say as visible speech —
/// the run ends at its Say, so a ladder streamed after it renders as its own
/// Turn. The resulting top-level shape is always one `N Steps` summary plus
/// Say. The WHOLE content (including the Say segment, sayActive and
/// progressActive) travels into StepsContent: the Say header reads its
/// single-line preview and the Say-row running tag from it, while the Say
/// BODY still renders below as sibling nodes.
function AssistantTurnContent({ turn }) {
  const say = turn && Array.isArray(turn.say) ? turn.say : [];
  // 头部标签已经渲染了正文首个非空行（preview）：正文跳过该行，单行 Say
  // 不再与 preview 一字不差地重复；去完为空则整个正文块不渲染 —— 不残留
  // 任何间距或空节点。口径见 sayText.js（与头部共用同一份拼接）。
  const body = sayBodyParts(say, sayPreview(say));
  return (
    <div>
      <StepsContent turn={turn} />
      {body.length > 0 ? (
        // TUI 对齐（头部行后插一空行）：正文块与头部保持 16px 的真实块级
        // 间距，不再与 `❯ Say(N steps)` 行挤在一起。
        <div style={{ marginTop: 16 }}>
          {body.map((part, index) => {
            if (part.kind === 'think') {
              return <ThinkContent key={'think:' + index} turn={part} />;
            }
            if (part.kind === 'sys') {
              // Absorbed retry/status row (bubbleItems): render as the centered
              // sys line inside the bubble tail, not as an AI text part.
              return <SysContent key={'sys:' + index} turn={part} />;
            }
            return (
              <div key={'say:' + index} style={index === 0 ? undefined : { marginTop: 8 }}>
                <TextContent turn={part} />
              </div>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}

/// Reasoning and tool rows live in stepsBlock.jsx (ThinkContent /
/// ToolContent, moved there verbatim when the step ladder landed) — the
/// think/tool/steps bubbles below all render through them.

/// System status lines: centered, secondary, small.
function SysContent({ turn }) {
  return (
    <div style={{ textAlign: 'center', width: '100%' }}>
      <Text type="secondary" style={{ fontSize: 12 }}>{turn.text}</Text>
    </div>
  );
}

/// Per-role Bubble config. Keys beyond the built-in ai/system/user are X's
/// documented extension point (RoleType = Record<AnyStr, RoleProps>).
const BUBBLE_ROLES = {
  user: {
    placement: 'end',
    variant: 'filled',
    avatar: <RoleAvatar glyph="❯" color="#13c2c2" />,
    contentRender: (content) => <TextContent turn={content} />,
  },
  ai: {
    placement: 'start',
    variant: 'outlined',
    avatar: <RoleAvatar glyph="◉" color="#9254de" />,
    contentRender: (content) => <TextContent turn={content} />,
  },
  assistantTurn: {
    placement: 'start',
    variant: 'outlined',
    avatar: <RoleAvatar glyph="◉" color="#9254de" />,
    contentRender: (content) => <AssistantTurnContent turn={content} />,
  },
  // Defensive/history only: since reasoning_delta streams straight into the
  // steps ladder (reduce.js appendThinkDelta), the live path no longer
  // produces `think` turns — this role only renders turns built by older
  // reducers / hand-built fixtures.
  think: {
    placement: 'start',
    variant: 'borderless',
    contentRender: (content) => <ThinkContent turn={content} />,
  },
  // Defensive standalone step ladder. Normal steps+Say runs are grouped by
  // itemsFromTurns into assistantTurn above; this only fires for turns the
  // grouping walk cannot attach (e.g. non-assistant role).
  steps: {
    placement: 'start',
    variant: 'borderless',
    contentRender: (content) => <StepsContent turn={content} />,
  },
  tool: {
    placement: 'start',
    variant: 'borderless',
    contentRender: (content) => <ToolContent turn={content} />,
  },
  sys: {
    placement: 'start',
    variant: 'borderless',
    contentRender: (content) => <SysContent turn={content} />,
  },
  // Subagent fold block: header + status + drill-in replay live in
  // subagentBlock.jsx; the bubble itself stays borderless like tool rows.
  subagent: {
    placement: 'start',
    variant: 'borderless',
    contentRender: (content) => <SubagentContent turn={content} />,
  },
};

/// Footer chip: ▲in / ▼out / Σ total (+ context % only when a frame carried a
/// context-window figure — llm_usage payloads have none today, see report).
export function UsageFooter({ usage }) {
  if (!usage) {
    return null;
  }
  return (
    <div style={{ marginTop: 12, fontFamily: MONO, fontSize: 12 }}>
      <Text type="secondary">{usageLine(usage)}</Text>
    </div>
  );
}

export function StatusTag({ status, error }) {
  if (status === 'done') {
    return <Tag color="green" style={{ marginTop: 8 }}>done</Tag>;
  }
  if (status === 'error') {
    return <Tag color="red" style={{ marginTop: 8 }}>{'error: ' + (error || 'error')}</Tag>;
  }
  if (status === 'streaming') {
    return <Tag color="blue" style={{ marginTop: 8 }}>streaming…</Tag>;
  }
  return null;
}

export function EmptyHint({ text }) {
  return (
    <div style={{ padding: '48px 0', textAlign: 'center' }}>
      <Text type="secondary">{text}</Text>
    </div>
  );
}

export function TranscriptView({ turns, usage, status, error, emptyText }) {
  const empty = isEmptyTranscript(turns, usage);
  // Collapse-all epoch: Ctrl/Cmd+L (or the ⤒ link) bumps the key on
  // Bubble.List, remounting every bubble so all Collapses reset closed —
  // step rows, call rows and subagent blocks alike. Say bubbles remount
  // too but hold no local state, so nothing visually changes for them.
  const [epoch, setEpoch] = useState(0);
  useEffect(() => {
    const onKey = (e) => {
      if (e.key === 'l' && (e.ctrlKey || e.metaKey)) {
        e.preventDefault();
        setEpoch(epoch + 1);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [epoch]);
  return (
    <div>
      {empty ? (
        <EmptyHint text={emptyText || '暂无消息'} />
      ) : (
        <>
          <div style={{ textAlign: 'right' }}>
            <Typography.Link
              onClick={() => setEpoch(epoch + 1)}
              type="secondary"
              style={{ fontFamily: MONO, fontSize: 12 }}
            >
              ⤒ 收起
            </Typography.Link>
          </div>
          <Bubble.List key={epoch} items={itemsFromTurns(turns)} role={BUBBLE_ROLES} autoScroll />
        </>
      )}
      <UsageFooter usage={usage} />
      <StatusTag status={status} error={error} />
    </div>
  );
}
