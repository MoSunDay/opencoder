// @vitest-environment jsdom
import { expect, it } from 'vitest';
import { createDraft, repairPlan, submission } from '../editor/model.js';
import { draftKey, readDraft } from '../editor/draft.js';
import { graph } from '../model.js';
it('separates user, server and plan-version drafts and rejects damaged cache without overwriting it', () => {
  expect(draftKey('server:a')).not.toBe(draftKey('server:b'));
  expect(draftKey('a', { id: 'p', version: 1 })).not.toBe(draftKey('a', { id: 'p', version: 2 }));
  const key = draftKey('bad'); localStorage.setItem(key, '{bad');
  expect(() => readDraft(key)).toThrow();
  expect(localStorage.getItem(key)).toBe('{bad');
});
it('projects library entities, actions and explicit return edges without flattening the loop', () => {
  const capabilities = [{ id: 'cap', kind: 'agent', target: 'act', summary: '执行实体' }];
  const plan = repairPlan(capabilities); const view = graph(plan, [], true, capabilities);
  expect(view.nodes.some((n) => n.id === 'route:after-verify')).toBe(true);
  expect(view.nodes.filter((n) => n.data.step)).toHaveLength(2);
  expect(view.edges).toContainEqual(expect.objectContaining({ source: 'route:after-verify', target: 'input:feedback' }));
  const draft = createDraft(); draft.version.plan = plan; draft.metadata = { title: ' 修复 ', summary: ' 复测后发布 ' };
  expect(submission(draft, capabilities).plan.title).toBe('修复');
  expect(() => submission(draft, [])).toThrow('注册能力不可用');
});
