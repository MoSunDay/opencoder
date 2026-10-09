import { apiGet, apiPost } from '../../api.js';

export const SUBMIT_WAIT_MS = 15000;
export const RECEIPT_POLL_MS = 3000;
export const uncertainFailure = (error) => !error.status || error.status >= 500 || [408, 425, 429].includes(error.status);

// Bound both headers and response-body reads. Aborting only the browser request
// does not cancel an execution which the server may already have accepted.
export async function boundedRequest(read, signal, timeout = SUBMIT_WAIT_MS) {
  const controller = new AbortController();
  let timer, abort;
  const stopped = new Promise((_, reject) => {
    abort = () => { controller.abort(); reject(new DOMException('读取已取消', 'AbortError')); };
    if (signal.aborted) { abort(); return; }
    signal.addEventListener('abort', abort, { once: true });
    timer = setTimeout(() => {
      controller.abort();
      reject(Object.assign(new Error('等待响应超时'), { status: 0 }));
    }, timeout);
  });
  try {
    return await Promise.race([stopped, Promise.resolve().then(() => {
      if (controller.signal.aborted) throw new DOMException('读取已取消', 'AbortError');
      return read(controller.signal);
    })]);
  } finally { clearTimeout(timer); signal.removeEventListener('abort', abort); }
}

export const postExecution = (request, signal) => boundedRequest((signal) => apiPost('/api/executions', request, { signal }), signal);
export const readReceipt = (id, signal) => boundedRequest((signal) => apiGet(`/api/executions/${encodeURIComponent(id)}/receipt`, { signal }), signal, 10000);

export function acceptanceOf(result, id) {
  if (result?.id !== id || result.kind !== 'team') throw new Error('服务端未返回匹配的 Team 执行编号');
  return result;
}

export function receiptOutcome(result, id) {
  if (result?.id !== id || !['prepared', 'pending', 'accepted', 'rejected'].includes(result.phase)) throw new Error('受理结果格式无效');
  if (result.phase === 'accepted') return acceptanceOf(result.receipt?.body, id);
  if (result.phase === 'rejected') {
    throw Object.assign(new Error(result.receipt?.body?.error || '服务端拒绝启动'), { status: result.receipt?.status || 400, rejected: true });
  }
  return null;
}
