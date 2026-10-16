// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import '../../test/setup-dom.js';
const api = vi.hoisted(() => ({ apiGet: vi.fn(), apiPatch: vi.fn(), apiPost: vi.fn(), apiDel: vi.fn() }));
const files = vi.hoisted(() => ({ downloadFile: vi.fn(), uploadFile: vi.fn(), fileUrl: vi.fn() }));
vi.mock('../../api.js', () => api);
vi.mock('../files/api.js', () => files);
vi.mock('../../fleet/detail.jsx', () => ({ ExecutionView: () => <div>原生执行</div> }));
import { TodoDrawer } from '../todoDrawer.jsx';
import { setIdentity } from '../../store.js';

const overview = { goals: [], standalone_initiatives: [], tags: [], backlog: [{ id: 't', title: '阅读任务', draft: '# 交付计划\n\n**验收标准**', board_status: 'todo' }] };
const props = () => ({ todoId: 't', overview, refresh: vi.fn(), onNotice: vi.fn(), onClose: vi.fn() });
beforeEach(() => {
  vi.resetAllMocks(); setIdentity({ name: 'alice', role: 'editor' });
  api.apiGet.mockImplementation(async (path) => path.endsWith('/people') ? { people: [] }
    : path.includes('/comments') ? { comments: [], has_more: false } : { capabilities: [], assignments: [] });
  api.apiPatch.mockResolvedValue({});
});

it('shows inline comments with the task and only reads execution records when selected', async () => {
  render(<TodoDrawer {...props()} />);
  expect(await screen.findByRole('heading', { name: '交付计划' })).toBeTruthy();
  expect(screen.getByText('验收标准').tagName).toBe('STRONG');
  expect(screen.queryByRole('button', { name: '编辑任务' })).toBeNull();
  expect(screen.queryByRole('textbox', { name: '关键信息' })).toBeNull();
  expect(screen.getAllByRole('tab')).toHaveLength(4);
  expect(screen.queryByRole('tab', { name: '评论' })).toBeNull();
  const heading = screen.getByLabelText('TODO 标题').closest('.project-task-heading');
  expect(within(heading).getByRole('combobox', { name: 'TODO 看板列' })).toBeTruthy();
  expect(screen.getByRole('tab', { name: '任务', exact: true }).textContent).toBe('任务');
  expect(await screen.findByText('暂无评论')).toBeTruthy();
  expect(api.apiGet.mock.calls.some(([path]) => path.endsWith('/executions'))).toBe(false);
  fireEvent.click(screen.getByRole('tab', { name: '执行', exact: true }));
  await waitFor(() => expect(api.apiGet.mock.calls.some(([path]) => path.endsWith('/executions'))).toBe(true));
  fireEvent.click(screen.getByRole('tab', { name: '任务', exact: true }));
  expect(await screen.findByText('暂无评论')).toBeTruthy();
  expect(api.apiPatch).not.toHaveBeenCalled(); expect(api.apiPost).not.toHaveBeenCalled();
});

it('edits status beside the task title and persists both through the footer save', async () => {
  render(<TodoDrawer {...props()} />);
  const task = screen.getByRole('tabpanel', { name: '任务', exact: true });
  fireEvent.change(within(task).getByLabelText('TODO 标题'), { target: { value: '更新标题和状态' } });
  fireEvent.mouseDown(within(task).getByRole('combobox', { name: 'TODO 看板列' }));
  fireEvent.click(await screen.findByText('进行中', { selector: '.ant-select-item-option-content' }));
  fireEvent.click(screen.getByRole('tab', { name: '设置', exact: true }));
  expect(within(screen.getByRole('tabpanel', { name: '设置', exact: true })).queryByLabelText('TODO 看板列')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: '保存 TODO' }));
  await waitFor(() => expect(api.apiPatch).toHaveBeenCalledWith('/api/project/todos/t', expect.objectContaining({ title: '更新标题和状态', board_status: 'in_progress' })));
});

it('preserves task edits and previews across tabs and saves untouched settings together', async () => {
  const p = props(); const view = render(<TodoDrawer {...p} />);
  fireEvent.click(within(screen.getByLabelText('关键信息显示方式')).getByText('编辑'));
  fireEvent.change(screen.getByLabelText('关键信息'), { target: { value: '## 草稿\n\n**完整内容**' } });
  fireEvent.click(within(screen.getByLabelText('关键信息显示方式')).getByText('预览'));
  expect(within(screen.getByLabelText('关键信息预览')).getByText('完整内容').tagName).toBe('STRONG');
  expect(api.apiPatch).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('tab', { name: '设置', exact: true }));
  view.rerender(<TodoDrawer {...p} overview={{ ...overview, backlog: [{ ...overview.backlog[0], draft: '远端文本' }] }} />);
  fireEvent.click(screen.getByRole('button', { name: '保存 TODO' }));
  await waitFor(() => expect(api.apiPatch).toHaveBeenCalledWith('/api/project/todos/t', expect.objectContaining({
    title: '阅读任务', draft: '## 草稿\n\n**完整内容**', board_status: 'todo', owner_user_name: null, follower_user_names: [], capability_id: null,
  })));
});

it('opens and focuses a hidden invalid task field when saving from settings', async () => {
  render(<TodoDrawer {...props()} />);
  fireEvent.click(within(screen.getByLabelText('关键信息显示方式')).getByText('编辑'));
  fireEvent.change(screen.getByLabelText('TODO 标题'), { target: { value: ' ' } });
  fireEvent.click(screen.getByRole('tab', { name: '设置', exact: true }));
  fireEvent.click(screen.getByRole('button', { name: '保存 TODO' }));
  expect(await screen.findByText('TODO 标题不能为空')).toBeTruthy();
  await waitFor(() => expect(document.activeElement).toBe(screen.getByLabelText('TODO 标题')));
  expect(screen.getByRole('tab', { name: '任务', exact: true }).getAttribute('aria-selected')).toBe('true');
  expect(api.apiPatch).not.toHaveBeenCalled();
});

it('asks before discarding a comment draft and keeps it when continuing to edit', async () => {
  const p = props(); render(<TodoDrawer {...p} />);
  fireEvent.click(screen.getByRole('tab', { name: '任务', exact: true }));
  fireEvent.change(screen.getByLabelText('评论正文'), { target: { value: '未提交的补充' } });
  fireEvent.click(document.querySelector('.ant-drawer-close'));
  const confirm = (await screen.findByText('放弃未保存的内容？', { selector: '.ant-modal-confirm-title' })).closest('[role="dialog"]');
  fireEvent.click(within(confirm).getByRole('button', { name: '继续编辑' }));
  expect(screen.getByLabelText('评论正文').value).toBe('未提交的补充');
  expect(screen.getByRole('heading', { name: '交付计划' })).toBeTruthy();
  expect(p.onClose).not.toHaveBeenCalled();
  fireEvent.click(document.querySelector('.ant-drawer-close'));
  const next = await waitFor(() => {
    const dialog = screen.getAllByText('放弃未保存的内容？', { selector: '.ant-modal-confirm-title' })
      .map((node) => node.closest('[role="dialog"]')).find((node) => node !== confirm);
    expect(dialog).toBeTruthy(); return dialog;
  });
  fireEvent.click(within(next).getByRole('button', { name: '放弃修改' }));
  await waitFor(() => expect(p.onClose).toHaveBeenCalledOnce());
});

it('saves the task and inline comment independently while preserving the other draft', async () => {
  const p = props(); render(<TodoDrawer {...p} />);
  fireEvent.change(screen.getByLabelText('TODO 标题'), { target: { value: '保存的标题' } });
  fireEvent.change(screen.getByLabelText('评论正文'), { target: { value: '讨论草稿' } });
  fireEvent.click(screen.getByRole('button', { name: '保存 TODO' }));
  await waitFor(() => expect(p.refresh).toHaveBeenCalledOnce());
  expect(screen.getByLabelText('评论正文').value).toBe('讨论草稿');
  expect(api.apiPost).not.toHaveBeenCalled();
  expect(api.apiPatch.mock.calls[0][1]).not.toHaveProperty('body_md');
  fireEvent.change(screen.getByLabelText('TODO 标题'), { target: { value: '仍未保存的标题' } });
  api.apiPost.mockResolvedValue({ seq: 1, body_md: '讨论草稿', author_name: 'alice', author_kind: 'user', created_at: 1 });
  fireEvent.click(screen.getByRole('button', { name: '发表评论' }));
  await waitFor(() => expect(p.refresh).toHaveBeenCalledTimes(2));
  expect(api.apiPost).toHaveBeenCalledWith('/api/project/todos/t/comments', expect.objectContaining({ body_md: '讨论草稿' }));
  expect(api.apiPatch).toHaveBeenCalledOnce();
  expect(screen.getByLabelText('TODO 标题').value).toBe('仍未保存的标题');
  expect(screen.getByLabelText('评论正文').value).toBe('');
});

it('opens an inline comment source in execution and retains the task draft', async () => {
  api.apiGet.mockImplementation(async (path) => path.includes('/comments') ? { comments: [{ seq: 1, body_md: '执行补充', author_name: 'alice', author_kind: 'user', execution_id: 'dag-source', created_at: 1 }], has_more: false }
    : path.endsWith('/index') ? { id: 'dag-source', kind: 'dag', status: 'done' }
    : path.endsWith('/people') ? { people: [] } : { capabilities: [], assignments: [] });
  render(<TodoDrawer {...props()} />);
  fireEvent.change(screen.getByLabelText('TODO 标题'), { target: { value: '未保存的任务标题' } });
  fireEvent.click(await screen.findByRole('button', { name: '查看执行来源' }));
  expect(await screen.findByText('原生执行')).toBeTruthy();
  expect(screen.getByRole('tab', { name: '执行', exact: true }).getAttribute('aria-selected')).toBe('true');
  fireEvent.click(screen.getByRole('tab', { name: '任务', exact: true }));
  expect(screen.getByLabelText('TODO 标题').value).toBe('未保存的任务标题');
  expect(api.apiPatch).not.toHaveBeenCalled();
});

it('keeps the drawer open until a download finishes even if a concurrent comment completes', async () => {
  let finishDownload;
  files.downloadFile.mockImplementation(() => new Promise((resolve) => { finishDownload = resolve; }));
  api.apiGet.mockImplementation(async (path) => path.endsWith('/files')
    ? { directory: '2026-10-16/t', max_file_bytes: 100, files: [{ path: 'notes.md', bytes: 4, modified_at: 1 }] }
    : path.endsWith('/people') ? { people: [] }
    : path.includes('/comments') ? { comments: [], has_more: false } : { capabilities: [], assignments: [] });
  api.apiPost.mockResolvedValue({ seq: 1, body_md: '已同步', author_name: 'alice', author_kind: 'user', created_at: 1 });
  const p = props(); render(<TodoDrawer {...p} />);
  fireEvent.change(screen.getByLabelText('评论正文'), { target: { value: '已同步' } });
  fireEvent.click(screen.getByRole('tab', { name: '文件', exact: true }));
  await screen.findByText('notes.md');
  const download = within(screen.getByLabelText('任务文件')).getByText(/下\s*载/, { selector: 'button span' }).closest('button');
  fireEvent.click(download);
  expect(files.downloadFile).toHaveBeenCalledOnce();
  fireEvent.click(screen.getByText('任务', { selector: '[role=tab]' }));
  fireEvent.click(screen.getByText('发表评论').closest('button'));
  await waitFor(() => expect(screen.getByLabelText('评论正文').value).toBe(''));
  expect(screen.getByText('保存 TODO').closest('button').disabled).toBe(true);
  fireEvent.click(document.querySelector('.ant-drawer-close'));
  expect(p.onClose).not.toHaveBeenCalled();
  finishDownload();
  await waitFor(() => expect(screen.getByText('保存 TODO').closest('button').disabled).toBe(false));
  fireEvent.click(document.querySelector('.ant-drawer-close'));
  expect(p.onClose).toHaveBeenCalledOnce();
});
