// @vitest-environment jsdom
import '../../../test/setup-dom.js';
import { act, renderHook } from '@testing-library/react';
import { afterEach, expect, it } from 'vitest';
import { useDraft } from '../scheduler/draft.js';
import { changeDraft } from '../scheduler/changes.js';
import { version } from '../milestone/canvas/fixtures.js';
afterEach(() => localStorage.clear());

it('queued callbacks and an immediate close persist the latest complete draft', () => {
  const { result } = renderHook(() => useDraft('latest', version));
  const first = result.current;
  act(() => {
    first.setDraft((draft) => changeDraft(draft, { type: 'layer', id: 'coding', patch: { task: '刚修改的任务' } }));
    first.setDraft((draft) => changeDraft(draft, { type: 'remove', items: [{ type: 'node', id: 'code-a' }] }));
    expect(first.persist()).toBe(true);
    const saved = JSON.parse(localStorage.getItem('latest'));
    expect(saved.version.plan.layers[0].task).toBe('刚修改的任务');
    expect(saved.version.plan.nodes.map((n) => n.node_id)).toEqual(['code-b', 'test-a']);
  });
  expect(result.current.draft.version.plan.nodes).toHaveLength(2);
});

it('rejected edits leave the current draft and storage unchanged', () => {
  const { result } = renderHook(() => useDraft('failed', version));
  const before = localStorage.getItem('failed');
  act(() => expect(() => result.current.setDraft((draft) => changeDraft(draft, { type: 'add_node', id: 'n', layerId: 'gone' }))).toThrow('不存在'));
  expect(result.current.draft.version.plan.nodes).toHaveLength(3);
  expect(localStorage.getItem('failed')).toBe(before);
});
