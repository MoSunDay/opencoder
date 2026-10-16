// @vitest-environment jsdom
import '../../test/setup-dom.js';
import { afterEach, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';
import { ExecutionsPanel } from '../executions.jsx';
import { apiGet } from '../../api.js';
import { executionHref } from './links.js';

vi.mock('../../api.js', () => ({ apiGet: vi.fn(), apiPost: vi.fn(), apiDel: vi.fn(), apiPut: vi.fn() }));
vi.mock('../../sse.js', () => ({ openStream: vi.fn(() => ({ abort() {} })) }));
Range.prototype.getClientRects = () => [];
Range.prototype.getBoundingClientRect = () => ({ left: 0, right: 0, top: 0, bottom: 0 });
afterEach(() => { cleanup(); window.history.replaceState(null, '', '/'); vi.clearAllMocks(); });

it.each(['team', 'dag', 'todos', 'brain'])('resolves a generic ID to its native %s process with no type in the link', async (kind) => {
  const id = `history-${kind}`;
  const detail = {
    execution: { id, kind, node_id: 'original-runtime', status: 'done', created_at: 1 },
    request: { kind, input: {} },
    ...(kind === 'team' ? { definition: { name: '原始组队', members: [] }, topic: { requirement: '原始讨论要求', final_summary: '原始讨论结论', turns: [], finish_reason: 'aligned' } } : {}),
    ...(kind === 'dag' ? { definition: { name: '原始 DAG', steps: [{ name: '历史步骤', depends_on: [], kind: { type: 'command', command: 'true' } }] } } : {}),
    ...(kind === 'todos' ? { workflow: { workflow: { id, status: 'done' }, items: [] } } : {}),
  };
  apiGet.mockImplementation(async (path) => {
    if (path === `/api/executions/${id}`) return detail;
    if (path === `/api/dag/runs/${id}/progress`) return { head_seq: 1, execution_status: 'done', steps: [{ name: '历史步骤', status: 'done' }] };
    if (path === `/api/brain/runs/${id}/layered`) return {
      schema_version: 4, run: { run_id: id, phase: 'completed', layer: 1 },
      plan: { title: '原始计划', nodes: [{ node_id: 's1', title: '原始大脑步骤', capability_id: 'cap-1' }], edges: [] },
      layers: [['s1']], operations: [], events: [],
    };
    if (path.includes('section=files')) return { files: { 'objective.md': '原始工作流要求' } };
    if (path.startsWith(`/api/todo/workflows/${id}/review`)) return {
      workflow: { id, status: 'done', generation: 1, world_epoch: 0 }, execution_status: 'done', head_seq: 1, total: 1,
      nodes: [{ id: 't1', title: '原始工作流步骤', agent: 'act', depends_on: [], status: 'passed', attempt: 1 }],
    };
    return { nodes: [], executions: [], events: [], more: false };
  });
  window.history.replaceState(null, '', executionHref(id));
  render(<ExecutionsPanel onNotice={vi.fn()} />);
  expect(await screen.findByText('original-runtime')).toBeTruthy();
  expect(apiGet).toHaveBeenCalledWith(`/api/executions/${id}`);
  const process = { team: '原始讨论结论', dag: '原始 DAG', todos: '原始工作流步骤', brain: '原始大脑步骤' }[kind];
  expect(await screen.findByText(process)).toBeTruthy();
  expect(screen.getByText(id, { selector: '.ant-drawer-title' })).toBeTruthy();
});
