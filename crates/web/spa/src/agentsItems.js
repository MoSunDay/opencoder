// agentsItems.js — 「Agent 配置」页的纯展示/编码助手：资源文件 b64 文本
// 编解码（PUT files 载荷 / GET content_b64 还原）、NFS 挂载提示行、卡片
// current 引用 → 表格单元格、资源池 → Select options 的映射。无 DOM、无
// JSX —— 与 teamItems.js / conversationItems.js 同一层。

/// 卡片的四类引用：field（meta.current 的键）↔ 池类别 cat 一一对应。
export const REF_FIELDS = [
  { field: 'prompt', label: 'Prompt', cat: 'prompts' },
  { field: 'skills', label: 'Skills', cat: 'skills' },
  { field: 'tools', label: 'Tools', cat: 'tools' },
  { field: 'memory', label: 'Memory', cat: 'memory' },
];

/// UTF-8 安全的 b64 编码（TextEncoder 先行，避免 btoa 直接吃非 Latin-1）。
export function b64EncodeText(text) {
  const bytes = new TextEncoder().encode(text === undefined || text === null ? '' : String(text));
  let bin = '';
  bytes.forEach((b) => {
    bin += String.fromCharCode(b);
  });
  return btoa(bin);
}

/// 宽容解码：截断/脏 b64 或非 UTF-8 字节一律降级为 ''，绝不在渲染途中
/// 抛错（一个坏文件不能拖垮整个面板）。
export function b64DecodeText(b64) {
  try {
    const bin = atob(String(b64 || ''));
    const bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i += 1) {
      bytes[i] = bin.charCodeAt(i);
    }
    return new TextDecoder().decode(bytes);
  } catch {
    return '';
  }
}

/// 卡片 current → 四个展示单元格（未引用 ⇒ value ''，渲染层显示 `—`）。
export function refCells(current) {
  const c = current && typeof current === 'object' ? current : {};
  return REF_FIELDS.map(({ field, label }) => ({ field, label, value: c[field] || '' }));
}

/// 池列表 [{name, current, versions}] → Select options（`name · vN`）。
export function resourceOptions(resources) {
  return (resources || []).filter(Boolean).map((r) => ({
    value: r.name,
    label: r.current ? `${r.name} · v${r.current}` : `${r.name}`,
  }));
}

/// 版本号列表 → 降序 Select options（回滚目标，新版本在前）。
export function versionOptions(versions) {
  return [...(versions || [])].sort((a, b) => b - a).map((v) => ({ value: v, label: `v${v}` }));
}

/// references 解析快照 → 只读名称清单（memory 是布尔 ⇒ 固定一行文案；
/// prompt 走 prompt_files 文件主干）。
export function resolvedNames(references, field) {
  const r = references && typeof references === 'object' ? references : {};
  if (field === 'memory') {
    return r.memory ? ['memory 已接入'] : [];
  }
  const key = field === 'prompt' ? 'prompt_files' : field;
  const v = r[key];
  return Array.isArray(v) ? v.map(String) : [];
}

/// NFS 状态 → mount(8) 提示行（占位符换成实参；仅运行中有意义）。
export function mountHint(status) {
  const s = status && typeof status === 'object' ? status : {};
  const host = s.host || 'HOST';
  const port = s.port || 'PORT';
  return `mount -t nfs -o vers=3,tcp,port=${port},mountport=${port},nolock ${host}:/ <dir>`;
}
