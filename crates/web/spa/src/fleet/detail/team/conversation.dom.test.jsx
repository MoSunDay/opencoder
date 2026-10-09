// @vitest-environment jsdom
import '../../../test/setup-dom.js';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { apiGet } from '../../../api.js';
import { TeamDetail } from './index.jsx';
import { TeamRecord } from './record.jsx';

vi.mock('../../../api.js', () => ({ apiGet: vi.fn() }));
afterEach(() => { cleanup(); vi.resetAllMocks(); });

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
const show = (changes = {}) => render(<TeamDetail id="team-a" detail={{ definition, execution: { status: 'done' }, topic: { ...topic, ...changes } }} />);

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

  it('shows an honest waiting state without inventing a round or finished result', () => {
    show({ status: 'executing', finish_reason: null, final_summary: null, turns: [] });
    expect(screen.getByText('讨论进行中')).toBeTruthy();
    expect(screen.getByText('等待本轮讨论记录')).toBeTruthy();
    expect(screen.queryByRole('heading', { name: '最终结论' })).toBeNull();
    expect(apiGet).not.toHaveBeenCalled();
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
