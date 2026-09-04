// teamItems.js — pure mapping from the team/topic API rows to display items
// (status tag colors, capability digests, timeline view models). Kept
// DOM-free so the whole contract is guarded by the node-env suite
// teamItems.test.js, mirroring conversationItems.js / bubbleItems.js.

export { absTime as fmtTime } from './format.js';

/// finish_reason → antd Tag preset color + Chinese label:
/// executing → processing; finished+complete → success; cancelled → default;
/// error → error; max_turns/max_sub_turns → warning.
const FINISH_VIEW = {
  complete: { color: 'success', label: '已完成' },
  max_turns: { color: 'warning', label: '轮数上限' },
  max_sub_turns: { color: 'warning', label: '子轮上限' },
  cancelled: { color: 'default', label: '已取消' },
  error: { color: 'error', label: '错误' },
};
const FINISHED_UNSET = { color: 'default', label: '已结束' };
const EXECUTING_VIEW = { color: 'processing', label: '执行中' };

export function topicStatusView(topic) {
  const t = topic || {};
  if (t.status === 'executing') {
    return EXECUTING_VIEW;
  }
  if (t.status === 'finished') {
    return FINISH_VIEW[t.finish_reason] || FINISHED_UNSET;
  }
  return { color: 'default', label: t.status || '-' };
}

/// finish_reason column text: the raw machine value ('-' until stamped) —
/// the Chinese rendering already lives in the status Tag next to it.
export function finishReasonText(reason) {
  return reason || '-';
}

function capListText(caps, max) {
  const head = caps.slice(0, max).join(' / ');
  return caps.length > max ? head + ' +' + (caps.length - max) : head;
}

/// One member's capabilities as text. Members without profiled_at read
/// 未画像 so operators can see profiling never ran (vs a genuinely empty 无).
export function memberCapsText(member, max = 4) {
  const m = member || {};
  const caps = Array.isArray(m.capabilities) ? m.capabilities.filter(Boolean) : [];
  if (caps.length === 0) {
    return m.profiled_at ? '无' : '未画像';
  }
  return capListText(caps, max);
}

/// Team-level capability digest for the table's 能力概况 column: the
/// distinct union across members, with the same 未画像 / 无 / '-' ladder.
export function teamCapSummary(team, max = 4) {
  const members = team && Array.isArray(team.members) ? team.members : [];
  const seen = [];
  members.forEach((m) => {
    (m && Array.isArray(m.capabilities) ? m.capabilities : []).forEach((c) => {
      if (c && !seen.includes(c)) {
        seen.push(c);
      }
    });
  });
  if (seen.length === 0) {
    if (members.length === 0) {
      return '-';
    }
    return members.some((m) => m && m.profiled_at) ? '无' : '未画像';
  }
  return capListText(seen, max);
}

/// /api/nodes rows → Select options shared by the member picker.
export function nodeSelectOptions(nodes) {
  return (Array.isArray(nodes) ? nodes : [])
    .filter((n) => n && n.id)
    .map((n) => ({ value: n.id, label: n.name || n.id }));
}

/// Captain picker options: current members first (tagged 成员), then any
/// other online node (tagged 节点), deduped by node id.
export function captainOptions(team, nodes) {
  const seen = [];
  const out = [];
  (team && Array.isArray(team.members) ? team.members : []).forEach((m) => {
    if (m && m.node_id && !seen.includes(m.node_id)) {
      seen.push(m.node_id);
      out.push({ value: m.node_id, label: (m.name || m.node_id) + ' · 成员' });
    }
  });
  (Array.isArray(nodes) ? nodes : []).forEach((n) => {
    if (n && n.id && !seen.includes(n.id)) {
      seen.push(n.id);
      out.push({ value: n.id, label: (n.name || n.id) + ' · 节点' });
    }
  });
  return out;
}

/// Aligned flag for a turn. The list endpoint stamps `aligned` directly;
/// the detail endpoint keeps it inside the last sub-turn's summary, so both
/// row shapes resolve through here.
export function turnAligned(turn) {
  const t = turn || {};
  if (typeof t.aligned === 'boolean') {
    return t.aligned;
  }
  const subs = Array.isArray(t.sub_turns) ? t.sub_turns : [];
  const last = subs.length > 0 ? subs[subs.length - 1] : null;
  return !!(last && last.summary && last.summary.aligned);
}

export function subTurnCount(turn) {
  return (turn && Array.isArray(turn.sub_turns) ? turn.sub_turns : []).length;
}

/// turns → plain Timeline view models (key/color/question/...) so the
/// mapping stays testable without a DOM. The question/participants live
/// top-level on list rows but inside `plan` on detail rows.
export function turnTimelineItems(turns) {
  return (Array.isArray(turns) ? turns : [])
    .filter((t) => t && t.turn !== undefined && t.turn !== null)
    .map((t) => ({
      key: t.turn,
      turn: t.turn,
      color: turnAligned(t) ? 'green' : 'gray',
      question: t.question || (t.plan && t.plan.question) || '',
      participants: t.participants || (t.plan && t.plan.participants) || [],
      aligned: turnAligned(t),
      subTurns: subTurnCount(t),
    }));
}

/// Collapse header for one member result: who, kind (对齐追答 vs 回答) and
/// the failure marker when ok=false.
export function resultLabel(r) {
  const x = r || {};
  const kind = x.kind === 'alignment' ? '对齐追答' : '回答';
  const failed = x.ok === false ? ' · 失败' : '';
  return (x.node_id || '-') + ' · ' + kind + failed;
}

/// One ambiguity row as text for the summary's 对齐链 list.
export function ambiguityText(a) {
  const x = a || {};
  return (x.node_id || '-') + '：' + (x.question || '');
}

/// Action availability for the topics table: only executing topics cancel;
/// only finished non-complete topics resume.
export function topicCancelable(topic) {
  return !!topic && topic.status === 'executing';
}

export function topicResumable(topic) {
  return !!topic && topic.status === 'finished' && topic.finish_reason !== 'complete';
}
