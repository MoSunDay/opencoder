// envModel.js — Env 面板的纯数据口径：context 归一化 + 工具目录过滤。
// 纯函数模块：无状态、无副作用，不引 React。

/// GET /api/todo/envs/:name 的「context object」归一化：{env:{...}} 包装或
/// 裸对象都接受。
export function envFromContext(j) {
  if (!j || typeof j !== 'object') {
    return null;
  }
  const e = j.env && typeof j.env === 'object' ? j.env : j;
  return e && typeof e.name === 'string' ? e : null;
}

/// shareTools(tools) → 目录中已导入（source !== 'importable'）且带 ref 的条目。
export function shareTools(tools) {
  return (tools || []).filter((t) => t && t.ref && t.source !== 'importable');
}

/// importableTools(tools) → 目录中可导入（source === 'importable'）且带 ref
/// 的条目（来自本地 agent root，逐条 POST import 后才进 share）。
export function importableTools(tools) {
  return (tools || []).filter((t) => t && t.ref && t.source === 'importable');
}
