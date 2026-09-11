// @vitest-environment jsdom
import '../test/setup-dom.js';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { err } from '../notice.js';
import { ExecutionsPanel } from './executions.jsx';
import { ExecutionTranscript, appendEvent, messageRefreshMode } from './detail.jsx';
import { FleetNodesPanel } from './nodes.jsx';
import { FleetBrainPanel } from './brain.jsx';
import { FleetTeamsPanel } from './teams.jsx';
import { PayloadWindows, detailMarkers } from './detail/fields.jsx';
import { WorkloadDetail } from './detail/workloads.jsx';
import { CREATABLE_KINDS, KINDS, LARGE_MESSAGE_BYTES, appendMessagePage, executionActions, executionPagePath, newId, nodeOptions, textOf } from './model.js';
import { apiGet, apiPost } from '../api.js';
vi.mock('../api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn(), apiPut: vi.fn() }));
vi.mock('./detail.jsx', async (importOriginal) => ({
  ...(await importOriginal()),
  ExecutionDetail: ({ id }) => <div>execution-detail:{id}</div>,
}));
const node = { id: 'n1', name: 'worker', online: true, kinds: ['agent'], maintenance_agent_id: 'maintainer-n1', snapshot: { ready: true, cpu_capacity: 2, active_agent_loops: 3 } };
afterEach(() => { cleanup(); vi.resetAllMocks(); vi.unstubAllGlobals(); });
describe('fleet execution boundaries', () => {
  it('keeps the same execution ID when durable acceptance reply is lost', async () => {
    apiGet.mockImplementation(async (path) => path === '/api/nodes' ? { nodes: [node] } : { executions: [] });
    apiPost.mockRejectedValueOnce(new Error('connection lost')).mockImplementationOnce(async (_, body) => ({ id: body.id }));
    const onNotice = vi.fn();
    render(<ExecutionsPanel onNotice={onNotice} />);
    fireEvent.change(screen.getByPlaceholderText('act / 定义名称 / 任务 ID'), { target: { value: 'act' } });
    fireEvent.click(screen.getByText('启动执行'));
    await waitFor(() => expect(apiPost).toHaveBeenCalledTimes(1));
    expect(onNotice).toHaveBeenLastCalledWith(err(expect.stringContaining('connection lost')));
    await waitFor(() => expect(screen.getByText('启动执行').closest('button').disabled).toBe(false));
    fireEvent.click(screen.getByText('启动执行'));
    await waitFor(() => expect(apiPost).toHaveBeenCalledTimes(2));
    expect(apiPost.mock.calls[0][1].id).toBe(apiPost.mock.calls[1][1].id);
    expect(onNotice).toHaveBeenLastCalledWith(err(''));
    expect(await screen.findByText(/execution-detail:agent-/)).toBeTruthy();
  });
  it('paginates the five-field execution index without dropping its cursor', async () => {
    apiGet.mockImplementation(async (path) => {
      if (path === '/api/nodes') return { nodes: [node] };
      if (String(path).includes('cursor_created_at=9')) return { executions: [{ id: 'agent-old', kind: 'agent', node_id: 'n1', status: 'done', created_at: 8 }] };
      return { executions: [{ id: 'agent-new', kind: 'agent', node_id: 'n1', status: 'running', created_at: 10 }], next_cursor: { created_at: 9, id: 'agent-next' } };
    });
    render(<ExecutionsPanel onNotice={vi.fn()} />);
    expect(await screen.findByText('agent-new')).toBeTruthy();
    fireEvent.click(screen.getByText('加载更早的执行'));
    expect(await screen.findByText('agent-old')).toBeTruthy();
    expect(apiGet).toHaveBeenCalledWith('/api/executions?limit=50&cursor_created_at=9&cursor_id=agent-next');
  });
  it('renders CPU normalized load and only performs maintenance after a user action', async () => {
    apiGet.mockResolvedValue({ nodes: [node] }); apiPost.mockResolvedValue({ snapshot: node.snapshot });
    render(<FleetNodesPanel onNotice={vi.fn()} />);
    expect(await screen.findByText('1.50')).toBeTruthy(); expect(screen.getByText('maintainer-n1')).toBeTruthy(); expect(apiPost).not.toHaveBeenCalled();
    fireEvent.click(screen.getByText('维护节点')); fireEvent.click(screen.getByText('执行指令'));
    await waitFor(() => expect(apiPost).toHaveBeenCalledWith('/api/nodes/n1/maintenance', { action: 'status', input: {} }));
  });
  it('creates distinct request IDs in browsers without randomUUID', () => {
    const getRandomValues = crypto.getRandomValues.bind(crypto);
    vi.stubGlobal('crypto', { getRandomValues });
    const first = newId('agent');
    expect(first).toMatch(/^agent-[a-f0-9]{32}$/);
    expect(newId('agent')).not.toBe(first);
  });
  it('keeps retired system runs filterable but removes their launch option', () => {
    expect(CREATABLE_KINDS.some((kind) => kind.value === 'system')).toBe(false);
    expect(KINDS.some((kind) => kind.value === 'system')).toBe(true);
  });
  it('rejects offline node options and reads persisted message blocks', () => {
    expect(nodeOptions([{ ...node, online: false }], 'agent')[1].disabled).toBe(true);
    expect(textOf({ blocks: [{ kind: 'text', text: 'node answer' }] })).toBe('node answer');
    expect(newId('agent')).toMatch(/^agent-/);
  });
  it('maps lifecycle actions without offering resume after terminal cancel', () => {
    expect(executionActions({ kind: 'agent', status: 'idle' })).toEqual({ interrupt: true, cancel: true, resume: false });
    expect(executionActions({ kind: 'agent', status: 'interrupted' })).toEqual({ interrupt: false, cancel: true, resume: true });
    expect(executionActions({ kind: 'system', status: 'interrupted' }).resume).toBe(false);
    expect(executionActions({ kind: 'agent', status: 'cancelled' })).toEqual({ interrupt: false, cancel: false, resume: false });
  });
  it('builds a stable list cursor and assembles ordinary message chunks', () => {
    expect(executionPagePath('dag', { created_at: 9, id: 'dag-a' }, 25)).toBe('/api/executions?limit=25&kind=dag&cursor_created_at=9&cursor_id=dag-a');
    const raw = new TextEncoder().encode(JSON.stringify([{ kind: 'text', text: '完成' }]));
    const bytes_b64 = btoa(String.fromCharCode(...raw));
    const state = appendMessagePage(null, { chunks: [{ seq: 1, role: 'assistant', created_at: 2, offset: 0, next_offset: raw.length, total_bytes: raw.length, eof: true, encoding: 'base64', bytes_b64 }], more: false });
    expect(state.messages[0].blocks[0].text).toBe('完成');
    expect(state.partial).toBeNull();
  });
  it('renders completed assistant Markdown through the shared transcript', () => {
    render(<ExecutionTranscript messages={[{ id: 'm1', role: 'assistant', blocks: [{ kind: 'text', text: '**完成**' }] }]} />);
    expect(screen.getByText('完成').tagName).toBe('STRONG');
  });
  it('bounds retained ordinary history while preserving cursor progress', () => {
    let state = null;
    for (let seq = 1; seq <= 105; seq += 1) {
      const bytes = new TextEncoder().encode(JSON.stringify([{ kind: 'text', text: `m${seq}` }]));
      state = appendMessagePage(state, { chunks: [{ seq, role: 'assistant', created_at: seq, offset: 0, next_offset: bytes.length, total_bytes: bytes.length, eof: true, encoding: 'base64', bytes_b64: btoa(String.fromCharCode(...bytes)) }], next_cursor: { seq, offset: 0 }, more: true });
    }
    expect(state.messages).toHaveLength(100); expect(state.messages[0].seq).toBe(6);
    expect(state.trimmed).toBe(true); expect(state.nextCursor).toEqual({ seq: 105, offset: 0 });
  });
  it('bounds the rendered event payload instead of retaining tool-sized frames', () => {
    const events = appendEvent([], { seq: 1, event: 'tool', data: { output: 'x'.repeat(80 * 1024) } });
    expect(events[0].text.length).toBeLessThan(66 * 1024); expect(events[0].text.endsWith('…')).toBe(true);
  });
  it('discovers bounded detail markers and reads UTF-8 without replacement characters', async () => {
    const chinese = new TextEncoder().encode('中');
    apiGet
      .mockResolvedValueOnce({ encoding: 'json-base64', offset: 0, next_offset: 2, total_bytes: 4, eof: false, bytes_b64: btoa(String.fromCharCode(91, chinese[0])) })
      .mockResolvedValueOnce({ encoding: 'json-base64', offset: 2, next_offset: 4, total_bytes: 4, eof: true, bytes_b64: btoa(String.fromCharCode(chinese[1], chinese[2])) });
    const marker = { omitted: true, field: 'result', total_bytes: 4, read_via: 'detail_field' };
    expect(detailMarkers({ request: { input: marker }, result: marker })).toEqual([marker]);
    render(<PayloadWindows id="agent-a" marker={marker} />);
    fireEvent.click(screen.getByText('执行结果'));
    expect(await screen.findByText('[', { exact: true })).toBeTruthy();
    fireEvent.click(screen.getByText('下一段'));
    expect(await screen.findByText('中', { exact: true })).toBeTruthy();
    expect(document.body.textContent).not.toContain('�');
  });
  it('shows one bounded workflow page at a time and preserves its cursor', async () => {
    apiGet.mockResolvedValue({ items: [{ todo_id: 'b', status: 'done' }], next_ordinal: null });
    render(<WorkloadDetail id="todos-a" kind="todos" detail={{ workflow: {
      workflow: { status: 'running' },
      items: [{ todo_id: 'a', status: 'running' }],
      items_page: { next_ordinal: 7, more: true },
    } }} />);
    expect(screen.getByText('a · running')).toBeTruthy();
    fireEvent.click(screen.getByText('下一页'));
    expect(await screen.findByText('b · done')).toBeTruthy();
    expect(screen.queryByText('a · running')).toBeNull();
    expect(apiGet).toHaveBeenCalledWith('/api/executions/todos-a/todo-items?after_ordinal=7');
    fireEvent.click(screen.getByText('上一页'));
    expect(screen.getByText('a · running')).toBeTruthy();
  });
  it('shows accepted TODO initialization without reporting false zero progress', () => {
    render(<WorkloadDetail id="todos-new" kind="todos" detail={{
      execution: { status: 'running' },
      workflow: null,
      workflow_initializing: true,
    }} />);
    expect(screen.getByText('TODO 工作流正在初始化')).toBeTruthy();
    expect(screen.getByText('节点已接受任务，详情准备完成后会自动刷新。')).toBeTruthy();
    expect(screen.queryByText('0 / 0 完成')).toBeNull();
  });
  it.each([
    ['stopping', 'TODO 工作流正在停止'],
    ['stopped', 'TODO 工作流未启动'],
    ['failed', 'TODO 工作流初始化失败'],
  ])('renders the %s TODO pre-initialization outcome without false progress', (state, label) => {
    const { unmount } = render(<WorkloadDetail id={`todos-${state}`} kind="todos" detail={{
      execution: { status: state === 'failed' ? 'error' : 'interrupted' },
      error: state === 'failed' ? 'injected initialization failure' : null,
      workflow: null,
      workflow_initialization: state,
      workflow_initializing: false,
    }} />);
    expect(screen.getByText(label)).toBeTruthy();
    if (state === 'failed') expect(screen.getByText('injected initialization failure')).toBeTruthy();
    expect(screen.queryByText('0 / 0 完成')).toBeNull();
    unmount();
  });
  it('reads an omitted child field through its top execution owner', async () => {
    const bytes = new TextEncoder().encode('会话片段');
    apiGet.mockResolvedValue({ encoding: 'utf8-base64', offset: 0, next_offset: bytes.length, total_bytes: bytes.length, eof: true, bytes_b64: btoa(String.fromCharCode(...bytes)) });
    const marker = { omitted: true, field: 'todo.item.child.session_history', total_bytes: bytes.length, read_via: 'detail_field' };
    render(<WorkloadDetail id="todos-owner" kind="todos" detail={{ workflow: {
      workflow: { status: 'running' }, items: [{ todo_id: 'child', status: 'done', session_history: marker }], items_page: {},
    } }} />);
    fireEvent.click(screen.getByText('会话记录'));
    expect(await screen.findByText('会话片段', { exact: true })).toBeTruthy();
    expect(apiGet).toHaveBeenCalledWith('/api/executions/todos-owner/detail-field?field=todo.item.child.session_history&offset=0');
  });
  it('offers bounded team plan, participant result and summary windows', async () => {
    const bytes = new TextEncoder().encode('成员结论');
    apiGet.mockResolvedValue({ encoding: 'utf8-base64', offset: 0, next_offset: bytes.length, total_bytes: bytes.length, eof: true, bytes_b64: btoa(String.fromCharCode(...bytes)) });
    render(<WorkloadDetail id="team-owner" kind="team" detail={{ definition: { name: 'release', captain: 'lead', members: [{ agent: 'lead', capabilities: ['发布编排'] }] }, topic: {
      turns: [{ turn: 3, meta: { question: '发布检查', participants: ['lead'], aligned: true, sub_turns: 1 }, detail_fields: { plan: 'team.turn.3.plan' } }],
    } }} />);
    expect(screen.getByText('发布检查')).toBeTruthy(); expect(screen.getByText('已对齐')).toBeTruthy();
    /// 成员身份即 agent：固化下来的能力快照随 Tag 一并展示。
    expect(screen.getByText('lead · 发布编排')).toBeTruthy();
    expect(screen.getByText('查看本轮计划')).toBeTruthy(); expect(screen.getByText('第 1 次小结')).toBeTruthy();
    fireEvent.click(screen.getByText('lead · 第 1 次结果'));
    expect(await screen.findByText('成员结论', { exact: true })).toBeTruthy();
    expect(apiGet).toHaveBeenCalledWith('/api/executions/team-owner/detail-field?field=team.turn.3.sub.0.result.lead&offset=0');
  });
  it('reloads the first message window once when an execution becomes idle', () => {
    expect(messageRefreshMode('running', 0)).toBe('poll');
    expect(messageRefreshMode('idle', 0)).toBe('once');
    expect(messageRefreshMode('idle', 1)).toBe('paused');
  });
  it('keeps a large UTF-8 message as bounded adjacent text windows', () => {
    const chinese = new TextEncoder().encode('中');
    const first = Uint8Array.from([91, 34, chinese[0]]);
    const second = Uint8Array.from([chinese[1], chinese[2], 34]);
    const chunk = (bytes, offset) => ({ seq: 7, role: 'assistant', created_at: 3, offset, next_offset: offset + bytes.length, total_bytes: LARGE_MESSAGE_BYTES + 1, eof: false, encoding: 'base64', bytes_b64: btoa(String.fromCharCode(...bytes)) });
    const a = appendMessagePage(null, { chunks: [chunk(first, 0)], next_cursor: { seq: 7, offset: first.length }, more: true });
    expect(a.large[0].text).toBe('["'); expect(a.large[0].tail.byteLength).toBe(1);
    const b = appendMessagePage(a, { chunks: [chunk(second, first.length)], more: true }, a.large[0].tail);
    expect(b.large[0].text).toBe('中"'); expect(b.large[0].text).not.toContain('�');
    const revisited = appendMessagePage(null, { chunks: [chunk(Uint8Array.from([chinese[1], chinese[2], 34]), 100)], more: true });
    expect(revisited.large[0].text).toBe('"'); expect(revisited.large[0].start).toBe(102);
  });
  it('retries an unconfirmed Brain dispatch with the same request identifier', async () => {
    apiGet.mockImplementation(async (path) => path === '/api/nodes' ? { nodes: [node] } : { capabilities: [] });
    apiPost.mockRejectedValueOnce(new Error('connection lost')).mockResolvedValueOnce({ execution: { id: 'agent-brain-1', kind: 'agent', node_id: 'n1', status: 'pending', created_at: 1 } });
    const onNotice = vi.fn();
    render(<FleetBrainPanel onNotice={onNotice} />);
    fireEvent.mouseDown(screen.getByLabelText('目标节点').closest('.ant-select'));
    fireEvent.click(await screen.findByText('worker', { selector: '.ant-select-item-option-content' }));
    fireEvent.change(await screen.findByLabelText('需求'), { target: { value: '检查发布' } });
    fireEvent.click(screen.getByText('开始执行'));
    await waitFor(() => expect(apiPost).toHaveBeenCalledTimes(1));
    expect(onNotice).toHaveBeenLastCalledWith(err('connection lost'));
    await waitFor(() => expect(screen.getByText('开始执行').closest('button').disabled).toBe(false));
    fireEvent.click(screen.getByText('开始执行'));
    await waitFor(() => expect(apiPost).toHaveBeenCalledTimes(2));
    expect(apiPost.mock.calls[0][1].request_id).toBe(apiPost.mock.calls[1][1].request_id);
    expect(apiPost.mock.calls[0][1].request_id).toMatch(/^request-/);
    expect(onNotice).toHaveBeenLastCalledWith(err(''));
    expect(await screen.findByText('execution-detail:agent-brain-1')).toBeTruthy();
  });
  it('launches a configured team as one node-owned execution', async () => {
    const teamNode = { ...node, kinds: ['team'] };
    apiGet.mockImplementation(async (path) => {
      if (path === '/api/teams') return { teams: [{ name: 'release', captain: 'act', members: [{ agent: 'act' }] }] };
      if (path === '/api/nodes') return { nodes: [teamNode] };
      return { agents: [{ agent: 'act', capabilities: [{ id: 'c1', summary: '交付' }] }] };
    });
    apiPost.mockRejectedValueOnce(new Error('connection lost')).mockImplementation(async (_, body) => ({ ...body, node_id: 'n1', status: 'pending', created_at: 1 }));
    const onNotice = vi.fn();
    render(<FleetTeamsPanel onNotice={onNotice} />);
    fireEvent.click(await screen.findByText('启动团队'));
    expect(await screen.findByText(/整个团队会在同一个执行节点/)).toBeTruthy();
    fireEvent.change(screen.getByLabelText('任务要求'), { target: { value: '准备发布' } });
    const submit = [...document.querySelectorAll('.ant-modal button')].find((button) => button.textContent.replace(/\s+/g, '') === '启动');
    expect(submit).toBeTruthy(); fireEvent.click(submit);
    await waitFor(() => expect(apiPost).toHaveBeenCalledTimes(1));
    expect(onNotice).toHaveBeenLastCalledWith(err(expect.stringContaining('connection lost')));
    await waitFor(() => expect(submit.disabled).toBe(false)); fireEvent.click(submit);
    await waitFor(() => expect(apiPost).toHaveBeenCalledTimes(2));
    expect(apiPost.mock.calls[0][1].id).toBe(apiPost.mock.calls[1][1].id);
    expect(apiPost).toHaveBeenLastCalledWith('/api/executions', expect.objectContaining({ kind: 'team', target: 'release', id: expect.stringMatching(/^team-/), input: { prompt: '准备发布' } }));
    expect(onNotice).toHaveBeenLastCalledWith(err(''));
  });
  it('builds a captain-first roster from agent identity and posts deduped members', async () => {
    apiGet.mockImplementation(async (path) => {
      if (path === '/api/teams') return { teams: [] };
      if (path === '/api/nodes') return { nodes: [] };
      return { agents: [
        { agent: 'act', capabilities: [{ id: 'c1', summary: '执行任务' }] },
        { agent: 'plan', capabilities: [{ id: 'c2', summary: '规划拆解' }] },
        { agent: 'explore', capabilities: [] },
      ] };
    });
    apiPost.mockResolvedValue({ ok: true });
    const onNotice = vi.fn();
    render(<FleetTeamsPanel onNotice={onNotice} />);
    fireEvent.click(await screen.findByText('创建团队'));
    fireEvent.change(await screen.findByLabelText('团队名称'), { target: { value: 'release' } });
    /// 队长 Select 可搜索：先过滤再选中 plan。
    fireEvent.mouseDown(screen.getByLabelText('队长').closest('.ant-select'));
    fireEvent.change(document.activeElement, { target: { value: 'pl' } });
    fireEvent.click(await screen.findByText('plan', { selector: '.ant-select-item-option-content' }));
    /// 队员为 multiple Select：再选 act（Form.useWatch 的更新在下一个交互 tick 生效，断言前等待）。
    fireEvent.mouseDown(screen.getByLabelText('队员').closest('.ant-select'));
    fireEvent.click(await screen.findByText('act', { selector: '.ant-select-item-option-content' }));
    /// 多选下拉在选中后收起：重新展开再选 explore（无绑定能力的兜底分支）。
    fireEvent.mouseDown(screen.getByLabelText('队员').closest('.ant-select'));
    /// 重开下拉会留下旧浮层：取最后一个（最新）explore 选项。
    const exploreOptions = await screen.findAllByText('explore', { selector: '.ant-select-item-option-content' });
    fireEvent.click(exploreOptions[exploreOptions.length - 1]);
    const rosterRow = (agent) => document.querySelector(`[data-agent="${agent}"]`);
    await waitFor(() => expect(rosterRow('act')).toBeTruthy());
    expect(rosterRow('plan').textContent).toContain('规划拆解');
    expect(rosterRow('act').textContent).toContain('执行任务');
    /// 无绑定能力的 agent 走「暂无能力画像」兜底文案。
    expect(rosterRow('explore').textContent).toContain('暂无能力画像');
    /// 队长置顶：plan 行带队长标识且排在 act 之前。
    expect(rosterRow('plan').textContent).toContain('队长');
    expect(rosterRow('act').textContent).not.toContain('队长');
    expect(rosterRow('plan').compareDocumentPosition(rosterRow('act')) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    fireEvent.click(screen.getByText('保存团队'));
    await waitFor(() => expect(apiPost).toHaveBeenCalledWith('/api/teams', { name: 'release', captain: 'plan', members: [{ agent: 'plan' }, { agent: 'act' }, { agent: 'explore' }] }));
    expect(onNotice).toHaveBeenLastCalledWith(err(''));
  });
});
