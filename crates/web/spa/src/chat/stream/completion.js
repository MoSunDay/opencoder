import { ensurePendingEcho, turnsFromMessages, usageFromMessages } from '../../reduce.js';

// A fleet stream can finish without a session-level done/error event (for
// example, initialization failed before the runner started). Read the
// execution outcome as well as the persisted transcript before releasing it.
export function completionState(previous, detail, snapshot) {
  const status = detail?.execution?.status;
  const error = detail?.error || ({ error: '执行失败，未返回错误详情', cancelled: '执行已取消', interrupted: '执行已中断' }[status] || null);
  const messages = snapshot?.messages || [];
  return {
    ...previous,
    status: error ? 'error' : 'done',
    error,
    turns: messages.length ? ensurePendingEcho(turnsFromMessages(messages), previous.pendingEcho) : previous.turns,
    usage: messages.length ? usageFromMessages(messages) : previous.usage,
    pendingEcho: null,
  };
}
