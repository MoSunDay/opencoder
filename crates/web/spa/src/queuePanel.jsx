// queuePanel.jsx — TUI queue-panel parity over HTTP: what is still pending
// for the active LOCAL session, one row per admitted input, deletable and
// (queue-only) reorderable before the drain consumes it. Endpoints
// (crates/web/src/api_inputs.rs):
//   GET    /api/sessions/:id/inputs?delivery=steer|queue → {inputs}
//   DELETE /api/sessions/:id/inputs/:seq                 → {ok}
//   POST   /api/sessions/:id/inputs/reorder {a,b}        → {ok}
// chat.jsx bumps `refreshSignal` on every queue_consumed/steer_consumed
// frame — the refresh here is pull-only, never stream-coupled.

import { Button, Tag, Typography } from 'antd';
import { useCallback, useEffect, useState } from 'react';
import { apiDel, apiGet, apiPost } from './api.js';

const { Text } = Typography;

const DELIVERY_LABEL = { steer: { text: 'steer', color: 'blue' }, queue: { text: 'queue', color: 'orange' } };

/// Raw inputs → normalized rows (stable field names, finite seq only).
/// Missing/None payload degrades to [] so a {} router fallback never throws.
export function rowsFromInputs(inputs) {
  const list = Array.isArray(inputs) ? inputs : [];
  return list
    .filter((i) => i && Number.isFinite(i.seq))
    .map((i) => ({
      seq: i.seq,
      delivery: i.delivery === 'queue' ? 'queue' : 'steer',
      prompt: String(i.prompt || ''),
    }));
}

function DeliveryTag({ delivery }) {
  const meta = DELIVERY_LABEL[delivery] || DELIVERY_LABEL.steer;
  return <Tag color={meta.color} style={{ marginRight: 8 }}>{meta.text}</Tag>;
}

function Row({ row, index, rows, delivery, onDelete, onReorder }) {
  const loneQueue = delivery === 'queue' && rows.length < 2;
  const move = (dir) => {
    const other = rows[index + dir];
    if (!other) {
      return; // already at the edge — nothing to swap with
    }
    onReorder(row.seq, other.seq);
  };
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 4, padding: '2px 0' }}>
      <DeliveryTag delivery={row.delivery} />
      <Text style={{ flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }} title={row.prompt}>
        {row.prompt}
      </Text>
      {delivery === 'queue' ? (
        <>
          <Button size="small" type="text" aria-label="上移" disabled={loneQueue || index === 0} onClick={() => move(-1)}>↑</Button>
          <Button size="small" type="text" aria-label="下移" disabled={loneQueue || index === rows.length - 1} onClick={() => move(1)}>↓</Button>
        </>
      ) : null}
      <Button size="small" type="text" danger aria-label="移除" onClick={() => onDelete(row.seq)}>删除</Button>
    </div>
  );
}

export function QueuePanel({ sessionId, refreshSignal }) {
  const [rows, setRows] = useState([]);
  const [open, setOpen] = useState(true);

  const refresh = useCallback(async () => {
    if (!sessionId) {
      setRows([]);
      return;
    }
    const base = '/api/sessions/' + encodeURIComponent(sessionId) + '/inputs';
    try {
      const [steer, queue] = await Promise.all([
        apiGet(base + '?delivery=steer'),
        apiGet(base + '?delivery=queue'),
      ]);
      setRows(rowsFromInputs((steer && steer.inputs || []).concat((queue && queue.inputs) || [])));
    } catch {
      // Session gone / mid-drain 409 — keep the last known rows.
    }
  }, [sessionId]);

  useEffect(() => {
    refresh();
  }, [refresh, refreshSignal]);

  const remove = async (seq) => {
    if (!sessionId) {
      return;
    }
    try {
      await apiDel('/api/sessions/' + encodeURIComponent(sessionId) + '/inputs/' + seq);
    } catch {
      // 404 = already consumed — the refresh below converges the list.
    }
    refresh();
  };

  const reorder = async (a, b) => {
    if (!sessionId || a === b) {
      return;
    }
    try {
      await apiPost('/api/sessions/' + encodeURIComponent(sessionId) + '/inputs/reorder', { a, b });
    } catch {
      // Refusals surface on the next refresh; never block the panel.
    }
    refresh();
  };

  if (!sessionId) {
    return null;
  }
  const steerRows = rows.filter((r) => r.delivery === 'steer');
  const queueRows = rows.filter((r) => r.delivery === 'queue');
  return (
    <div style={{ marginTop: 12, border: '1px solid #f0f0f0', borderRadius: 8, padding: '6px 12px' }}>
      <div
        style={{ display: 'flex', alignItems: 'center', cursor: 'pointer', userSelect: 'none' }}
        onClick={() => setOpen((o) => !o)}
      >
        <Text strong style={{ fontSize: 12 }}>待处理输入</Text>
        <Tag style={{ marginLeft: 8 }} color={DELIVERY_LABEL.steer.color}>{'steer ' + steerRows.length}</Tag>
        <Tag color={DELIVERY_LABEL.queue.color}>{'queue ' + queueRows.length}</Tag>
        <Text type="secondary" style={{ marginLeft: 'auto', fontSize: 12 }}>{open ? '收起 ▾' : '展开 ▸'}</Text>
      </div>
      {open ? (
        <div style={{ marginTop: 4 }}>
          {rows.length === 0 ? <Text type="secondary">暂无待处理输入</Text> : null}
          {steerRows.map((row, i) => (
            <Row key={'steer:' + row.seq} row={row} index={i} rows={steerRows} delivery="steer"
              onDelete={remove} onReorder={reorder} />
          ))}
          {queueRows.map((row, i) => (
            <Row key={'queue:' + row.seq} row={row} index={i} rows={queueRows} delivery="queue"
              onDelete={remove} onReorder={reorder} />
          ))}
        </div>
      ) : null}
    </div>
  );
}
