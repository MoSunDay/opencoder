// teamItems.test.js — pure-node mapping rules for the 组队/话题 tabs (no
// DOM, no JSX). Guards the team/topic → display-item contract that
// teamPanel / topicsPanel / topicDetail render.

import { describe, expect, it } from 'vitest';
import {
  ambiguityText,
  captainOptions,
  finishReasonText,
  memberCapsText,
  nodeSelectOptions,
  resultLabel,
  subTurnCount,
  teamCapSummary,
  topicCancelable,
  topicResumable,
  topicStatusView,
  turnAligned,
  turnTimelineItems,
} from './teamItems.js';

describe('topicStatusView', () => {
  it('maps executing to the processing preset', () => {
    expect(topicStatusView({ status: 'executing', finish_reason: null }))
      .toEqual({ color: 'processing', label: '执行中' });
  });

  it('maps finished topics by finish_reason', () => {
    expect(topicStatusView({ status: 'finished', finish_reason: 'complete' }))
      .toEqual({ color: 'success', label: '已完成' });
    expect(topicStatusView({ status: 'finished', finish_reason: 'max_turns' }))
      .toEqual({ color: 'warning', label: '轮数上限' });
    expect(topicStatusView({ status: 'finished', finish_reason: 'max_sub_turns' }))
      .toEqual({ color: 'warning', label: '子轮上限' });
    expect(topicStatusView({ status: 'finished', finish_reason: 'cancelled' }))
      .toEqual({ color: 'default', label: '已取消' });
    expect(topicStatusView({ status: 'finished', finish_reason: 'error' }))
      .toEqual({ color: 'error', label: '错误' });
  });

  it('falls back for unknown statuses, unstamped finishes and garbage', () => {
    expect(topicStatusView({ status: 'finished', finish_reason: null }).color).toBe('default');
    expect(topicStatusView({ status: 'weird' })).toEqual({ color: 'default', label: 'weird' });
    expect(topicStatusView(null)).toEqual({ color: 'default', label: '-' });
  });
});

describe('finishReasonText', () => {
  it('passes the machine value through, - while unstamped', () => {
    expect(finishReasonText('max_turns')).toBe('max_turns');
    expect(finishReasonText(null)).toBe('-');
    expect(finishReasonText(undefined)).toBe('-');
  });
});

describe('memberCapsText', () => {
  it('distinguishes 未画像 from a genuinely empty profile', () => {
    expect(memberCapsText({ capabilities: [], profiled_at: null })).toBe('未画像');
    expect(memberCapsText({ capabilities: [], profiled_at: 1 })).toBe('无');
    expect(memberCapsText({})).toBe('未画像');
  });

  it('joins capabilities and truncates beyond max', () => {
    expect(memberCapsText({ capabilities: ['rust', 'web'], profiled_at: 1 })).toBe('rust / web');
    expect(memberCapsText({ capabilities: ['a', 'b', 'c'], profiled_at: 1 }, 2)).toBe('a / b +1');
    expect(memberCapsText({ capabilities: ['a', null, 'b'], profiled_at: 1 })).toBe('a / b');
  });
});

describe('teamCapSummary', () => {
  const team = {
    members: [
      { node_id: 'n1', capabilities: ['rust', 'web'] },
      { node_id: 'n2', capabilities: ['rust', 'sql'] },
    ],
  };

  it('dedupes the union across members', () => {
    expect(teamCapSummary(team)).toBe('rust / web / sql');
  });

  it('keeps the 未画像 / 无 / - ladder for empty capability sets', () => {
    expect(teamCapSummary({ members: [] })).toBe('-');
    expect(teamCapSummary({ members: [{ profiled_at: null }] })).toBe('未画像');
    expect(teamCapSummary({ members: [{ capabilities: [], profiled_at: 1 }] })).toBe('无');
    expect(teamCapSummary(null)).toBe('-');
  });
});

describe('nodeSelectOptions / captainOptions', () => {
  const nodes = [
    { id: 'n1', name: 'alpha' },
    { id: 'n2' }, // nameless node falls back to id
    null,
    { name: 'no-id' },
  ];

  it('builds picker options and skips garbage rows', () => {
    expect(nodeSelectOptions(nodes)).toEqual([
      { value: 'n1', label: 'alpha' },
      { value: 'n2', label: 'n2' },
    ]);
    expect(nodeSelectOptions(undefined)).toEqual([]);
  });

  it('lists current members first, then unseen nodes, deduped', () => {
    const team = { members: [{ node_id: 'n1', name: 'alpha' }] };
    expect(captainOptions(team, nodes)).toEqual([
      { value: 'n1', label: 'alpha · 成员' },
      { value: 'n2', label: 'n2 · 节点' },
    ]);
    expect(captainOptions(null, nodes)).toHaveLength(2);
  });
});

describe('turn mapping', () => {
  it('turnAligned reads the list stamp, else the last sub-turn summary', () => {
    expect(turnAligned({ aligned: true })).toBe(true);
    expect(turnAligned({ aligned: false, sub_turns: [{ summary: { aligned: true } }] })).toBe(false);
    expect(turnAligned({ sub_turns: [{ summary: { aligned: false } }, { summary: { aligned: true } }] })).toBe(true);
    expect(turnAligned({ sub_turns: [] })).toBe(false);
    expect(turnAligned(null)).toBe(false);
  });

  it('subTurnCount tolerates missing arrays', () => {
    expect(subTurnCount({ sub_turns: [{}, {}] })).toBe(2);
    expect(subTurnCount({})).toBe(0);
    expect(subTurnCount(null)).toBe(0);
  });

  it('turnTimelineItems reads question/participants from both row shapes', () => {
    const items = turnTimelineItems([
      { turn: 1, question: 'q1', participants: ['n1'], aligned: true, sub_turns: [{}, {}] },
      { turn: 2, plan: { question: 'q2', participants: ['n2'] }, sub_turns: [{ summary: { aligned: false } }] },
      { sub_turns: [] }, // no turn number → dropped
      null,
    ]);
    expect(items).toEqual([
      { key: 1, turn: 1, color: 'green', question: 'q1', participants: ['n1'], aligned: true, subTurns: 2 },
      { key: 2, turn: 2, color: 'gray', question: 'q2', participants: ['n2'], aligned: false, subTurns: 1 },
    ]);
    expect(turnTimelineItems(undefined)).toEqual([]);
  });
});

describe('result / ambiguity text', () => {
  it('resultLabel marks alignment follow-ups and failures', () => {
    expect(resultLabel({ node_id: 'n1', kind: 'answer', ok: true })).toBe('n1 · 回答');
    expect(resultLabel({ node_id: 'n2', kind: 'alignment', ok: true })).toBe('n2 · 对齐追答');
    expect(resultLabel({ node_id: 'n3', kind: 'answer', ok: false })).toBe('n3 · 回答 · 失败');
    expect(resultLabel(null)).toBe('- · 回答');
  });

  it('ambiguityText renders node + question', () => {
    expect(ambiguityText({ node_id: 'n1', question: '边界在哪' })).toBe('n1：边界在哪');
    expect(ambiguityText(null)).toBe('-：');
  });
});

describe('topic action availability', () => {
  it('only executing topics cancel; only finished non-complete topics resume', () => {
    const executing = { status: 'executing', finish_reason: null };
    const done = { status: 'finished', finish_reason: 'complete' };
    const capped = { status: 'finished', finish_reason: 'max_turns' };
    expect(topicCancelable(executing)).toBe(true);
    expect(topicCancelable(capped)).toBe(false);
    expect(topicCancelable(null)).toBe(false);
    expect(topicResumable(capped)).toBe(true);
    expect(topicResumable({ status: 'finished', finish_reason: 'cancelled' })).toBe(true);
    expect(topicResumable(done)).toBe(false);
    expect(topicResumable(executing)).toBe(false);
  });
});
