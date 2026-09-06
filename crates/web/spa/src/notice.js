// notice.js — 面板 → 壳层通知的唯一载荷契约（评审 R1 修复）：
// 所有 onNotice 调用必须传本模块构造的 {type, text} 对象，壳层（main.jsx）
// 经 normalizeNotice 兜底后渲染为对应色调的 antd Alert——成功不再被画成红色。
// 纯函数模块：无状态、无副作用，禁止 class。

/// antd Alert 接受的四种语义色调（success 对应 ok，warning 对应 warn）。
const TYPES = ['success', 'error', 'info', 'warning'];

const of = (type) => (text) => ({ type, text });

/// ok(text) — 完成性结果（已创建/已更新/已删除/已保存…）→ 绿色 success。
export const ok = of('success');
/// err(text) — 失败或错误性质 → 红色 error。
export const err = of('error');
/// info(text) — 异步已受理或指引 → 蓝色 info。
export const info = of('info');
/// warn(text) — 前置校验（请选择…/已取消不能恢复…）→ 黄色 warning。
export const warn = of('warning');

/// normalizeNotice(value) — 壳层唯一入口的兜底归一化（全定义域安全）：
/// - 纯字符串 → 按 err 处理（历史调用面兼容，如 onNotice('') 清屏）；
/// - 合法对象（text 为字符串且 type 在 TYPES 内）原样返回；
/// - 其余（null/undefined/缺 text/text 非字符串/type 非法）→ 空错误对象。
export function normalizeNotice(value) {
  if (typeof value === 'string') return err(value);
  if (
    value && typeof value === 'object'
    && typeof value.text === 'string'
    && TYPES.includes(value.type)
  ) {
    return value;
  }
  return { type: 'error', text: '' };
}
