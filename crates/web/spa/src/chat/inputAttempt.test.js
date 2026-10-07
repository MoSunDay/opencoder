import { describe, expect, it, vi } from 'vitest';
import { prepareInput, postSessionInput } from './inputAttempt.js';

describe('shared execution input ids', () => {
  it('retries the same message once and separates new messages and sessions', () => {
    const first = prepareInput(null, 'agent-1', 'steer', { prompt: 'continue' }, () => 'first');
    const retry = prepareInput(first.attempt, 'agent-1', 'steer', { prompt: 'continue' }, () => 'unwanted');
    expect(retry.input.input_id).toBe('first');
    expect(prepareInput(first.attempt, 'agent-2', 'steer', { prompt: 'continue' }, () => 'second').input.input_id).toBe('second');
    expect(prepareInput(null, 'agent-1', 'steer', { prompt: 'continue' }, () => 'next').input.input_id).toBe('next');
  });
});

it('retains a lost input acknowledgement and clears only the successful attempt', async () => {
  const ref = { current: null };
  const post = vi.fn().mockRejectedValueOnce(new Error('lost reply')).mockResolvedValue({ ok: true });
  const input = { prompt: 'continue', delivery: 'steer' };
  await expect(postSessionInput(post, ref, 'agent-1', input)).rejects.toThrow('lost reply');
  const pending = ref.current;
  await postSessionInput(post, ref, 'agent-1', input);
  expect(post.mock.calls[0]).toEqual(post.mock.calls[1]);
  expect(post.mock.calls[1][1].input_id).toBe(pending.id);
  expect(ref.current).toBeNull();
});
