// conversationItems.test.js — pure-node contract for the Conversations sidebar
// mapping (no DOM, no JSX, mirrors bubbleItems.test.js style). Guards what
// chatSidebar.jsx renders: key = session_id, label = frozen dialogLabel
// (title wins → id head truncation → '(untitled)'), dirty rows skipped.

import { describe, expect, it } from 'vitest';
import { dialogsToItems } from './conversationItems.js';
import { dialogLabel } from './format.js';

describe('dialogsToItems', () => {
  it('maps dialog rows to items keyed by session_id, in order', () => {
    const dialogs = [
      { session_id: 's1', title: '修复登录页', first_created_at: 1, last_created_at: 2 },
      { session_id: 's2', title: '重构 store', first_created_at: 3, last_created_at: 4 },
    ];
    expect(dialogsToItems(dialogs)).toEqual([
      { key: 's1', label: '修复登录页' },
      { key: 's2', label: '重构 store' },
    ]);
  });

  it('prefers the title over the id head', () => {
    const [item] = dialogsToItems([{ session_id: 'aaaaaaaaaaaaZZZZ', title: '标题优先' }]);
    expect(item.label).toBe('标题优先');
  });

  it('truncates the id head when the title is missing', () => {
    const [item] = dialogsToItems([{ session_id: 'abcdefgh12345678' }]);
    expect(item.key).toBe('abcdefgh12345678');
    expect(item.label).toBe('abcdefgh1234…');
  });

  it('keeps the (untitled) fallback inside dialogLabel, unreachable for kept rows', () => {
    // Frozen helper contract: a row with no title and no id labels as
    // '(untitled)' — but such rows carry no session_id either, so the mapper
    // drops them before labelling; the fallback is purely defensive.
    expect(dialogLabel({})).toBe('(untitled)');
    expect(dialogsToItems([{}])).toEqual([]);
  });

  it('returns [] for empty and absent lists', () => {
    expect(dialogsToItems([])).toEqual([]);
    expect(dialogsToItems(undefined)).toEqual([]);
    expect(dialogsToItems(null)).toEqual([]);
  });

  it('skips dirty rows without a session_id', () => {
    const items = dialogsToItems([
      null,
      { title: '没有 id 的行' },
      { id: 'legacy-id-only' },
      { session_id: '', title: '空 id' },
      { session_id: 'ok-1', title: '有效行' },
    ]);
    expect(items).toEqual([{ key: 'ok-1', label: '有效行' }]);
  });
});
