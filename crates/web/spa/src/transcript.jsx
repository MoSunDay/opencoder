// transcript.jsx — chat transcript on @ant-design/x Bubble.List (T3
// migration of render.jsx). Each reduce.js turn becomes one bubble item:
//   user   → placement end,   variant filled   (❯ avatar, monospace body)
//   ai     → placement start, variant outlined (◉ avatar, monospace body)
//   think  → placement start, variant borderless, ghost 💭 Thinking collapse
//   tool   → placement start, variant borderless, 🔧 collapse with
//            duration + error tag + input/output paragraphs
//   sys    → placement start, variant borderless, centered secondary text
// UsageFooter / StatusTag (moved from render.jsx verbatim in spirit) stay
// below the list; the empty-state hint keeps the old wording contract.

import { Bubble } from '@ant-design/x';
import { Collapse, Tag, Typography } from 'antd';
import { fmtDuration } from './format.js';
import { isEmptyTranscript, itemsFromTurns, usageLine } from './bubbleItems.js';

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

/// Reasoning row: ghost collapse, renders even with empty text so a
/// reasoning-only frame still leaves a visible trace (old ThinkTurn feel).
function ThinkContent({ turn }) {
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
function ToolContent({ turn }) {
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
  think: {
    placement: 'start',
    variant: 'borderless',
    contentRender: (content) => <ThinkContent turn={content} />,
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
  return (
    <div>
      {empty ? (
        <EmptyHint text={emptyText || '暂无消息'} />
      ) : (
        <Bubble.List items={itemsFromTurns(turns)} role={BUBBLE_ROLES} autoScroll />
      )}
      <UsageFooter usage={usage} />
      <StatusTag status={status} error={error} />
    </div>
  );
}
