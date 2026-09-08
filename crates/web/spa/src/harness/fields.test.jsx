// @vitest-environment jsdom
import { expect, it } from 'vitest';
import { parseEnvs } from './fields.jsx';

it('preserves values, handles empty values and resolves repeated keys', () => {
  expect({ ...parseEnvs('A= x=y \nB=\nA=final\n\n') }).toEqual({ A: 'final', B: '' });
  expect(parseEnvs('__proto__=literal')['__proto__']).toBe('literal');
});
it('rejects malformed environment lines before submission', () => {
  for (const text of ['A', '=value', 'A=x\0']) { expect(() => parseEnvs(text)).toThrow(); }
});
