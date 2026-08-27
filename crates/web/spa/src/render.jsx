// render.jsx — transcript rendering aligned with the TUI look: monospace role
// header lines (❯ User: / ◉ Assistant:), streaming assistant text, collapsible
// tool rows, usage footer, terminal status tag.

import { Collapse, Tag, Typography } from 'antd';
import { absTime, fmtDuration } from './format.js';

const { Text, Paragraph } = Typography;

const ROLE_STYLE = {
  user: { marker: '❯ User:', color: '#13c2c2' },
  assistant: { marker: '◉ Assistant:', color: '#9254de' },
};

export function RoleHeader({ role }) {
  const s = ROLE_STYLE[role] || ROLE_STYLE.assistant;
  return (
    <div style={{
      fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace',
      fontSize: 13,
      fontWeight: 600,
      color: s.color,
      marginTop: 10,
      userSelect: 'none',
    }}
    >
      {s.marker}
    </div>
  );
}

function TextTurn({ turn }) {
  return (
    <>
      <RoleHeader role={turn.role} />
      <Paragraph
        style={{
          fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace',
          fontSize: 13,
          whiteSpace: 'pre-wrap',
          wordBreak: 'break-word',
          marginBottom: 4,
        }}
      >
        {turn.text || ''}
      </Paragraph>
    </>
  );
}

function ThinkTurn({ turn }) {
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
              fontFamily: 'ui-monospace, monospace', fontSize: 12,
              whiteSpace: 'pre-wrap', color: '#8c8c8c', marginBottom: 0,
            }}
          >
            {turn.text || ''}
          </Paragraph>
        ),
      }]}
      style={{ marginLeft: 12, marginBottom: 4 }}
    />
  );
}

function ToolTurn({ turn }) {
  const dur = fmtDuration(turn.durationMs);
  return (
    <div style={{ margin: '4px 0 4px 12px' }}>
      <Collapse
        size="small"
        items={[{
          key: 'tool',
          label: (
            <span style={{ fontFamily: 'ui-monospace, monospace', fontSize: 12 }}>
              🔧 {turn.name || 'tool'}
              {dur ? <Text type="secondary"> · {dur}</Text> : null}
              {turn.isError ? <Tag color="red" style={{ marginLeft: 8 }}>error</Tag> : null}
            </span>
          ),
          children: (
            <>
              {turn.input ? (
                <Paragraph style={{ fontFamily: 'ui-monospace, monospace', fontSize: 12, whiteSpace: 'pre-wrap', marginBottom: 4 }}>
                  <Text type="secondary">input:</Text>
                  {'\n'}
                  {turn.input}
                </Paragraph>
              ) : null}
              {turn.output ? (
                <Paragraph style={{ fontFamily: 'ui-monospace, monospace', fontSize: 12, whiteSpace: 'pre-wrap', marginBottom: 0 }}>
                  <Text type="secondary">output:</Text>
                  {'\n'}
                  {turn.output}
                </Paragraph>
              ) : null}
            </>
          ),
        }]}
      />
    </div>
  );
}

/// Turn dispatcher (kind: text | think | tool | sys).
export function TurnView({ turn }) {
  if (turn.kind === 'text') {
    return <TextTurn turn={turn} />;
  }
  if (turn.kind === 'think') {
    return <ThinkTurn turn={turn} />;
  }
  if (turn.kind === 'tool') {
    return <ToolTurn turn={turn} />;
  }
  if (turn.kind === 'sys') {
    return (
      <div style={{ margin: '2px 0 2px 12px' }}>
        <Text type="secondary" style={{ fontSize: 12 }}>{turn.text}</Text>
      </div>
    );
  }
  return null;
}

/// Footer chip: ▲in / ▼out / Σ total (+ context % only when a frame carried a
/// context-window figure — llm_usage payloads have none today, see report).
export function UsageFooter({ usage }) {
  if (!usage) {
    return null;
  }
  const pct = usage.contextWindow
    ? ' · 上下文 ' + Math.min(999, Math.round((usage.total / usage.contextWindow) * 100)) + '%'
    : '';
  return (
    <div style={{ marginTop: 12, fontFamily: 'ui-monospace, monospace', fontSize: 12 }}>
      <Text type="secondary">
        {'▲ in '}{usage.input}{'  ▼ out '}{usage.output}{'  Σ '}{usage.total}{pct}
      </Text>
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
  if ((!turns || !turns.length) && !usage) {
    return <EmptyHint text={emptyText || '暂无消息'} />;
  }
  return (
    <div>
      {(turns || []).map((t, i) => <TurnView key={i} turn={t} />)}
      <UsageFooter usage={usage} />
      <StatusTag status={status} error={error} />
    </div>
  );
}
