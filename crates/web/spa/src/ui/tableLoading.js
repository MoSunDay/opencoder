// tableLoading.js — 控制台列表表格 `loading` 语义的唯一约定（纯数据/纯函数，
// 不引 React）。两条 antd 6 的实现事实决定了它存在：
//
// 1. Table 把 `loading` 原样当 Spin props（es/table/hooks/useSpinProps.js）：
//    裸 boolean 变成 `delay: 0`，于是一次 <100ms 的刷新也会闪一下遮罩。这里
//    统一带上 SPIN_DELAY_MS，短刷新在视觉上完全消失。
// 2. Spin 生效时（根节点带 `.ant-spin-spinning`，antd 6 已无 v5 的
//    `.ant-spin-blur`）样式给 `.ant-spin-container` 上 opacity .5 +
//    pointer-events: none（es/spin/style/index.js 的 `&-spinning` 块）——被
//    遮罩的表格里的行内链接/按钮全部点不动；而空态占位只有在
//    `spinProps.spinning` 为真且 dataSource 与内部 EMPTY_LIST 同引用
//    （即 undefined/null）时才被抑制（es/table/InternalTable.js）。
//    `dataSource={[]}` 会一边拉取一边断言「暂无 …」——首屏对着用户撒谎。
//
// 所以：拉取中把 dataSource 交回 undefined（不知道就是不知道），并用带 delay
// 的对象表达 loading。新增列表表格请一律走这两个函数，不要各写各的语义。

/// Spin 延迟（ms）：短于该时长的拉取根本不渲染遮罩。
export const SPIN_DELAY_MS = 200;

/// tableLoading(spinning) -> antd Table 的 loading 值（对象形式，delay 生效）。
export function tableLoading(spinning) {
  return { spinning: !!spinning, delay: SPIN_DELAY_MS };
}

/// tableRows(spinning, rows) -> 拉取中返回 undefined（抑制撒谎的空态），否则
/// 原样返回同一个数组引用，不拷贝，避免 Table 因新引用而无谓重渲染。
export function tableRows(spinning, rows) {
  return spinning ? undefined : rows;
}
