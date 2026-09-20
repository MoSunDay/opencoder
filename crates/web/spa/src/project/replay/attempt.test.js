// @vitest-environment jsdom
import { afterEach, expect, it, vi } from 'vitest';
import { apiPost } from '../../api.js';
vi.mock('../../api.js', () => ({ apiPost: vi.fn() }));
afterEach(() => { sessionStorage.clear(); vi.unstubAllGlobals(); vi.resetAllMocks(); });

it('retries the same persisted project run after reload without randomUUID', async () => {
  vi.stubGlobal('crypto', { getRandomValues: crypto.getRandomValues.bind(crypto) });
  const { submitAttempt } = await import('./attempt.js');
  apiPost.mockRejectedValueOnce(new Error('connection lost'));
  await expect(submitAttempt('http-plan', 'plan', { prompt: 'review' })).rejects.toThrow('connection lost');
  const first = apiPost.mock.calls[0][1];
  expect(first.run_id).toMatch(/^prun-[a-f0-9]{32}$/);
  vi.resetModules();
  const reloaded = await import('./attempt.js');
  apiPost.mockResolvedValueOnce({ accepted: true });
  await expect(reloaded.submitAttempt('http-plan', 'plan', { prompt: 'review' })).resolves.toEqual({ accepted: true });
  expect(apiPost.mock.calls[1][1]).toEqual(first);
  expect(sessionStorage.length).toBe(0);
});
