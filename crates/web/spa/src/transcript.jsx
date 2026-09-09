// transcript.jsx — chat transcript on @ant-design/x Bubble.List (T3
// migration of render.jsx). Reduced assistant segments that contain a step
// ladder become one visual Turn bubble:
//   user   → placement end,   variant filled   (user avatar, monospace body)
//   ai     → placement start, variant outlined (robot avatar, monospace body)
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

import { useEffect, useMemo, useState } from 'react';
import { Bubble } from '@ant-design/x';
import { RobotOutlined, UserOutlined } from '@ant-design/icons';
import { Avatar, Empty, Tag, Typography } from 'antd';
import { isEmptyTranscript, itemsFromTurns, usageLine } from './bubbleItems.js';
import { StepsContent, ThinkContent, ToolContent } from './stepsBlock.jsx';
import { sayPresentation } from './transcript/markdown.js';
import { AssistantText, TextRows } from './transcript/text.jsx';
import { Markdown } from './project/markdown.jsx';
import { SubagentContent } from './subagentBlock.jsx';
import { MONO_VAR } from './ui/mono.js';

const { Text, Paragraph } = Typography;

function RoleAvatar({ role }) {
  const user = role === 'user';
  return (
    <Avatar
      role="img"
      aria-label={user ? '用户' : 'Agent'}
      size={32}
      icon={user ? <UserOutlined aria-hidden /> : <RobotOutlined aria-hidden />}
      style={{
        // The 10% wash used to be `hex + '1a'`; a var() cannot be string-
        // concatenated, so the rgba() wash reads the -rgb twins from :root.
        background: user ? 'rgba(var(--oc-accent-user-rgb), 0.1)' : 'rgba(var(--oc-accent-ai-rgb), 0.1)',
        color: user ? 'var(--oc-accent-user)' : 'var(--oc-accent-ai)',
        flexShrink: 0,
      }}
    />
  );
}

/// user / ai body: same monospace pre-wrap paragraph the old TextTurn used
/// (message roles are identified by the user / robot avatars).
function TextContent({ turn }) {
  if (turn.role === 'assistant' && !turn.image) return <AssistantText turn={turn} />;
  return (
    <Paragraph
      style={{
        fontFamily: MONO_VAR,
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
  // Streaming keeps the compact raw preview. Completed answers render the
  // complete Markdown document below the ladder; no line is stripped from
  // a heading, list, code fence or table, and no preview duplicates the body.
  const presentation = useMemo(() => {
    if (turn.sayActive === true) return sayPresentation(say, true);
    const isText = (part) => part.kind === 'text' && !part.image;
    return { preview: '', rows: [], markdown: say.filter(isText).map((part) => part.text || '').join(''), other: say.filter((part) => !isText(part)) };
  }, [say, turn.sayActive]);
  const body = presentation.other;
  return (
    <div>
      <StepsContent turn={turn} preview={presentation.preview} />
      {body.length > 0 || presentation.rows.length > 0 || presentation.markdown ? (
        // Keep the answer body separate from the expandable step summary.
        <div style={{ marginTop: 16 }}>
          {presentation.markdown && <Markdown text={presentation.markdown} />}
          {!!presentation.rows.length && <TextRows rows={presentation.rows} />}
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
    avatar: <RoleAvatar role="user" />,
    contentRender: (content) => <TextContent turn={content} />,
  },
  ai: {
    placement: 'start',
    variant: 'outlined',
    avatar: <RoleAvatar role="assistant" />,
    contentRender: (content) => <TextContent turn={content} />,
  },
  assistantTurn: {
    placement: 'start',
    variant: 'outlined',
    avatar: <RoleAvatar role="assistant" />,
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
    <div style={{ marginTop: 12, fontFamily: MONO_VAR, fontSize: 12 }}>
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

/// Empty-state hint: the console-wide antd Empty idiom (same as
/// project/goalsTab.jsx). antd's Empty already carries marginBlock: 32, so the
/// old outer 48px padding was double breathing room (~230px total). We drop
/// the wrapper div and pin the Empty's own margin to 24 — the vertical space
/// is now ONE spacing decision, not two stacked ones. `text` still lands in
/// the DOM as the Empty description.
export function EmptyHint({ text }) {
  return <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={text} style={{ marginBlock: 24 }} />;
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
              style={{ fontFamily: MONO_VAR, fontSize: 12 }}
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
