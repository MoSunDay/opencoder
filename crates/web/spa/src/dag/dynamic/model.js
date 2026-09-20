export const inputNodes = (spec) => (spec?.steps || []).filter((s) => s.kind?.type === 'dynamic' && s.kind.source?.type === 'input');

export function batchError(template, items) {
  if (!Array.isArray(items)) return '请输入 JSON 数组';
  if (items.length > 1000) return '单节点最多 1,000 个实例';
  for (let i = 0; i < items.length; i += 1) {
    if (template?.type === 'agent') {
      if (typeof items[i] !== 'string') return `实例 ${i} 必须是文本`;
    } else if (template?.type === 'wasm') {
      if (!Array.isArray(items[i]) || items[i].some((s) => typeof s !== 'string' || s.includes('\0'))) return `实例 ${i} 必须是字符串 argv 数组`;
    } else return '模板必须是 Agent 或 Wasm';
  }
  return '';
}

export function dispatchInput(spec, batches) {
  const entries = inputNodes(spec).map((s) => {
    let items;
    try { items = JSON.parse(batches[s.name] ?? '[]'); } catch { throw new Error(`${s.name}: JSON 格式错误`); }
    const error = batchError(s.kind.template, items);
    if (error) throw new Error(`${s.name}: ${error}`);
    return { pointer: s.kind.source.pointer, items };
  });
  let input = Object.create(null);
  const seen = new Map();
  for (const { pointer, items } of entries) {
    if (typeof pointer !== 'string' || (pointer && !pointer.startsWith('/')) || /~(?![01])/u.test(pointer)) throw new Error('无效的输入 JSON pointer');
    if (seen.has(pointer)) {
      if (JSON.stringify(seen.get(pointer)) !== JSON.stringify(items)) throw new Error(`共享路径 ${pointer} 的批次必须一致`);
      continue;
    }
    if ([...seen.keys()].some((p) => p === '' || pointer === '' || p.startsWith(pointer + '/') || pointer.startsWith(p + '/'))) throw new Error('动态输入路径不能相互包含');
    seen.set(pointer, items);
    if (!pointer) { input = items; continue; }
    const parts = pointer.slice(1).split('/').map((s) => s.replace(/~1/g, '/').replace(/~0/g, '~'));
    let target = input;
    parts.forEach((part, i) => {
      if (i === parts.length - 1) Object.defineProperty(target, part, { value: items, enumerable: true, writable: true });
      else {
        if (!Object.hasOwn(target, part)) Object.defineProperty(target, part, { value: Object.create(null), enumerable: true });
        target = target[part];
      }
    });
  }
  return input;
}

export function progressLabel(progress) {
  if (!progress) return '等待派生';
  return progress.total === 0 ? '0/0，无需执行' : `${progress.done}/${progress.total} 成功`;
}
