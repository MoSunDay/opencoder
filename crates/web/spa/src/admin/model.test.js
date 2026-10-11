import { expect, it } from 'vitest';
import { matchesSearch, readUsers } from './model.js';
it('matches trimmed case-insensitive terms across names and identifiers', () => {
  expect(matchesSearch(' ALICE ', 'CLI', 'alice')).toBe(true);
  expect(matchesSearch('', undefined)).toBe(true);
  expect(matchesSearch('missing', 'CLI', 'alice')).toBe(false);
});
it('distinguishes empty directories from malformed responses', () => {
  expect(readUsers({ users: [] })).toEqual([]);
  expect(() => readUsers({})).toThrow('用户列表格式错误');
});
