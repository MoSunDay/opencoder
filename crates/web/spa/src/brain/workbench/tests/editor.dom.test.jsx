// @vitest-environment jsdom
import '../../../test/setup-dom.js';
import { afterEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { PlanEditor } from '../history/editor.jsx';
import { draftKey, legacyDraftKey } from '../history/editor/draft.js';
import { repairPlan } from '../history/editor/model.js';
import { apiPost } from '../../../api.js';
vi.mock('../../../api.js', () => ({ apiPost: vi.fn() }));
vi.mock('../history/canvas.jsx', () => ({ PlanCanvas: () => <div>画布</div> }));
afterEach(() => { cleanup(); localStorage.clear(); vi.resetAllMocks(); });
it('blocks saving invalid JSON instead of submitting a stale valid binding', async () => {
  const plan = repairPlan([{ id: 'act', kind: 'agent', target: 'act', summary: '执行' }]);
  render(<PlanEditor version={{ id: 'review', version: 1, plan }} capabilities={[]} onSaved={vi.fn()} onClose={vi.fn()} />);
  fireEvent.click(screen.getByRole('tab', { name: '完整契约' }));
  fireEvent.change(screen.getByLabelText('计划 JSON'), { target: { value: '{broken' } });
  const save = screen.getByRole('button', { name: '提交计划' });
  await waitFor(() => expect(save.disabled).toBe(true));
  fireEvent.click(save); expect(apiPost).not.toHaveBeenCalled();
  fireEvent.change(screen.getByLabelText('计划 JSON'), { target: { value: JSON.stringify(plan) } });
  await waitFor(() => expect(save.disabled).toBe(false));
});

it('recovers the blocked editor from a legacy v1 draft cache via discard', () => {
  const key = draftKey('editor-user');
  const v1 = { version: { id: 'p1', version: 2, created_at: 1, plan: { schema_version: 1, steps: [{ id: 's1' }], inputs: {}, outputs: {} } }, selected: null, positions: {}, viewport: null, raw: {}, metadata: { title: '旧', summary: '' } };
  const raw = JSON.stringify(v1);
  localStorage.setItem(key, raw);
  render(<PlanEditor cacheKey={key} capabilities={[]} onSaved={vi.fn()} onClose={vi.fn()} />);
  expect(screen.getByText('无法读取浏览器草稿')).toBeTruthy();
  expect(screen.getByText(/旧版协议/)).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: '丢弃缓存并重新开始' }));
  // 编辑器恢复渲染：完整契约 tab 可用，缓存换新为合法 v2 空草稿，原文进入 legacy 备份槽。
  expect(screen.getByRole('tab', { name: '完整契约' })).toBeTruthy();
  expect(JSON.parse(localStorage.getItem(key)).version.plan.schema_version).toBe(2);
  expect(localStorage.getItem(legacyDraftKey(key))).toBe(raw);
});
