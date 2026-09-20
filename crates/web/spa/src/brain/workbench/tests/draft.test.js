// @vitest-environment jsdom
import '../../../test/setup-dom.js';
import { expect, it } from 'vitest';
import { act } from 'react';
import { renderHook } from '@testing-library/react';
import { createDraft, repairPlan, submission } from '../history/editor/model.js';
import { draftKey, legacyDraftKey, readDraft, useDraft } from '../history/editor/draft.js';
import { graph } from '../history/model.js';
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

const legacyV1Draft = () => ({ version: { id: 'p1', version: 2, created_at: 1, plan: { schema_version: 1, title: '旧协议计划', steps: [{ id: 's1', label: '步骤' }], inputs: {}, outputs: {}, routes: [] } }, selected: 's1', positions: {}, viewport: null, raw: {}, metadata: { title: '旧协议计划', summary: '旧' } });
it('blocks legacy-protocol v1 cache with guidance and quarantines it on discard', () => {
  const key = draftKey('legacy-user');
  localStorage.setItem(key, JSON.stringify(legacyV1Draft()));
  expect(() => readDraft(key)).toThrow('旧版协议');
  const { result } = renderHook(() => useDraft(key, undefined));
  expect(result.current.draft).toBe(null);
  expect(result.current.error).toContain('旧版协议');
  act(() => result.current.discard());
  const raw = localStorage.getItem(legacyDraftKey(key));
  expect(JSON.parse(raw).version.plan.steps).toHaveLength(1); // 原文完整备份
  expect(result.current.draft.version.plan.schema_version).toBe(2);
  expect(result.current.error).toBe('');
  act(() => result.current.setDraft((d) => ({ ...d, metadata: { title: '新计划', summary: 'v2' } })));
  expect(result.current.persist()).toBe(true);
  expect(JSON.parse(localStorage.getItem(key)).metadata.title).toBe('新计划'); // 丢弃后可继续编辑并写回
});
it('keeps damaged cache untouched until discard and preserves it as backup', () => {
  const key = draftKey('damaged'); localStorage.setItem(key, '{bad');
  expect(() => readDraft(key)).toThrow('已损坏');
  expect(localStorage.getItem(key)).toBe('{bad');
  const { result } = renderHook(() => useDraft(key, undefined));
  act(() => result.current.discard());
  expect(localStorage.getItem(legacyDraftKey(key))).toBe('{bad');
  expect(JSON.parse(localStorage.getItem(key)).version.plan.schema_version).toBe(2);
});
