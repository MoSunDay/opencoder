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
//            status tag + child replay drill-in (subagentBlock.jsx); its
//            header shows the shared disclosure glyph (❯ / ▸) and ALL
//            subagent blocks of the transcript open / close together
// Assistant Say stays visible at the Turn level — never folded into Steps.
// Collapse-all: Ctrl/Cmd+L (window keydown) or the `⤒ 收起` link bumps an
// epoch key on Bubble.List, remounting every bubble so the uncontrolled
// Collapses (step rows, call rows) reset closed, and closes the subagent
// disclosure at the same time. That disclosure is ONE transcript-wide
// `{ open, toggle }` (subagentFold.js, TUI toggle_subagents parity) held
// here — not inside the remounted bubbles — and handed to every subagent
// block through SubagentFoldContext: any subagent header toggles all of
// them; the default is closed (▸).
// UsageFooter / StatusTag (moved from render.jsx verbatim in spirit) stay
// below the list; the empty-state hint keeps the old wording contract.

import { useCallback, useEffect, useMemo, useState } from 'react';
import { Bubble } from '@ant-design/x';
import { RobotOutlined, UserOutlined } from '@ant-design/icons';
import { Avatar, Empty, Tag, Typography } from 'antd';
import { isEmptyTranscript, itemsFromTurns, usageLine } from './bubbleItems.js';
import { StepsContent, ThinkContent, ToolContent } from './stepsBlock.jsx';
import { sayPresentation } from './transcript/markdown.js';
import { AssistantText, TextRows } from './transcript/text.jsx';
import { Markdown } from './project/markdown.jsx';
import { SubagentContent } from './subagentBlock.jsx';
import { SubagentFoldContext } from './subagentFold.js';
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
///
/// Subagent blocks absorbed between the ladder and its Say (`itemsFromTurns`)
/// render ABOVE the pair's header row: the section never cuts through the
/// Say — header row and Say body stay contiguous — the same order the TUI
/// flattens (`chat_steps.rs::render_order` hoists the section above the
/// pair); parts from the first Say text on keep their place below the body.
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
  // The lead ends at the FIRST text part, empty or not: the TUI opens its
  // Say block on the first delta of any content (chat_stream.rs), so a
  // whitespace-only first chunk already hoists the section there. Without
  // a text part at all (`[subagent]`, Say not started) nothing hoists —
  // the TUI has no Say block to pair with either, so the order matches in
  // every window and does not move when the text turns non-empty.
  const firstSay = say.findIndex((part) => (
    part && part.kind === 'text' && !part.image && typeof part.text === 'string'
  ));
  const lead = firstSay < 0 ? [] : say.slice(0, firstSay).filter((part) => part && part.kind === 'subagent');
  const body = presentation.other.filter((part) => !lead.includes(part));
  return (
    <div>
      {/* The subagent section sits ABOVE the pair (never inside the Say):
          header row and Say body stay contiguous, like the TUI. */}
      {lead.map((part, index) => (
        <div key={'subagent:' + index} style={{ marginBottom: 8 }}>
          <SubagentContent turn={part} />
        </div>
      ))}
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
            if (part.kind === 'subagent') {
              return <SubagentContent key={'subagent:' + index} turn={part} />;
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
  // Its ❯ / ▸ disclosure is the ONE transcript-wide flag (SubagentFoldContext
  // above), so every subagent block of this list opens / closes together.
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
/// project/projectsTab.jsx). antd's Empty already carries marginBlock: 32, so the
/// old outer 48px padding was double breathing room (~230px total). We drop
/// the wrapper div and pin the Empty's own margin to 24 — the vertical space
/// is now ONE spacing decision, not two stacked ones. `text` still lands in
/// the DOM as the Empty description.
export function EmptyHint({ text }) {
  return <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={text} style={{ marginBlock: 24 }} />;
}

export function TranscriptView({ turns, usage, status, error, emptyText, active = true, autoScroll = true }) {
  const empty = isEmptyTranscript(turns, usage);
  // Collapse-all epoch: Ctrl/Cmd+L (or the ⤒ link) bumps the key on
  // Bubble.List, remounting every bubble so the uncontrolled Collapses
  // reset closed — step rows and call rows alike. Say bubbles remount
  // too but hold no local state, so nothing visually changes for them.
  const [epoch, setEpoch] = useState(0);
  // The subagent disclosure is NOT bubble-local: one transcript-wide flag
  // (default closed) drives every subagent block's ❯ / ▸ glyph and body,
  // toggled from any subagent header (TUI toggle_subagents). It lives here
  // because the bubbles remount on collapse-all — so that path must close
  // it explicitly, next to the epoch bump.
  const [subagentsOpen, setSubagentsOpen] = useState(false);
  const collapseAll = useCallback(() => {
    setEpoch((e) => e + 1);
    setSubagentsOpen(false);
  }, []);
  useEffect(() => {
    if (!active) return undefined;
    const onKey = (e) => {
      if (e.key === 'l' && (e.ctrlKey || e.metaKey)) {
        e.preventDefault();
        collapseAll();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [collapseAll, active]);
  const fold = useMemo(() => ({ open: subagentsOpen, toggle: () => setSubagentsOpen((v) => !v) }), [subagentsOpen]);
  return (
    <div>
      {empty ? (
        <EmptyHint text={emptyText || '暂无消息'} />
      ) : (
        <>
          <div style={{ textAlign: 'right' }}>
            <Typography.Link
              onClick={collapseAll}
              type="secondary"
              style={{ fontFamily: MONO_VAR, fontSize: 12 }}
            >
              ⤒ 收起
            </Typography.Link>
          </div>
          <SubagentFoldContext.Provider value={fold}>
            <Bubble.List key={epoch} items={itemsFromTurns(turns)} role={BUBBLE_ROLES} autoScroll={autoScroll} />
          </SubagentFoldContext.Provider>
        </>
      )}
      <UsageFooter usage={usage} />
      <StatusTag status={status} error={error} />
    </div>
  );
}
