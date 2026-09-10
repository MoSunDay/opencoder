// appMessage.js — message API that follows the antd App context when there is
// one, and falls back to the static API when there is not.
//
// Why the fallback exists: antd's AppContext defaults to `{ message: {} }`,
// so `App.useApp().message.success(...)` THROWS outside an <App> wrapper —
// and the DOM suites mount individual panels standalone. main.jsx does wrap
// the real app in <App> (so toasts pick up theme + zh-CN locale, which the
// static call cannot see — the official anti-pattern this removes), while
// tests keep working unchanged.

import { App, message as staticMessage } from 'antd';

/// useMessage() -> a message API. Call at the top of a component, never
/// inside a plain helper (hooks rules); pass the result down if needed.
export function useMessage() {
  const ctx = App.useApp();
  const api = ctx && ctx.message;
  return api && typeof api.success === 'function' ? api : staticMessage;
}
