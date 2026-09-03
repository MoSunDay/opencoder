// subagentBlock.jsx — render for `subagent` turns (reduce.js folds
// subagent_start / subagent_child / subagent_end into one turn of kind
// 'subagent' carrying the child's already-reduced `events`). TUI chat.rs
// parity: one collapsible block per subagent, plus a drill-in modal that
// replays the child session read-only from /events after=0 (same signed
// stream as the main console, folded by the same reduceFrame — NOT a
// Bubble.List, the block stays light). Self-contained: no props drilled
// in from chat state.

import { Collapse, Modal, Tag, Typography } from 'antd';
import { useEffect, useRef, useState } from 'react';
import { emptyStream, reduceFrame } from './reduce.js';
import { openStream } from './sse.js';

const { Text, Paragraph } = Typography;

const MONO = 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace';

/// antd Tag color per subagent turn status.
const STATUS_COLOR = {
  running: 'processing',
  done: 'success',
  error: 'error',
  cancelled: 'default',
};

export function statusColorOf(status) {
  return STATUS_COLOR[status] || 'processing';
}

/// Child usage → 'Σ n tokens', or null when the run emitted no llm_usage.
export function usageTextOf(usage) {
  const u = usage || {};
  return Number.isFinite(u.total) && u.total !== null ? 'Σ ' + u.total + ' tokens' : null;
}

/// A subagent turn ({events, usage}) → compact line descriptors for both the
/// collapsed block body and the drill-in replay (child streams fold into the
/// SAME reduce.js turn shapes, so one renderer covers live and replay).
/// `steps` events (the child's step ladder) flatten to ONE line per call —
/// the child's step thinking is intentionally not listed at this density.
export function childLines(turn) {
  const t = turn || {};
  const list = Array.isArray(t.events) ? t.events : [];
  const lines = [];
  list.forEach((ev, i) => {
    const kind = (ev && ev.kind) || 'text';
    if (kind === 'steps') {
      let j = 0;
      for (const step of (ev && ev.steps) || []) {
        for (const call of (step && step.calls) || []) {
          lines.push({ key: 'tool:' + i + ':' + j, kind: 'tool', text: call.name || 'tool', isError: !!call.isError });
          j += 1;
        }
      }
      return;
    }
    if (kind === 'tool') {
      lines.push({ key: 'tool:' + i, kind: 'tool', text: (ev && ev.name) || 'tool', isError: !!(ev && ev.isError) });
      return;
    }
    if (kind === 'think') {
      lines.push({ key: 'think:' + i, kind: 'think', text: (ev && ev.text) || '' });
      return;
    }
    if (kind === 'sys') {
      lines.push({ key: 'sys:' + i, kind: 'sys', text: (ev && ev.text) || '' });
      return;
    }
    lines.push({ key: 'text:' + i, kind: 'text', text: (ev && ev.text) || '' });
  });
  const usage = usageTextOf(t.usage);
  if (usage) {
    lines.push({ key: 'usage', kind: 'usage', text: usage });
  }
  return lines;
}

function LineRow({ line }) {
  if (line.kind === 'tool') {
    return (
      <div style={{ fontFamily: MONO, fontSize: 12, padding: '1px 0' }}>
        🔧 {line.text}
        {line.isError ? <Tag color="red" style={{ marginLeft: 8 }}>error</Tag> : null}
      </div>
    );
  }
  if (line.kind === 'think') {
    return <Paragraph style={{ fontStyle: 'italic', color: '#8c8c8c', fontSize: 12, whiteSpace: 'pre-wrap', marginBottom: 0 }}>{line.text}</Paragraph>;
  }
  if (line.kind === 'sys' || line.kind === 'usage') {
    return <Text type="secondary" style={{ fontSize: 12 }}>{line.text}</Text>;
  }
  return (
    <Paragraph style={{ fontFamily: MONO, fontSize: 12, whiteSpace: 'pre-wrap', wordBreak: 'break-word', margin: 0 }}>
      {line.text}
    </Paragraph>
  );
}

export function CompactLines({ lines }) {
  return (
    <div>
      {(lines || []).map((line) => <LineRow key={line.key} line={line} />)}
    </div>
  );
}

function SubagentLabel({ turn, open, onView }) {
  const status = (turn && turn.status) || 'running';
  const showSummary = !open && turn && typeof turn.summary === 'string' && turn.summary;
  return (
    <span style={{ fontFamily: MONO, fontSize: 12 }}>
      🤖 {(turn && turn.name) || 'subagent'} · {status}
      <Tag color={statusColorOf(status)} style={{ marginLeft: 8 }}>{status}</Tag>
      {showSummary ? <Text type="secondary">{turn.summary}</Text> : null}
      {turn && turn.childSessionId ? (
        <Typography.Link
          style={{ marginLeft: 8 }}
          onClick={(e) => {
            e.stopPropagation(); // drill in without toggling the collapse
            onView();
          }}
        >
          [→ view]
        </Typography.Link>
      ) : null}
    </span>
  );
}

export function SubagentContent({ turn }) {
  const [open, setOpen] = useState(false);
  const [viewing, setViewing] = useState(false);
  const [child, setChild] = useState(emptyStream);
  const streamRef = useRef(null);
  const childSessionId = (turn && turn.childSessionId) || null;

  // Drill-in replay: open on modal mount, abort on close (effect cleanup
  // covers both the ✕ and the unmount path).
  useEffect(() => {
    if (!viewing || !childSessionId) {
      return undefined;
    }
    setChild(emptyStream());
    streamRef.current = openStream({
      path: '/api/sessions/' + encodeURIComponent(childSessionId) + '/events',
      sessionId: childSessionId,
      after: 0,
      onFrame: (f) => setChild((s) => reduceFrame(s, f, Date.now())),
    });
    return () => {
      if (streamRef.current) {
        streamRef.current.abort();
        streamRef.current = null;
      }
    };
  }, [viewing, childSessionId]);

  const lines = childLines({ events: (turn && turn.events) || [], usage: turn && turn.usage });
  const replayLines = childLines({ events: child.turns, usage: child.usage });
  return (
    <div>
      <Collapse
        size="small"
        activeKey={open ? ['sa'] : []}
        onChange={(keys) => setOpen((Array.isArray(keys) ? keys : [keys]).includes('sa'))}
        items={[{
          key: 'sa',
          label: <SubagentLabel turn={turn} open={open} onView={() => setViewing(true)} />,
          children: <CompactLines lines={lines} />,
        }]}
      />
      <Modal
        title={'🤖 ' + ((turn && turn.name) || 'subagent') + ' · 子会话回放'}
        open={viewing}
        footer={null}
        width={640}
        onCancel={() => setViewing(false)}
      >
        {replayLines.length
          ? <CompactLines lines={replayLines} />
          : <Text type="secondary">暂无子会话事件</Text>}
      </Modal>
    </div>
  );
}
