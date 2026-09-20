// @vitest-environment jsdom
import '../../../test/setup-dom.js';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { BrainRunBody } from '../run.jsx';
import { apiGet } from '../../../api.js';

vi.mock('../../../api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn() }));
vi.mock('../../../sse.js', () => ({ openStream: vi.fn(() => ({ abort() {} })) }));
afterEach(() => { cleanup(); vi.clearAllMocks(); });

const view = {
  schema_version: 3,
  objective: '修复并复测',
  input_names: ['repo', 'commit'],
  run: { run_id: 'brain-v3', phase: 'waiting', round: 1, generation: 4, last_event_seq: 8, updated_at: 1, error: null },
  capabilities: [{ capability_id: 'cap-agent', kind: 'agent', target: 'act', version: 'current' }],
  rounds: [{ round: 1, status: 'running', operations: [{ operation_id: 'agent-1', run_id: 'brain-v3', round: 1, capability_id: 'cap-agent', execution_kind: 'agent', execution_id: 'agent-1', status: 'running', source_sequence: null, cancel_requested: false }] }],
};

describe('v3 大脑运行总览', () => {
  it('只显示摘要画布和轮次索引，点击 execution ID 打开托管执行面板', async () => {
    apiGet.mockImplementation(async (path) => {
      if (path === '/api/brain/runs/brain-v3') return { run: view.run, operations: view.rounds[0].operations };
      if (path === '/api/brain/runs/brain-v3/view') return view;
      if (path.startsWith('/api/brain/runs/brain-v3/events-page')) return { events: [], more: false };
      if (path === '/api/executions/agent-1') return { execution: { id: 'agent-1', kind: 'agent', status: 'running', node_id: 'node-1', created_at: 1 }, request: { kind: 'agent', target: 'act', input: {} } };
      if (path.startsWith('/api/executions/agent-1/messages')) return { messages: [], more: false, next_cursor: null };
      return {};
    });
    render(<BrainRunBody id="brain-v3" onNotice={vi.fn()} />);
    expect(await screen.findByRole('heading', { name: '修复并复测' })).toBeTruthy();
    expect(screen.getByLabelText('大脑调度总览画布')).toBeTruthy();
    expect(screen.getAllByText('第 1 轮').length).toBeGreaterThan(0);
    expect(screen.queryByText('步骤执行画布')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: /查看执行 agent-1/ }));
    expect(await screen.findByText('agent-1')).toBeTruthy();
    expect(screen.getByText('所属节点')).toBeTruthy();
  });
});
