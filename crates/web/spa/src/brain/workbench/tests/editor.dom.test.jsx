// @vitest-environment jsdom
import '../../../test/setup-dom.js';
import { afterEach, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { PlanEditor } from '../editor.jsx';
import { repairPlan } from '../editor/model.js';
import { apiPost } from '../../../api.js';
vi.mock('../../../api.js', () => ({ apiPost: vi.fn() }));
vi.mock('../canvas.jsx', () => ({ PlanCanvas: () => <div>画布</div> }));
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
