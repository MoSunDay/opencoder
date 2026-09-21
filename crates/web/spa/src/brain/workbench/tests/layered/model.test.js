// model.test.js — the pure v4 layered helpers. Every expectation is derived
// from LOCKED contract §4 payloads: layers are 1-based on the wire, run.layer
// counts the layers already dispatched, operation_id is `#l<layer>#<node>#a<n>`
// and the latest attempt is the live one (a superseded attempt never wins).
import { describe, expect, it } from 'vitest';
import {
  DEFAULT_MAX_ATTEMPTS, LAYERED_EVENTS, activeLayer, attemptsOf, barrier,
  decisionLabel, eventLabel, eventRows, eventSummary, isLayeredSnapshot,
  isLayeredView, layerGraph, layerLabel, layerNodeIds, layerRows, latestAttempt,
  maxAttemptsOf, nodeRow, operationsOf, planOf, roundDetail, schemaVersionOf,
  terminalPhase, totalLayers,
} from '../../layered/model.js';

const operation = (layer, node, attempt, status, executionId, extra = {}) => ({
  operation_id: `brain-v4#l${layer}#${node}#a${attempt}`, layer, node_id: node, attempt,
  capability_id: `cap-${node}`, execution_kind: 'agent', execution_id: executionId,
  status, cancel_requested: false, ...extra,
});

const view = {
  schema_version: 4,
  run: { run_id: 'brain-v4', phase: 'waiting', layer: 2, generation: 4, last_event_seq: 12, summary: null, error: null, depth: 0, parent: null, total_layers: 3, created_at: 100, updated_at: 200 },
  plan: {
    title: '分层能力画布', objective: '按层交付画布与决策明细', todo: { id: 'todo-7' }, max_rounds: 32,
    nodes: [
      { node_id: 'n-fetch', title: '抓取仓库', capability_id: 'cap-n-fetch', retry: { max_attempts: 2 } },
      { node_id: 'n-api', title: 'API 影响面', capability_id: 'cap-n-api', retry: { max_attempts: 3 } },
      { node_id: 'n-ui', title: '前端画布', capability_id: 'cap-n-ui', retry: { max_attempts: 2 } },
      { node_id: 'n-verdict', title: '汇总裁决', capability_id: 'cap-n-verdict', retry: { max_attempts: 2 } },
    ],
    edges: [{ from: 'n-fetch', to: 'n-api' }, { from: 'n-fetch', to: 'n-ui' }, { from: 'n-api', to: 'n-verdict' }, { from: 'n-ui', to: 'n-verdict' }],
  },
  layers: [['n-fetch'], ['n-api', 'n-ui'], ['n-verdict']],
  operations: [
    operation(1, 'n-fetch', 1, 'done', 'exec-fetch-a1'),
    operation(2, 'n-api', 1, 'done', 'exec-api-a1'),
    operation(2, 'n-ui', 1, 'error', 'exec-ui-a1'),
    operation(2, 'n-ui', 2, 'running', 'exec-ui-a2', { cancel_requested: true }),
  ],
  events: [
    { seq: 4, layer: 2, event_type: 'operation_retry_scheduled', node_id: 'n-ui', attempt: 2, execution_id: 'exec-ui-a2', decision_summary: null, reason_summary: '前端画布失败，重试第 2 次', evidence_execution_ids: ['exec-ui-a1'], at_ms: 200 },
    { seq: 1, layer: 0, event_type: 'run_created', node_id: null, attempt: null, execution_id: null, decision_summary: 'dispatch_layer', reason_summary: null, evidence_execution_ids: [], at_ms: 100 },
  ],
  capabilities: [{ capability_id: 'cap-n-fetch', kind: 'agent', target: 'act-fetch', version: 1 }],
};

describe('v4 分层画布的 schema_version 判定', () => {
  it('只认显式 schema_version 4：v3 快照与历史运行都不进入分层分支', () => {
    expect(schemaVersionOf(view)).toBe(4);
    expect(isLayeredSnapshot(view)).toBe(true);
    expect(isLayeredView(view)).toBe(true);
    expect(isLayeredSnapshot({ schema_version: 3, run: { run_id: 'r' }, operations: [] })).toBe(false);
    expect(isLayeredSnapshot({ phase: 'completed', plan: { plan: { schema_version: 2 } } })).toBe(false);
    expect(schemaVersionOf({})).toBeNull();
    // 4 的判定要求携带 run：一个没有 run 的 4 号载荷不是可用视图
    expect(isLayeredView({ schema_version: 4, plan: {} })).toBe(false);
  });

  it('phase / terminalPhase 覆盖 LOCKED 的八种运行阶段', () => {
    expect(terminalPhase('completed')).toBe(true);
    expect(terminalPhase('cancelled')).toBe(true);
    expect(terminalPhase('blocked')).toBe(false);
  });
});

describe('v4 分层分组与节点汇总', () => {
  it('layers[i] 是第 i+1 层，totalLayers 在 total_layers 缺失时回落到层数', () => {
    expect(layerNodeIds(view)).toEqual([['n-fetch'], ['n-api', 'n-ui'], ['n-verdict']]);
    expect(totalLayers(view)).toBe(3);
    expect(totalLayers({ ...view, run: { ...view.run, total_layers: 0 } })).toBe(3);
    expect(activeLayer(view)).toBe(2); // run.layer = 2 已派发，本层仍在等待
    expect(activeLayer({ ...view, run: { ...view.run, layer: 3, phase: 'completed' } })).toBeNull();
  });

  it('层汇总按最新尝试滚动：全部 done 才完成，重试计入尝试次数', () => {
    const rows = layerRows(view);
    expect(rows.map((row) => row.layer)).toEqual([1, 2, 3]);
    expect(rows.map((row) => row.status)).toEqual(['done', 'running', 'pending']);
    expect(rows.map((row) => row.complete)).toEqual([true, false, false]);
    expect(rows.map((row) => row.attempts)).toEqual([1, 3, 0]);
    expect(rows.map((row) => row.active)).toEqual([false, true, false]);
  });

  it('重试链保留全部尝试，最新一次尝试获胜且带重试徽标', () => {
    const operations = operationsOf(view);
    expect(attemptsOf(operations, 'n-ui').map((op) => op.attempt)).toEqual([1, 2]);
    expect(latestAttempt(operations, 'n-ui').operationId).toBe('brain-v4#l2#n-ui#a2');
    const row = nodeRow(view, 'n-ui');
    expect(row).toMatchObject({ status: 'running', attempt: 2, maxAttempts: 2, started: true, cancelRequested: true });
    expect(row.attempts).toHaveLength(2);
    // 未派发节点：无尝试，状态 pending，进度徽标 0/max_attempts
    expect(nodeRow(view, 'n-verdict')).toMatchObject({ status: 'pending', attempt: 0, maxAttempts: 2, started: false });
    // max_attempts 缺省 2，retry.max_attempts 越小越不合法
    expect(maxAttemptsOf({ retry: { max_attempts: 5 } })).toBe(5);
    expect(maxAttemptsOf({ retry: { max_attempts: 0 } })).toBe(DEFAULT_MAX_ATTEMPTS);
    expect(planOf(view)).toMatchObject({ title: '分层能力画布', todoId: 'todo-7', maxRounds: 32 });
  });

  it('层屏障进度 = 已完成层 / 总层数，并区分已派发层', () => {
    expect(barrier(view)).toMatchObject({ total: 3, completed: 1, dispatched: 2, remaining: 2, percent: 33, label: '1/3' });
    const done = { ...view, run: { ...view.run, layer: 3, phase: 'completed' }, operations: [
      operation(1, 'n-fetch', 1, 'done', 'exec-fetch-a1'), operation(2, 'n-api', 1, 'done', 'exec-api-a1'),
      operation(2, 'n-ui', 2, 'done', 'exec-ui-a2'), operation(3, 'n-verdict', 1, 'done', 'exec-verdict-a1'),
    ] };
    expect(barrier(done)).toMatchObject({ completed: 3, dispatched: 3, remaining: 0, percent: 100, label: '3/3' });
    expect(barrier({ schema_version: 4, run: { phase: 'ready' } })).toMatchObject({ total: 0, completed: 0, percent: 0, label: '0/0' });
  });

  it('画布从上到下：层内居中、边带绑定标签、只连已声明的节点', () => {
    const graph = layerGraph(view);
    expect(graph.nodes.map((node) => node.id)).toEqual(['n-fetch', 'n-api', 'n-ui', 'n-verdict']);
    const fetch = graph.nodes[0]; const api = graph.nodes[1];
    expect(fetch.position.y).toBe(0);
    expect(api.position.y).toBe(156 + 100);
    expect(fetch.data).toMatchObject({ layer: 1, active: false, status: 'done', upstreamTitles: [] });
    expect(api.data.upstreamTitles).toEqual(['抓取仓库']);
    expect(graph.nodes[3].data.active).toBe(false);
    expect(api.data.active).toBe(true);
    expect(graph.edges.map((edge) => edge.id)).toEqual(['n-fetch->n-api', 'n-fetch->n-ui', 'n-api->n-verdict', 'n-ui->n-verdict']);
    expect(graph.edges[2].data).toMatchObject({ upstream: 'API 影响面', downstream: '汇总裁决' });
    expect(graph.edges[2].animated).toBe(false);
    const orphan = layerGraph({ ...view, plan: { ...view.plan, edges: [{ from: 'n-fetch', to: 'missing' }] } });
    expect(orphan.edges).toEqual([]);
  });
});

describe('v4 事件日志与层决策明细', () => {
  it('事件按 seq 升序，类型/层/摘要可读', () => {
    const rows = eventRows(view);
    expect(rows.map((row) => row.seq)).toEqual([1, 4]);
    expect(rows.map((row) => row.eventType)).toEqual(['run_created', 'operation_retry_scheduled']);
    expect(eventLabel(rows[1].eventType)).toBe('重试排期');
    expect(eventLabel('unknown_type')).toBe('unknown_type');
    expect(rows.map((row) => layerLabel(row.layer))).toEqual(['初始', '层 2']);
    expect(eventSummary(rows[1])).toBe('前端画布失败，重试第 2 次');
    expect(eventSummary(rows[0])).toBe('派发本层');
    expect(Object.keys(LAYERED_EVENTS)).toContain('layer_barrier_reached');
    expect(rows[1].evidence).toEqual(['exec-ui-a1']);
  });

  it('roundDetail 归一化层决策明细并补默认值', () => {
    const detail = roundDetail({
      schema_version: 4, layer: 1, phase: 'dispatch_layer', reason: '抓取完成，进入并行层', evidence_execution_ids: ['exec-fetch-a1'],
      nodes: [{ node_id: 'n-api', title: 'API 影响面', capability_id: 'cap-n-api', status: 'running', attempt: 2, execution_id: 'exec-api-a2', execution_kind: 'agent', cancel_requested: true }, { node_id: 'n-ui' }],
    });
    expect(detail).toMatchObject({ schemaVersion: 4, layer: 1, phase: 'dispatch_layer', reason: '抓取完成，进入并行层', evidence: ['exec-fetch-a1'] });
    expect(detail.nodes[0]).toMatchObject({ nodeId: 'n-api', attempt: 2, cancelRequested: true });
    expect(detail.nodes[1]).toMatchObject({ nodeId: 'n-ui', title: 'n-ui', status: 'pending', attempt: 1, executionId: '' });
    expect(decisionLabel('dispatch_layer')).toBe('派发本层');
    expect(decisionLabel('waiting')).toBe('等待执行'); // round detail 的 phase 也可以直接展示
    expect(decisionLabel('')).toBe('');
  });
});
