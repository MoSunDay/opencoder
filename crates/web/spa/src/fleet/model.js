export const CREATABLE_KINDS = [
  ['agent', 'Agent'], ['team', 'Team'], ['dag', 'DAG'], ['todos', 'TODO 工作流'],
  ['project', '项目任务'],
].map(([value, label]) => ({ value, label }));
export const KINDS = [...CREATABLE_KINDS, { value: 'system', label: '历史系统执行' }];
export const KIND_LABELS = Object.fromEntries(KINDS.map(({ value, label }) => [value, label]));
// 执行状态表已搬进 src/ui/statusTag.jsx（全控制台唯一状态→视觉映射）；这里
// re-export 保持旧导入路径（executions/detail/todoRunsPanel）零改动，文案
// 与颜色逐字不变（DOM 测试守卫）。
export { STATUS_COLORS, STATUS_LABELS } from '../ui/statusTag.jsx';
export function newId(kind) {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  return `${kind}-${Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('')}`;
}
export function nodeOptions(nodes, kind) {
  return [{ value: '', label: '自动调度（活跃 loop / CPU 最低）' }, ...nodes.map((n) => ({
    value: n.id, label: `${n.name} · ${n.snapshot?.active_agent_loops ?? '?'} loops / ${n.snapshot?.cpu_capacity ?? '?'} CPU`,
    disabled: !n.online || !n.snapshot?.ready || (kind && !n.kinds?.includes(kind)),
  }))];
}
// Conversation and Brain launch require an explicit, executable node.
export function explicitNodeOptions(nodes = [], kind = null) {
  return nodeOptions(nodes, kind).filter((option) => option.value).map((option) => ({
    ...option, label: nodes.find((node) => node.id === option.value)?.name || option.value,
  }));
}
export function canUseNode(nodes, id, kind = null) {
  return !!id && explicitNodeOptions(nodes, kind).some((option) => option.value === id && !option.disabled);
}

export function textOf(message) {
  if (message.display) return message.display;
  if (typeof message.content === 'string') return message.content;
  return (message.blocks || message.content || []).filter((b) => (b.kind || b.type) === 'text').map((b) => b.text).join('\n');
}

export function executionActions(execution) {
  const status = execution?.status;
  const system = execution?.kind === 'system';
  return {
    interrupt: ['pending', 'running', 'idle'].includes(status),
    cancel: ['pending', 'running', 'idle', 'cancelling', 'interrupted'].includes(status),
    resume: !system && ['interrupted', 'error'].includes(status),
  };
}

export function executionPagePath(kind, cursor, limit = 50) {
  const query = new URLSearchParams({ limit: String(limit) });
  if (kind) query.set('kind', kind);
  if (cursor) {
    query.set('cursor_created_at', String(cursor.created_at));
    query.set('cursor_id', cursor.id);
  }
  return `/api/executions?${query}`;
}

export function messagePagePath(id, cursor) {
  const query = new URLSearchParams();
  if (cursor?.seq) query.set('seq', String(cursor.seq));
  if (cursor?.offset) query.set('offset', String(cursor.offset));
  return `/api/executions/${encodeURIComponent(id)}/messages${query.size ? `?${query}` : ''}`;
}

export function decodeBase64(value) {
  const raw = atob(value);
  return Uint8Array.from(raw, (char) => char.charCodeAt(0));
}

function joinBytes(parts, total) {
  const joined = new Uint8Array(total);
  let offset = 0;
  parts.forEach((part) => { joined.set(part, offset); offset += part.byteLength; });
  return joined;
}

export const LARGE_MESSAGE_BYTES = 512 * 1024;
const RETAINED_MESSAGE_BYTES = 4 * 1024 * 1024;
const RETAINED_MESSAGE_COUNT = 100;

function retainMessages(messages) {
  const kept = [...messages];
  let bytes = kept.reduce((sum, message) => sum + (message.byteLength || 0), 0);
  let dropped = false;
  while (kept.length > 1 && (kept.length > RETAINED_MESSAGE_COUNT || bytes > RETAINED_MESSAGE_BYTES)) {
    bytes -= kept.shift().byteLength || 0;
    dropped = true;
  }
  return { messages: kept, dropped };
}

export function decodeWindow(parts, leading = new Uint8Array(), eof = false) {
  const total = leading.byteLength + parts.reduce((sum, part) => sum + part.byteLength, 0);
  const bytes = joinBytes([leading, ...parts], total);
  const maxTail = eof ? 0 : 3;
  for (let removed = 0; removed <= 3 + maxTail; removed += 1) {
    for (let headLength = 0; headLength <= Math.min(3, removed, bytes.byteLength); headLength += 1) {
      const tailLength = removed - headLength;
      if (tailLength > maxTail || headLength + tailLength > bytes.byteLength) continue;
      try {
        const end = bytes.byteLength - tailLength;
        const text = new TextDecoder('utf-8', { fatal: true }).decode(bytes.slice(headLength, end));
        return { text, skipped: headLength, tail: bytes.slice(end) };
      } catch {
        // A UTF-8 scalar can straddle either window boundary by at most 3 bytes.
      }
    }
  }
  throw new Error('消息内容不是有效文本');
}

/// Appends one wire page without decoding an incomplete UTF-8/JSON message.
/// The retained partial is bounded by the single message the user elects to load.
export function appendMessagePage(previous, page, leading = new Uint8Array()) {
  const state = previous || { messages: [], partial: null };
  const messages = [...state.messages];
  let partial = state.partial;
  const largeGroups = [];
  let large = null;
  const finishLarge = () => {
    if (large) largeGroups.push(large);
    large = null;
  };
  for (const chunk of page?.chunks || []) {
    if (chunk.encoding !== 'base64' || !Number.isSafeInteger(chunk.seq) || !Number.isSafeInteger(chunk.offset)) {
      throw new Error('消息分段格式无效');
    }
    const bytes = decodeBase64(chunk.bytes_b64 || '');
    if (chunk.next_offset !== chunk.offset + bytes.byteLength || chunk.next_offset > chunk.total_bytes) {
      throw new Error('消息分段长度无效');
    }
    if (chunk.total_bytes > LARGE_MESSAGE_BYTES) {
      if (partial) throw new Error('消息分段不完整');
      if (!large || large.seq !== chunk.seq) {
        finishLarge();
        large = { seq: chunk.seq, role: chunk.role, created_at: chunk.created_at, start: chunk.offset, end: chunk.offset, total: chunk.total_bytes, eof: false, parts: [] };
      }
      if (chunk.offset !== large.end) throw new Error('消息分段顺序无效');
      large.parts.push(bytes); large.end = chunk.next_offset; large.eof = !!chunk.eof;
      continue;
    }
    if (!partial || partial.seq !== chunk.seq) {
      if (partial) throw new Error('消息分段不完整');
      if (chunk.offset !== 0) throw new Error('消息分段起点无效');
      partial = { seq: chunk.seq, role: chunk.role, created_at: chunk.created_at, total: chunk.total_bytes, next: 0, parts: [] };
    }
    if (chunk.offset !== partial.next || chunk.total_bytes !== partial.total || chunk.next_offset < chunk.offset) {
      throw new Error('消息分段顺序无效');
    }
    partial = { ...partial, next: chunk.next_offset, parts: [...partial.parts, bytes] };
    if (chunk.eof) {
      if (partial.next !== partial.total) throw new Error('消息分段提前结束');
      let blocks;
      try {
        blocks = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(joinBytes(partial.parts, partial.total)));
      } catch {
        throw new Error('消息内容损坏，无法读取');
      }
      const message = { id: `message-${partial.seq}`, seq: partial.seq, role: partial.role, created_at: partial.created_at, byteLength: partial.total, blocks, content: blocks };
      if (!messages.some((item) => item.seq === message.seq)) messages.push(message);
      partial = null;
    }
  }
  finishLarge();
  const largeWindows = largeGroups.map((group, index) => {
    const prefix = index === 0 ? leading : new Uint8Array();
    const decoded = decodeWindow(group.parts, prefix, group.eof);
    const { parts, ...wire } = group;
    return {
      ...wire,
      start: Math.max(0, group.start - prefix.byteLength + decoded.skipped),
      end: group.end - decoded.tail.byteLength,
      text: decoded.text,
      tail: decoded.tail,
    };
  });
  const retained = retainMessages(messages);
  return {
    messages: retained.messages,
    trimmed: !!state.trimmed || retained.dropped,
    partial,
    large: largeWindows,
    nextCursor: page?.next_cursor || null,
    more: !!page?.more,
  };
}
