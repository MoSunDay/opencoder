import { newId } from '../fleet/model.js';

// Keep the same id across a lost acknowledgement; reset only after success.
export function prepareInput(previous, executionId, action, input, allocate = () => newId('input')) {
  const signature = JSON.stringify([executionId, action, input]);
  const attempt = previous?.signature === signature ? previous : { signature, id: allocate() };
  return { attempt, input: { ...input, input_id: attempt.id } };
}

export async function postSessionInput(post, reference, sessionId, input) {
  const prepared = prepareInput(reference.current, sessionId, input.delivery, input);
  reference.current = prepared.attempt;
  const reply = await post(`/api/sessions/${encodeURIComponent(sessionId)}/prompt`, prepared.input);
  if (reply?.ok === false) throw new Error(reply.error || '输入被拒绝');
  if (reference.current === prepared.attempt) reference.current = null;
  return reply;
}
