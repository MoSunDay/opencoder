import { expect, it } from 'vitest';
import { engineeringInputs, launchBody, newVersion, validatePlan } from '../scheduler/model.js';
import { readDraft } from '../scheduler/draft.js';
it('rejects duplicate input names and retains named JSON values', () => {
  expect(engineeringInputs([{ key: 'repo', value: 'r' }, { key: 'verified', value: 'true' }])).toEqual({ repo: 'r', verified: true });
  expect(() => engineeringInputs([{ key: 'repo', value: 'a' }, { key: ' repo ', value: 'b' }])).toThrow('重复');
  expect(() => engineeringInputs([{ key: '', value: 'a' }])).toThrow('不能为空');
  expect(engineeringInputs([{ key: '__proto__', value: '{"polluted":true}' }]).__proto__).toEqual({ polluted: true });
  expect({}.polluted).toBeUndefined();
});
it('requires an explicit usable capability scope', () => {
  const plan = { ...newVersion().plan, title: 'test', objective: 'test' };
  expect(() => validatePlan(plan, [])).toThrow('至少');
  expect(() => validatePlan({ ...plan, capability_ids: ['missing'] }, [])).toThrow('不可用');
});
it('new revisions preserve existing versions and saved launches reference exact versions', () => {
  const first = newVersion(); const next = newVersion(first);
  expect(next.version).toBe(2); expect(first.version).toBe(1);
  expect(launchBody({ node: 'n', engineering: [] }, 'brain-a', first)).toEqual({ schema_version: 3, id: 'brain-a', node_id: 'n', inputs: {}, plan: { id: first.id, version: 1 } });
});
it('rejects malformed v3 draft storage without replacing its original content', () => {
  const storage = { getItem: () => '{bad' };
  expect(() => readDraft('x', null, storage)).toThrow('原文已保留');
  expect(storage.getItem()).toBe('{bad');
});
