import { expect, it } from 'vitest';
import { matchesSearch, readTokens, readUsers, tokenStatus } from './model.js';
it('matches trimmed case-insensitive terms across names and identifiers', () => {
  expect(matchesSearch(' ALICE ', 'CLI', 'alice')).toBe(true);
  expect(matchesSearch('', undefined)).toBe(true);
  expect(matchesSearch('missing', 'CLI', 'alice')).toBe(false);
});
it('classifies token expiry at the boundary and gives revocation precedence', () => {
  expect(tokenStatus({ expires_at: null }, 100).value).toBe('active');
  expect(tokenStatus({ expires_at: 101 }, 100).value).toBe('active');
  expect(tokenStatus({ expires_at: 100 }, 100).value).toBe('expired');
  expect(tokenStatus({ expires_at: 0, revoked_at: 0 }, 100).value).toBe('revoked');
});
it('distinguishes empty directories from malformed responses', () => {
  expect(readUsers({ users: [] })).toEqual([]);
  expect(readTokens({ tokens: [] })).toEqual([]);
  expect(() => readUsers({})).toThrow('用户列表格式错误');
  expect(() => readTokens(null)).toThrow('Token 列表格式错误');
});
