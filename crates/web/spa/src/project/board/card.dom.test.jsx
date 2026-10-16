// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import '../../test/setup-dom.js';
import { TodoCard } from './card.jsx';
import { taskSummary } from './summary.js';

const todo = { id: 't', title: '交付可验收的任务', draft: '## 交付说明\n\n**完整方案**与[验收文档](https://example.com)', board_status: 'todo', owner_user_name: 'alice', tag_ids: ['tag'], latest_assignment: { execution_id: 'agent-1' } };
const overview = { goals: [], standalone_initiatives: [], tags: [{ id: 'tag', name: '前端', scope_type: 'global', scope_id: '' }] };
beforeEach(() => vi.clearAllMocks());

it('opens the task from its title and shows readable context without duplicate detail actions', () => {
  const onOpen = vi.fn();
  render(<TodoCard todo={todo} overview={overview} group="all" onOpen={onOpen} onDelete={vi.fn()} />);
  fireEvent.click(screen.getByRole('button', { name: todo.title, exact: true }));
  expect(onOpen).toHaveBeenCalledWith('t');
  expect(screen.getByText('交付说明 完整方案与验收文档')).toBeTruthy();
  expect(screen.getByText('alice')).toBeTruthy();
  expect(screen.getByText('已关联执行')).toBeTruthy();
  expect(screen.getByText('前端')).toBeTruthy();
  expect(screen.queryByRole('button', { name: '详情' })).toBeNull();
  expect(screen.queryByRole('button', { name: /^删\s*除$/ })).toBeNull();
  expect(screen.queryByRole('button', { name: `拖动 ${todo.title}` })).toBeNull();
  expect(screen.getByRole('button', { name: todo.title, exact: true }).getAttribute('aria-roledescription')).toBe('sortable');
});

it('confirms deletion from the compact menu and keeps cancellation harmless', async () => {
  const onDelete = vi.fn();
  render(<TodoCard todo={todo} overview={overview} group="all" onOpen={vi.fn()} onDelete={onDelete} />);
  fireEvent.click(screen.getByLabelText(`更多操作 ${todo.title}`));
  fireEvent.click(await screen.findByRole('menuitem', { name: /删除任务/ }));
  const first = (await screen.findByText('删除该 TODO？', { selector: '.ant-modal-confirm-title' })).closest('[role="dialog"]');
  fireEvent.click(within(first).getByRole('button', { name: /^取\s*消$/ }));
  expect(onDelete).not.toHaveBeenCalled();
  fireEvent.click(screen.getByLabelText(`更多操作 ${todo.title}`));
  fireEvent.click(await screen.findByRole('menuitem', { name: /删除任务/ }));
  const second = await waitFor(() => {
    const dialog = screen.getAllByText('删除该 TODO？', { selector: '.ant-modal-confirm-title' }).map((node) => node.closest('[role="dialog"]')).find((node) => node !== first);
    expect(dialog).toBeTruthy(); return dialog;
  });
  fireEvent.click(within(second).getByRole('button', { name: /^删\s*除$/ }));
  await waitFor(() => expect(onDelete).toHaveBeenCalledWith(todo));
});

it('keeps read-only cards browsable while disabling drag and destructive controls', () => {
  const onOpen = vi.fn();
  render(<TodoCard todo={{ ...todo, draft: '', owner_user_name: null, latest_assignment: null, tag_ids: [] }} overview={overview} group="all" onOpen={onOpen} onDelete={vi.fn()} disabled />);
  expect(screen.queryByRole('button', { name: `拖动 ${todo.title}` })).toBeNull();
  expect(screen.getByRole('button', { name: todo.title, exact: true }).hasAttribute('aria-roledescription')).toBe(false);
  expect(screen.getByLabelText(`更多操作 ${todo.title}`).disabled).toBe(true);
  expect(screen.getByText('未指定负责人')).toBeTruthy();
  expect(screen.queryByText('已关联执行')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: todo.title, exact: true }));
  expect(onOpen).toHaveBeenCalledWith('t');
});

it('summarizes lists and tables without Markdown syntax or link addresses', () => {
  expect(taskSummary('- [x] **接口完成**\n- [ ] [界面验收](https://example.com)')).toBe('接口完成 界面验收');
  expect(taskSummary('| 目标 | 结果 |\n| --- | --- |\n| 前端 | 可验收 |')).toBe('目标 结果 前端 可验收');
});

it('omits embedded HTML and bounds long summaries without rendering markup', () => {
  expect(taskSummary('<script>alert(1)</script>\n\n正常说明')).toBe('正常说明');
  expect(taskSummary('长内容'.repeat(200)).length).toBe(241);
  expect(taskSummary(null)).toBe('');
});
