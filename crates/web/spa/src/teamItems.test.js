// teamItems.test.js — pure-node mapping rules for the team modal pickers
// (no DOM, no JSX). Guards the member/node → display-item contract that
// teamModals.jsx renders.

import { describe, expect, it } from 'vitest';
import {
  captainOptions,
  memberCapsText,
  nodeSelectOptions,
} from './teamItems.js';

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
