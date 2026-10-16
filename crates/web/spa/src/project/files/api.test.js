import { expect, it, vi } from 'vitest';
const authFetch = vi.hoisted(() => vi.fn());
vi.mock('../../api.js', () => ({ authFetch }));
import { fileUrl, uploadFile } from './api.js';
it('encodes relative paths and uploads raw bytes with authenticated fetch', async () => {
  expect(fileUrl('t', '日志/a b')).toContain('path=%E6%97%A5%E5%BF%97%2Fa+b');
  const file = new Blob(['bytes']);
  authFetch.mockResolvedValueOnce({ ok: true, json: async () => ({ bytes: 5 }) });
  expect(await uploadFile('t', 'a', file)).toEqual({ bytes: 5 });
  expect(authFetch.mock.calls[0][3]).toMatchObject({ rawBody: file, contentType: 'application/octet-stream' });
  authFetch.mockResolvedValueOnce({ ok: false, status: 409, json: async () => ({ error: 'duplicate' }) });
  await expect(uploadFile('t', 'a', file)).rejects.toThrow('duplicate');
});
