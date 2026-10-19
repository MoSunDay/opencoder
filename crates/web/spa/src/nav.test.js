// nav.test.js — pure-node unit tests for the IA single source of truth
// (nav.js): category membership and fallbacks, per-category menu scoping,
// and PAGE_META coverage. No DOM here — the shell rendering lives in
// app.dom.test.jsx.

import { isValidElement } from 'react';
import { describe, expect, it } from 'vitest';
import { HEADERLESS_REASONS, NAV_CATEGORIES, PAGE_META, allowedPages, categoryHome, menuItemsOf, selectItemsOf, visibleCategories } from './nav.js';

// Flat {category, page, menu, icon} rows for table-driven assertions.
const ALL_ITEMS = NAV_CATEGORIES.flatMap((c) => c.items.map((i) => ({ ...i, category: c.key })));
const ALL_PAGES = ALL_ITEMS.map((i) => i.page);

describe('NAV_CATEGORIES shape', () => {
  it('declares the four IA categories in order', () => {
    expect(NAV_CATEGORIES.map((c) => c.key)).toEqual(['project', 'agent', 'ontology', 'admin']);
    expect(NAV_CATEGORIES.map((c) => c.label)).toEqual(['项目', 'Agent', 'Ontology', '后台管理']);
  });

  it('has no duplicate page keys across categories', () => {
    expect(new Set(ALL_PAGES).size).toBe(ALL_PAGES.length);
  });

  it('renames the fleet list to 节点列表 (old label Opencoder 列表 is gone)', () => {
    const node = NAV_CATEGORIES.find((c) => c.key === 'admin');
    expect(node.items.find((i) => i.page === 'nodes').menu).toBe('节点列表');
    expect(ALL_ITEMS.some((i) => i.menu === 'Opencoder 列表')).toBe(false);
  });

  it('renames the chat page to Agent (old label Operator is gone)', () => {
    // page key `chat`、ALL_PAGES 与 HEADERLESS_REASONS 约定不动：仅 menu 文案
    // 由 Operator 改名 Agent（页内模式 Segmented 才区分 Operator/Agent 链路）。
    const item = ALL_ITEMS.find((i) => i.page === 'chat');
    expect(item.menu).toBe('Agent');
    expect(ALL_ITEMS.some((i) => i.menu === 'Operator')).toBe(false);
  });

  it('stores icon component references, not JSX elements', () => {
    for (const item of ALL_ITEMS) {
      expect(item.icon).toBeTruthy();
      expect(isValidElement(item.icon)).toBe(false);
    }
  });
});

describe('categoryHome', () => {
  it('returns the first page of each category', () => {
    expect(categoryHome('project')).toBe('project');
    expect(categoryHome('agent')).toBe('brain');
    expect(categoryHome('admin')).toBe('nodes');
  });

  it('falls back to the default category home for unknown keys', () => {
    expect(categoryHome('nope')).toBe('nodes');
  });

  it('lists category pages in menu order', () => {
    expect(selectItemsOf(NAV_CATEGORIES.find((category) => category.key === 'project').items).map((item) => item.value)).toEqual(['project', 'projectTags']);
    // schedules（调度）挂在「全部执行」之后：cron 台账是执行面的时间维度。
    expect(selectItemsOf(NAV_CATEGORIES.find((category) => category.key === 'agent').items).map((item) => item.value)).toEqual(['brain', 'topics', 'schedules', 'dag', 'todos', 'team', 'chat', 'agents']);
    expect(selectItemsOf(NAV_CATEGORIES.find((category) => category.key === 'admin').items).map((item) => item.value)).toEqual(['nodes', 'users']);
  });
});

describe('menuItemsOf / selectItemsOf scoping', () => {
  it('builds antd Menu items keyed by page with matching labels', () => {
    const items = menuItemsOf(NAV_CATEGORIES.find((category) => category.key === 'admin').items);
    expect(items.map((i) => i.key)).toEqual(['nodes', 'users']);
    expect(items.map((i) => i.label)).toEqual(['节点列表', '用户权限']);
    expect(items.every((i) => isValidElement(i.icon))).toBe(true);
  });

  it('never leaks other categories into a menu', () => {
    const labels = menuItemsOf(NAV_CATEGORIES.find((category) => category.key === 'project').items).map((i) => i.label);
    expect(labels).toEqual(['Overview', 'Tag']);
    expect(selectItemsOf(NAV_CATEGORIES.find((category) => category.key === 'project').items)).toEqual([{ value: 'project', label: 'Overview' }, { value: 'projectTags', label: 'Tag' }]);
    for (const foreign of ['大脑调度', '全部执行', '节点列表']) {
      expect(labels).not.toContain(foreign);
    }
  });

  it('mirrors the same scoping as Select options for the mobile picker', () => {
    expect(selectItemsOf(NAV_CATEGORIES.find((category) => category.key === 'agent').items)).toEqual([
      { value: 'brain', label: '大脑调度' },
      { value: 'topics', label: '全部执行' },
      { value: 'schedules', label: '定时任务' },
      { value: 'dag', label: 'DAG 工作流' },
      { value: 'todos', label: 'TODO 管理' },
      { value: 'team', label: 'Team 组队' },
      { value: 'chat', label: 'Agent' },
      { value: 'agents', label: 'Agent 配置' },
    ]);
  });
});

describe('PAGE_META coverage', () => {
  it('covers every page key exactly', () => {
    const expected = [...ALL_PAGES].sort();
    expect([...Object.keys(PAGE_META), ...Object.keys(HEADERLESS_REASONS)].sort()).toEqual(expected);
  });

  it('keeps headerless pages out of PAGE_META while they stay in the IA', () => {
    for (const page of Object.keys(HEADERLESS_REASONS)) {
      expect(PAGE_META[page]).toBeUndefined();
      expect(ALL_PAGES).toContain(page);
    }
  });

  it('declares a headerless reason for every page missing from PAGE_META', () => {
    for (const page of Object.keys(HEADERLESS_REASONS)) {
      expect(['body-title', 'menu-only']).toContain(HEADERLESS_REASONS[page]);
    }
  });

  it('gives every page a non-empty title and description', () => {
    for (const [page, meta] of Object.entries(PAGE_META)) {
      expect(String(meta.title).length).toBeGreaterThan(0);
      expect(String(meta.desc).length).toBeGreaterThan(0);
    }
  });
});

describe('visibleCategories / allowedPages', () => {
  it('admin sees every page', () => {
    expect(visibleCategories({ role: 'admin' })).toBe(NAV_CATEGORIES);
    expect(allowedPages({ role: 'admin' })).toEqual(ALL_PAGES);
  });
  it.each(['editor', 'viewer', undefined])('keeps platform pages and hides administration for %s', (role) => {
    const visible = visibleCategories(role ? { role } : null);
    expect(visible.map((category) => category.key)).toEqual(['project', 'agent', 'ontology']);
    expect(allowedPages({ role })).toEqual(ALL_ITEMS.filter((item) => item.category !== 'admin').map((item) => item.page));
    expect(menuItemsOf(visible[0].items).map((item) => item.key)).toEqual(['project', 'projectTags']);
    expect(selectItemsOf(visible[0].items)).toEqual([{ value: 'project', label: 'Overview' }, { value: 'projectTags', label: 'Tag' }]);
  });
});
