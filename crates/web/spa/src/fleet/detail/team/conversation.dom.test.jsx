// @vitest-environment jsdom
import '../../../test/setup-dom.js';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, renderHook, screen, waitFor } from '@testing-library/react';
import { apiGet } from '../../../api.js';
import { TeamExecutionProcess } from './index.jsx';
import { TeamRecord } from './record.jsx';
import { useRoundProgress } from './progress.js';

vi.mock('../../../api.js', () => ({ apiGet: vi.fn() }));
afterEach(() => { cleanup(); vi.resetAllMocks(); vi.useRealTimers(); });

const chunk = (record) => {
  const bytes = new TextEncoder().encode(JSON.stringify(record));
  return { encoding: 'json-base64', offset: 0, next_offset: bytes.length, total_bytes: bytes.length, eof: true, bytes_b64: btoa(String.fromCharCode(...bytes)) };
};
const topic = {
  status: 'finished', finish_reason: 'complete', requirement: '检查发布方案', final_summary: '最终采用分批发布',
  turns: [
    { turn: 1, question: '方案选择', participants: ['review'], aligned: true, sub_turns: 1 },
    { turn: 2, question: '风险检查', participants: ['act', 'review'], aligned: true, sub_turns: 2 },
  ],
};
const definition = { name: 'release', captain: 'lead', members: [{ agent: 'lead' }, { agent: 'act' }, { agent: 'review' }] };
const records = {
  'team.turn.1.sub.0.result.review': { answer: '历史方案发言', ok: true },
  'team.turn.1.sub.0.summary': { summary: '方案已选定', aligned: true },
  'team.turn.2.plan': { question: '风险检查', rationale: '先确认回滚条件' },
  'team.turn.2.sub.0.result.act': { answer: '执行成员首次发言', ok: true },
  'team.turn.2.sub.0.result.review': { answer: '评审成员首次发言', ok: true },
  'team.turn.2.sub.0.summary': { summary: '需要明确回滚条件', aligned: false, ambiguities: [{ node_id: 'act', question: '失败时如何回滚？' }] },
  'team.turn.2.sub.1.result.act': { answer: '失败后切回上一版本', ok: true },
  'team.turn.2.sub.1.summary': { summary: '回滚条件已确认', aligned: true, ambiguities: [] },
};
const respond = async (path) => {
  const field = new URL(path, 'http://test').searchParams.get('field');
  if (!records[field]) throw new Error(`unexpected field: ${field}`);
  return chunk(records[field]);
};
const show = (changes = {}) => render(<TeamExecutionProcess id="team-a" detail={{ definition, execution: { status: 'done' }, topic: { ...topic, ...changes } }} />);

describe('Team 讨论阅读', () => {
  it('shows the conclusion first and opens the latest clarification with only the requested member', async () => {
    apiGet.mockImplementation(respond);
    show();
    expect(await screen.findByText('失败后切回上一版本')).toBeTruthy();
    expect(await screen.findByText('回滚条件已确认')).toBeTruthy();
    expect(screen.getByText('待澄清：失败时如何回滚？')).toBeTruthy();
    expect(screen.getByRole('heading', { name: '最终结论' }).compareDocumentPosition(screen.getByRole('heading', { name: '讨论过程' })) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    const paths = apiGet.mock.calls.map(([path]) => path);
    expect(paths.some((path) => path.includes('team.turn.1.'))).toBe(false);
    expect(paths.some((path) => path.includes('sub.1.result.review'))).toBe(false);
    fireEvent.click(screen.getByRole('tab', { name: '首次讨论' }));
    expect(await screen.findByText('执行成员首次发言')).toBeTruthy();
    expect(await screen.findByText('评审成员首次发言')).toBeTruthy();
    expect(await screen.findByText('待澄清问题')).toBeTruthy();
    fireEvent.click(screen.getByText('本轮讨论安排'));
    expect(await screen.findByText('先确认回滚条件')).toBeTruthy();
    fireEvent.click(screen.getByText('方案选择'));
    expect(await screen.findByText('历史方案发言')).toBeTruthy();
    await waitFor(() => expect(screen.queryByText('执行成员首次发言')).toBeNull());
  });

  it.each([
    ['max_turns', '达到讨论轮次上限'], ['max_sub_turns', '达到澄清次数上限'],
    ['error', '讨论出现错误'], ['cancelled', '讨论已取消'],
  ])('does not describe %s as a completed discussion', (finish_reason, label) => {
    show({ finish_reason, turns: [] });
    expect(screen.getByText(label)).toBeTruthy();
    expect(screen.getByRole('heading', { name: '阶段小结' })).toBeTruthy();
    expect(screen.queryByRole('heading', { name: '最终结论' })).toBeNull();
  });

  it('shows captain planning while waiting for the first persisted plan', async () => {
    apiGet.mockRejectedValue(Object.assign(new Error('not found'), { status: 404 }));
    show({ status: 'executing', finish_reason: null, final_summary: null, turns: [] });
    expect(screen.getByText('讨论进行中')).toBeTruthy();
    expect(await screen.findByText('队长正在安排首轮讨论')).toBeTruthy();
    expect(screen.queryByRole('heading', { name: '最终结论' })).toBeNull();
    expect(apiGet).toHaveBeenCalledWith('/api/executions/team-a/detail-field?field=team.turn.1.plan&offset=0', expect.objectContaining({ signal: expect.any(AbortSignal) }));
  });

  it('paginates round history and retries failures without showing an empty history as success', async () => {
    apiGet.mockRejectedValueOnce(new Error('节点离线')).mockResolvedValueOnce({ turns: [{ turn: 51, question: '后续问题', sub_turns: 0 }], next_turn: null });
    show({ turns: [], turns_page: { next_turn: 50 } });
    fireEvent.click(screen.getByRole('button', { name: '下一页轮次' }));
    expect(await screen.findByText('节点离线')).toBeTruthy();
    expect(screen.queryByText('暂无讨论记录')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: /重\s*试/ }));
    expect(await screen.findByText('后续问题')).toBeTruthy();
    expect(apiGet).toHaveBeenLastCalledWith('/api/executions/team-a/team-turns?after_turn=50', expect.objectContaining({ signal: expect.any(AbortSignal) }));
    fireEvent.click(screen.getByRole('button', { name: '上一页轮次' }));
    expect(screen.queryByText('后续问题')).toBeNull();
  });

  it('keeps a large final summary as a readable action instead of rendering its marker as text', () => {
    show({ turns: [], final_summary: { omitted: true, field: 'team.topic', read_via: 'detail_field' } });
    expect(screen.getByRole('button', { name: 'Team 协作内容' })).toBeTruthy();
    expect(screen.queryByText('[object Object]')).toBeNull();
  });

  it('keeps the current summary available when the previous clarification list is too large', async () => {
    apiGet.mockImplementation((path) => path.includes('sub.0.summary')
      ? Promise.resolve({ encoding: 'json-base64', offset: 0, next_offset: 65536, total_bytes: 100000, eof: false, bytes_b64: btoa('x'.repeat(65536)) })
      : respond(path));
    show();
    expect(await screen.findByText('回滚条件已确认')).toBeTruthy();
    expect(await screen.findByRole('button', { name: '分段查看上次待澄清问题' })).toBeTruthy();
    expect(apiGet.mock.calls.some(([path]) => path.includes('.result.'))).toBe(false);
  });
});

describe('Team 讨论记录读取', () => {
  it('aborts an old record request and ignores its late response after switching execution', async () => {
    let finishOld;
    apiGet.mockImplementation((path) => path.includes('/old/') ? new Promise((resolve) => { finishOld = resolve; }) : Promise.resolve(chunk({ answer: '新记录' })));
    const { rerender } = render(<TeamRecord id="old" field="team.turn.1.plan" label="计划">{(value) => <p>{value.answer}</p>}</TeamRecord>);
    const oldSignal = apiGet.mock.calls[0][1].signal;
    rerender(<TeamRecord id="new" field="team.turn.1.plan" label="计划">{(value) => <p>{value.answer}</p>}</TeamRecord>);
    expect(await screen.findByText('新记录')).toBeTruthy();
    expect(oldSignal.aborted).toBe(true);
    await act(async () => { finishOld(chunk({ answer: '旧记录' })); });
    expect(screen.queryByText('旧记录')).toBeNull();
  });

  it('can retry an unavailable member record', async () => {
    apiGet.mockRejectedValueOnce(new Error('暂时不可用')).mockResolvedValueOnce(chunk({ answer: '重试得到的回答' }));
    render(<TeamRecord id="a" field="team.turn.1.sub.0.result.act" label="成员发言">{(value) => <p>{value.answer}</p>}</TeamRecord>);
    expect(await screen.findByText('暂时不可用')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: /重\s*试/ }));
    expect(await screen.findByText('重试得到的回答')).toBeTruthy();
  });

  it('only fetches one bounded window automatically for a large record', async () => {
    apiGet.mockResolvedValue({ encoding: 'json-base64', offset: 0, next_offset: 65536, total_bytes: 100000, eof: false, bytes_b64: btoa('x'.repeat(65536)) });
    const renderRecord = vi.fn(() => <p>should not parse an incomplete JSON record</p>);
    render(<TeamRecord id="a" field="team.turn.1.plan" label="计划">{renderRecord}</TeamRecord>);
    expect(await screen.findByRole('button', { name: '分段查看计划' })).toBeTruthy();
    expect(apiGet).toHaveBeenCalledTimes(1);
    expect(renderRecord).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: '分段查看计划' }));
    await waitFor(() => expect(apiGet).toHaveBeenCalledTimes(2));
    expect(await screen.findByText('当前 0–65536 / 100000 字节')).toBeTruthy();
  });
});

const notFound = () => Object.assign(new Error('记录尚未产生'), { status: 404 });
const inProgress = (turns = []) => ({ definition, execution: { status: 'running' }, topic: { status: 'executing', turns } });

describe('Team 实时讨论过程', () => {
  it('shows a persisted member answer before the round is complete, then follows clarification and the next round', async () => {
    vi.useFakeTimers();
    const liveRecords = {
      'team.turn.1.plan': { question: '当前讨论问题', participants: ['review'] },
      'team.turn.1.sub.0.result.review': { answer: '已产生的首轮发言', ok: true },
    };
    apiGet.mockImplementation(async (path) => {
      const field = new URL(path, 'http://test').searchParams.get('field');
      if (!liveRecords[field]) throw notFound();
      return chunk(liveRecords[field]);
    });
    let view;
    await act(async () => { view = render(<TeamExecutionProcess id="live" detail={inProgress()} />); });
    expect(screen.getByText('已产生的首轮发言')).toBeTruthy();
    expect(screen.getByText('等待本轮成员发言结束，由队长整理小结')).toBeTruthy();
    expect(screen.getByText('成员讨论中')).toBeTruthy();
    liveRecords['team.turn.1.sub.0.summary'] = { summary: '需要执行成员澄清', aligned: false, ambiguities: [{ node_id: 'act', question: '如何回滚？' }] };
    liveRecords['team.turn.1.sub.1.result.act'] = { answer: '已补充回滚步骤', ok: true };
    await act(async () => { await vi.advanceTimersByTimeAsync(3100); });
    expect(screen.getByRole('tab', { name: '补充澄清 1' })).toBeTruthy();
    expect(screen.getByText('已补充回滚步骤')).toBeTruthy();
    expect(apiGet.mock.calls.some(([path]) => path.includes('sub.1.result.review'))).toBe(false);
    // Reading an earlier stage must not stop observing the newest stage.
    fireEvent.click(screen.getByRole('tab', { name: '首次讨论' }));
    liveRecords['team.turn.1.sub.1.summary'] = { summary: '本轮完成', aligned: true };
    await act(async () => { await vi.advanceTimersByTimeAsync(3100); });
    expect(screen.getByText('队长正在收尾')).toBeTruthy();
    liveRecords['team.turn.2.plan'] = { question: '第二轮问题', participants: ['review'] };
    liveRecords['team.turn.2.sub.0.result.review'] = { answer: '第二轮发言', ok: true };
    await act(async () => { view.rerender(<TeamExecutionProcess id="live" detail={inProgress([{ turn: 1, question: '当前讨论问题', participants: ['review'], sub_turns: 2, aligned: true }])} />); });
    expect(screen.getByText('第二轮发言')).toBeTruthy();
    expect(screen.getByText('本轮完成')).toBeTruthy();
    const ended = inProgress([{ turn: 1, question: '当前讨论问题', participants: ['review'], sub_turns: 2, aligned: true }]);
    ended.execution.status = 'done'; ended.topic = { ...ended.topic, status: 'finished', finish_reason: 'complete', final_summary: '完整讨论结论' };
    await act(async () => { view.rerender(<TeamExecutionProcess id="live" detail={ended} />); });
    expect(screen.getByText('完整讨论结论')).toBeTruthy();
    expect(screen.queryByLabelText('当前讨论进展')).toBeNull();
  });
  it('retains unfinished member output after a failed run', async () => {
    apiGet.mockImplementation(async (path) => {
      if (path.includes('team.turn.1.plan')) return chunk({ question: '失败前的问题', participants: ['review'] });
      if (path.includes('result.review')) return chunk({ answer: '失败前已保存的发言', ok: true });
      throw notFound();
    });
    render(<TeamExecutionProcess id="failed" detail={{ ...inProgress(), execution: { status: 'error' }, topic: { status: 'finished', finish_reason: 'error', turns: [] } }} />);
    expect(await screen.findByText('失败前已保存的发言')).toBeTruthy();
    expect(screen.getByText('本轮尚未产生队长小结')).toBeTruthy();
  });
  it('distinguishes a missing plan from a failed read and retries the error', async () => {
    apiGet.mockRejectedValueOnce(Object.assign(new Error('节点暂时离线'), { status: 503 })).mockRejectedValue(notFound());
    render(<TeamExecutionProcess id="offline" detail={inProgress()} />);
    expect(await screen.findByText('节点暂时离线')).toBeTruthy();
    expect(screen.queryByText('队长正在安排首轮讨论')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: /重\s*试/ }));
    expect(await screen.findByText('队长正在安排首轮讨论')).toBeTruthy();
  });
  it('bounds progress discovery and aborts old reads when the execution changes', async () => {
    apiGet.mockImplementation(async (path) => chunk(path.includes('.plan') ? { participants: ['review'] } : { aligned: false }));
    const view = renderHook((props) => useRoundProgress(props), { initialProps: { id: 'many', number: 1, running: true } });
    await waitFor(() => expect(view.result.current.step).toBe(8));
    expect(apiGet).toHaveBeenCalledTimes(9);
    const signal = apiGet.mock.calls[0][1].signal;
    let complete;
    apiGet.mockImplementation(() => new Promise((resolve) => { complete = resolve; }));
    view.rerender({ id: 'new', number: 1, running: true });
    expect(signal.aborted).toBe(true);
    view.unmount();
    expect(apiGet.mock.calls.at(-1)[1].signal.aborted).toBe(true);
    await act(async () => { complete(chunk({ participants: ['review'] })); });
  });
});
