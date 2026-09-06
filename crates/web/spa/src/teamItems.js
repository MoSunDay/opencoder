// teamItems.js — pure mapping helpers for the team modal pickers (member
// capability digests, node/captain Select options). Kept DOM-free so the
// whole contract is guarded by the node-env suite teamItems.test.js,
// mirroring conversationItems.js / bubbleItems.js.

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
