import { apiPost } from '../../api.js';

// Keep the same receipt identity across a lost HTTP response, including reloads.
const pending = new Map();
const inflight = new Map();
export function submitAttempt(todoId, action, input = {}, path) {
  const key = `project-attempt:${todoId}:${action}:${JSON.stringify(input)}`;
  if (inflight.has(key)) return inflight.get(key);
  let runId = pending.get(key) || sessionStorage.getItem(key);
  if (!runId) runId = `prun-${crypto.randomUUID()}`;
  pending.set(key, runId);
  sessionStorage.setItem(key, runId);
  const request = path
    ? { action, input: { ...input, run_id: runId } }
    : { ...input, run_id: runId };
  const operation = apiPost(path || `/api/project/todos/${encodeURIComponent(todoId)}/${action}`, request)
    .then((receipt) => {
      pending.delete(key); sessionStorage.removeItem(key);
      return receipt;
    }).finally(() => inflight.delete(key));
  inflight.set(key, operation);
  return operation;
}
