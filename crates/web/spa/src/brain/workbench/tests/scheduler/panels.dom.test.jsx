// @vitest-environment jsdom
import '../../../../test/setup-dom.js';
import { afterEach, expect, it, vi } from 'vitest';
import { useEffect, useState } from 'react';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { ExecutionDrawer } from '../../scheduler/detail.jsx';
import { RoundsPanel } from '../../scheduler/rounds.jsx';
const { mounted, unmounted } = vi.hoisted(() => ({ mounted: vi.fn(), unmounted: vi.fn() }));
vi.mock('../../../../fleet/detail.jsx', () => ({ ExecutionView: function Panel({ executionRef, managed }) {
  const [filter, setFilter] = useState('');
  useEffect(() => { mounted(executionRef); return () => unmounted(executionRef); }, []);
  return <div><span>{`panel:${executionRef.kind}:${executionRef.id}:${managed}`}</span><input aria-label="明细内部筛选" value={filter} onChange={(event) => setFilter(event.target.value)} /></div>;
} }));
afterEach(() => { cleanup(); vi.clearAllMocks(); });
const kinds = ['agent', 'team', 'dag', 'todos', 'operator'];
const operations = kinds.map((kind, i) => ({ operation_id: `op-${i}`, execution_id: `${kind}-1`, execution_kind: kind, capability_id: `cap-${kind}`, round: 1, status: 'done', execution_created: true }));
const view = { run: { round: 1 }, capabilities: [], rounds: [{ round: 1, status: 'completed', operations }] };
it('reuses the type-dispatched execution panel and remounts subscriptions and state for all five types', () => {
  const props = { view, onSelect: vi.fn(), onClose: vi.fn() };
  const result = render(<ExecutionDrawer {...props} operation={operations[0]} />);
  for (let i = 0; i < kinds.length; i++) {
    result.rerender(<ExecutionDrawer {...props} operation={operations[i]} />);
    expect(screen.getByText(`panel:${kinds[i]}:${kinds[i]}-1:true`)).toBeTruthy();
    expect(screen.getByLabelText('明细内部筛选').value).toBe('');
    fireEvent.change(screen.getByLabelText('明细内部筛选'), { target: { value: 'old state' } });
  }
  expect(mounted).toHaveBeenCalledTimes(5); expect(unmounted).toHaveBeenCalledTimes(4);
});
it('keeps earlier rounds expanded when a new round arrives and disables uncreated execution IDs', () => {
  const open = vi.fn(); const result = render(<RoundsPanel view={view} onOpen={open} />);
  fireEvent.click(screen.getByRole('button', { name: '查看执行 agent-1' })); expect(open).toHaveBeenCalledWith(operations[0]);
  const pending = { ...operations[0], operation_id: 'pending', execution_id: 'agent-2', round: 2, status: 'creating', execution_created: false };
  result.rerender(<RoundsPanel view={{ ...view, run: { round: 2 }, rounds: [...view.rounds, { round: 2, status: 'running', operations: [pending] }] }} onOpen={open} />);
  expect(screen.getByRole('button', { name: '查看执行 agent-1' }).disabled).toBe(false);
  expect(screen.getByRole('button', { name: '查看执行 agent-2' }).disabled).toBe(true);
});
