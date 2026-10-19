// @vitest-environment jsdom
import '../../../../test/setup-dom.js';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { useState } from 'react';
import { ReadingTabs } from '../../../../ui/runContent/tabs.jsx';
import { Rounds } from './rounds.jsx';
afterEach(cleanup);
function Tabs() {
  const [key, setKey] = useState('a');
  return <div className="brain-detail-scroll"><ReadingTabs activeKey={key} onChange={setKey} items={[
    { key: 'a', label: '第一项', children: '内容一' }, { key: 'b', label: '第二项', children: '内容二' },
  ]} /></div>;
}
it('切换页签恢复各自滚动位置', () => {
  const { container } = render(<Tabs />);
  const scroller = container.querySelector('.brain-detail-scroll');
  scroller.scrollTop = 240;
  fireEvent.click(screen.getByRole('tab', { name: '第二项' }));
  expect(scroller.scrollTop).toBe(0);
  scroller.scrollTop = 80;
  fireEvent.click(screen.getByRole('tab', { name: '第一项' }));
  expect(scroller.scrollTop).toBe(240);
  fireEvent.click(screen.getByRole('tab', { name: '第二项' }));
  expect(scroller.scrollTop).toBe(80);
});
it('后台推进轮次后保留用户选中的历史轮次', () => {
  const plan = { layers: [{ layer_id: 'a', title: '第一层' }], nodes: [] };
  const history = [1, 2].map((round) => ({ round, activation: round, layer: 1, operations: [] }));
  const view = { plan, run: { round: 2 }, events: [] };
  const { rerender } = render(<Rounds view={view} plan={plan} history={history} onExecution={vi.fn()} />);
  fireEvent.click(screen.getByText('第 1 轮 · 1 层 · 0 项执行'));
  rerender(<Rounds view={{ ...view, run: { round: 3 } }} plan={plan} history={[...history, { round: 3, activation: 3, layer: 1, operations: [] }]} />);
  expect(screen.getByText('第 1 轮 · 1 层 · 0 项执行').closest('button').getAttribute('aria-pressed')).toBe('true');
  expect(screen.getByText('第 3 轮 · 1 层 · 0 项执行').closest('button').getAttribute('aria-pressed')).toBe('false');
});
