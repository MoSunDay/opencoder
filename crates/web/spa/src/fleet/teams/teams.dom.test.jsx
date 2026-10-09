// @vitest-environment jsdom
import '../../test/setup-dom.js';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { FleetTeamsPanel } from '../teams.jsx';
import { TeamExecutionHistory } from './history.jsx';
import { apiGet, apiPost } from '../../api.js';
import { newId } from '../model.js';
vi.mock('../model.js', async (original) => ({ ...(await original()), newId: vi.fn() }));
vi.mock('../../api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn() }));
vi.mock('../detail.jsx', () => ({ ExecutionDetail: ({ id, onClose }) => <div>执行过程:{id}<button onClick={onClose}>关闭过程</button></div> }));
afterEach(() => { cleanup(); vi.resetAllMocks(); vi.useRealTimers(); });
const run = { id: 'team-run', kind: 'team', name: 'release', status: 'running', node_id: 'n1', created_at: 10 };
const setup = () => {
  newId.mockReturnValueOnce(run.id).mockReturnValue('unexpected-second-run');
  apiGet.mockImplementation(async (path) => {
    if (path === '/api/teams') return { teams: [{ name: 'release', captain: 'lead', members: [{ agent: 'lead' }] }] };
    if (path === '/api/nodes') return { nodes: [{ id: 'n1', name: 'worker', online: true, kinds: ['team'], snapshot: { ready: true } }] };
    if (path === '/api/brain/agents') return { agents: [] };
    if (path.endsWith('/receipt')) return { id: 'team-run', phase: 'prepared', receipt: null };
    return { executions: [run] };
  });
};
async function launch() {
  fireEvent.click(await screen.findByText('启动 Team'));
  const drawer = within(await screen.findByRole('dialog', { name: '启动 release' }));
  fireEvent.change(drawer.getByLabelText('任务要求'), { target: { value: '检查方案' } });
  return drawer.getByRole('button', { name: /^启\s*动$/ });
}

describe('Team 列表与执行记录', () => {
  it('opens retained history without starting another run and can reopen its process', async () => {
    setup(); render(<FleetTeamsPanel />);
    fireEvent.click(screen.getByRole('tab', { name: '执行记录' }));
    fireEvent.click(await screen.findByRole('link', { name: run.id }));
    expect(screen.getByText('执行过程:team-run')).toBeTruthy();
    fireEvent.click(screen.getByText('关闭过程'));
    fireEvent.click(screen.getByRole('link', { name: run.id }));
    expect(screen.getByText('执行过程:team-run')).toBeTruthy();
    expect(apiGet).toHaveBeenCalledWith('/api/executions?limit=50&kind=team', expect.objectContaining({ signal: expect.any(AbortSignal) }));
    expect(apiPost).not.toHaveBeenCalled();
  });
  it('acknowledges acceptance, opens the process and leaves history selected when it closes', async () => {
    setup(); let accept;
    apiPost.mockImplementation(() => new Promise((resolve) => { accept = resolve; }));
    render(<FleetTeamsPanel />);
    const submit = await launch();
    fireEvent.click(submit); fireEvent.click(submit);
    await waitFor(() => expect(apiPost).toHaveBeenCalledTimes(1));
    expect(screen.getByText('正在提交；若响应较慢，15 秒后会自动查询启动结果。')).toBeTruthy();
    await act(async () => { accept(run); });
    expect(await screen.findByText('Team 已启动')).toBeTruthy();
    expect(screen.getByText('执行过程:team-run')).toBeTruthy();
    expect(screen.getByRole('tab', { name: '执行记录' }).getAttribute('aria-selected')).toBe('true');
    fireEvent.click(screen.getByText('关闭过程'));
    expect(await screen.findByRole('link', { name: run.id })).toBeTruthy();
  });
  it('shows an uncertain submission inside the drawer and retries the same ID with the draft intact', async () => {
    setup(); apiPost.mockRejectedValueOnce(new Error('连接中断')).mockResolvedValue(run);
    render(<FleetTeamsPanel />);
    fireEvent.click(await launch());
    const drawer = within(screen.getByRole('dialog', { name: '启动 release' }));
    expect(await drawer.findByText('提交较慢，正在自动确认启动结果')).toBeTruthy();
    expect(drawer.getByLabelText('任务要求').value).toBe('检查方案');
    expect(drawer.getByRole('button', { name: '查看执行记录' })).toBeTruthy();
    fireEvent.click(drawer.getByRole('button', { name: '重试原请求' }));
    expect(await screen.findByText('Team 已启动')).toBeTruthy();
    expect(apiPost).toHaveBeenCalledTimes(2);
    expect(apiPost.mock.calls[1][1]).toEqual(apiPost.mock.calls[0][1]);
    expect(apiPost.mock.calls[0][1].id).toBe(run.id);
    expect(newId).toHaveBeenCalledTimes(1);
  });
  it('keeps whitespace-only requirements from dispatching', async () => {
    setup(); render(<FleetTeamsPanel />);
    await launch(); fireEvent.change(screen.getByLabelText('任务要求'), { target: { value: '   ' } });
    fireEvent.click(screen.getByRole('button', { name: /^启\s*动$/ }));
    expect(await screen.findByText('请填写任务要求')).toBeTruthy();
    expect(apiPost).not.toHaveBeenCalled();
  });
  it('paginates on the server and keeps errors retryable without presenting an empty success', async () => {
    apiGet.mockResolvedValueOnce({ executions: [run], next_cursor: { created_at: 10, id: run.id } })
      .mockRejectedValueOnce(new Error('读取失败')).mockResolvedValueOnce({ executions: [{ ...run, id: 'team-older' }] });
    render(<TeamExecutionHistory onOpen={vi.fn()} />);
    await screen.findByText(run.id);
    fireEvent.click(screen.getByText('下一页记录'));
    expect(await screen.findByText('读取失败')).toBeTruthy();
    expect(screen.queryByText('暂无 Team 执行记录')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: /^重\s*试$/ }));
    expect(await screen.findByText('team-older')).toBeTruthy();
    expect(apiGet).toHaveBeenLastCalledWith('/api/executions?limit=50&kind=team&cursor_created_at=10&cursor_id=team-run', expect.objectContaining({ signal: expect.any(AbortSignal) }));
  });
  it('cancels in-flight history reads when its host unmounts', async () => {
    let finish; apiGet.mockImplementation(() => new Promise((resolve) => { finish = resolve; }));
    const view = render(<TeamExecutionHistory onOpen={vi.fn()} />);
    const signal = apiGet.mock.calls[0][1].signal;
    view.unmount(); expect(signal.aborted).toBe(true);
    await act(async () => { finish({ executions: [run] }); });
    expect(screen.queryByText(run.id)).toBeNull();
  });
});
