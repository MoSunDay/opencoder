// Agent 资源的文本编码、引用名称与 NFS 挂载提示。

// Meta 页资源引用的展示顺序。
export const REF_FIELDS = ['prompt', 'skills', 'tools', 'memory'];

/// UTF-8 安全的 b64 编码（TextEncoder 先行，避免 btoa 直接吃非 Latin-1）。
export function b64EncodeText(text) {
  const bytes = new TextEncoder().encode(text === undefined || text === null ? '' : String(text));
  let bin = '';
  bytes.forEach((b) => {
    bin += String.fromCharCode(b);
  });
  return btoa(bin);
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
  return `mount -t nfs -o ro,vers=3,tcp,port=${port},mountport=${port},nolock ${host}:/ <dir>`;
}
