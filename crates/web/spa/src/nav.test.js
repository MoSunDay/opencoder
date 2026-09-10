// nav.test.js — pure-node unit tests for the IA single source of truth
// (nav.js): category membership and fallbacks, per-category menu scoping,
// and PAGE_META coverage. No DOM here — the shell rendering lives in
// app.dom.test.jsx.

import { isValidElement } from 'react';
import { describe, expect, it } from 'vitest';
import {
  CATEGORY_OPTIONS,
  DEFAULT_CATEGORY,
  DEFAULT_PAGE,
  NAV_CATEGORIES,
  PAGE_META,
  allowedPages,
  categoryHome,
  categoryOf,
  menuItemsOf,
  menuKey,
  menuOf,
  pagesOf,
  selectItemsOf,
  selectOptionsOf,
  visibleCategories,
} from './nav.js';

// Flat {category, page, menu, icon} rows for table-driven assertions.
const ALL_ITEMS = NAV_CATEGORIES.flatMap((c) => c.items.map((i) => ({ ...i, category: c.key })));
const ALL_PAGES = ALL_ITEMS.map((i) => i.page);

describe('NAV_CATEGORIES shape', () => {
  it('declares the three IA categories in order', () => {
    expect(NAV_CATEGORIES.map((c) => c.key)).toEqual(['project', 'agent', 'node']);
    expect(NAV_CATEGORIES.map((c) => c.label)).toEqual(['项目', 'Agent', '节点']);
  });

  it('has no duplicate page keys across categories', () => {
    expect(new Set(ALL_PAGES).size).toBe(ALL_PAGES.length);
  });

  it('renames the fleet list to 节点列表 (old label Opencoder 列表 is gone)', () => {
    const node = NAV_CATEGORIES.find((c) => c.key === 'node');
    expect(node.items.find((i) => i.page === 'nodes').menu).toBe('节点列表');
    expect(ALL_ITEMS.some((i) => i.menu === 'Opencoder 列表')).toBe(false);
  });

  it('stores icon component references, not JSX elements', () => {
    for (const item of ALL_ITEMS) {
      expect(item.icon).toBeTruthy();
      expect(isValidElement(item.icon)).toBe(false);
    }
  });
});

describe('categoryOf', () => {
  it('maps every declared page back to its own category', () => {
    for (const item of ALL_ITEMS) {
      expect(categoryOf(item.page)).toBe(item.category);
    }
  });

  it('falls back to the default category for unknown pages', () => {
    expect(DEFAULT_CATEGORY).toBe('node');
    expect(categoryOf('nope')).toBe('node');
    expect(categoryOf('')).toBe('node');
    expect(categoryOf(undefined)).toBe('node');
    // Case-sensitive: not a nodes alias.
    expect(categoryOf('Nodes')).toBe('node');
  });
});

describe('categoryHome / pagesOf', () => {
  it('returns the first page of each category', () => {
    expect(categoryHome('project')).toBe('project');
    expect(categoryHome('agent')).toBe('brain');
    expect(categoryHome('node')).toBe('nodes');
  });

  it('falls back to the default category home for unknown keys', () => {
    expect(categoryHome('nope')).toBe(DEFAULT_PAGE);
  });

  it('lists category pages in menu order', () => {
    // Iteration 4 widens the project category to 项目 / 进展 / Owner 视角.
    expect(pagesOf('project')).toEqual(['project', 'progress', 'ownerview']);
    expect(pagesOf('agent')).toEqual(['brain', 'topics', 'dag', 'todos', 'team', 'chat', 'agents']);
    expect(pagesOf('node')).toEqual(['nodes', 'envs']);
  });
});

describe('menuOf / selectOptionsOf scoping', () => {
  it('builds antd Menu items keyed by page with matching labels', () => {
    const items = menuOf('node');
    expect(items.map((i) => i.key)).toEqual(['nodes', 'envs']);
    expect(items.map((i) => i.label)).toEqual(['节点列表', 'Env 管理']);
    expect(items.every((i) => isValidElement(i.icon))).toBe(true);
  });

  it('never leaks other categories into a menu', () => {
    const labels = menuOf('project').map((i) => i.label);
    expect(labels).toEqual(['项目', '进展', 'Owner 视角']);
    for (const foreign of ['大脑调度', '全部执行', '节点列表']) {
      expect(labels).not.toContain(foreign);
    }
  });

  it('mirrors the same scoping as Select options for the mobile picker', () => {
    expect(selectOptionsOf('agent')).toEqual([
      { value: 'brain', label: '大脑调度' },
      { value: 'topics', label: '全部执行' },
      { value: 'dag', label: 'DAG 工作流' },
      { value: 'todos', label: 'TODO 管理' },
      { value: 'team', label: '团队组队' },
      { value: 'chat', label: '会话交互' },
      { value: 'agents', label: 'Agent 配置' },
    ]);
  });
});

describe('menuKey fallbacks', () => {
  it('keeps known pages verbatim', () => {
    for (const page of ALL_PAGES) {
      expect(menuKey(page)).toBe(page);
    }
  });

  it('falls back to the default page for unknown pages', () => {
    expect(DEFAULT_PAGE).toBe('nodes');
    expect(menuKey('nope')).toBe('nodes');
  });
});

describe('CATEGORY_OPTIONS / PAGE_META coverage', () => {
  it('exposes the categories as Segmented options', () => {
    expect(CATEGORY_OPTIONS).toEqual([
      { value: 'project', label: '项目' },
      { value: 'agent', label: 'Agent' },
      { value: 'node', label: '节点' },
    ]);
  });

  it('covers every page key exactly', () => {
    const expected = [...ALL_PAGES].sort();
    expect(Object.keys(PAGE_META).sort()).toEqual(expected);
  });

  it('gives every page a non-empty title and description', () => {
    for (const [page, meta] of Object.entries(PAGE_META)) {
      expect(String(meta.title).length).toBeGreaterThan(0);
      expect(String(meta.desc).length).toBeGreaterThan(0);
    }
  });
});

describe('visibleCategories / allowedPages (permission view)', () => {
  it('admin and the pre-probe null identity see the full IA', () => {
    expect(visibleCategories(null)).toBe(NAV_CATEGORIES);
    expect(visibleCategories({ name: 'boss', role: 'admin' })).toBe(NAV_CATEGORIES);
    expect(allowedPages(null)).toEqual(ALL_PAGES);
  });

  it('non-admin keeps a single Agent category with only 全部执行', () => {
    const visible = visibleCategories({ name: 'guest', role: 'user' });
    expect(visible).toHaveLength(1);
    expect(visible[0].key).toBe('agent');
    expect(visible[0].label).toBe('Agent');
    // 全部执行 item 与全量 IA 同一行（icon/menu 不漂移）。
    const topics = NAV_CATEGORIES.find((c) => c.key === 'agent').items.find((i) => i.page === 'topics');
    expect(visible[0].items).toEqual([topics]);
    expect(allowedPages({ name: 'guest', role: 'user' })).toEqual(['topics']);
  });

  it('feeds the shell Menu/Select builders without dangling keys', () => {
    const visible = visibleCategories({ name: 'guest', role: 'root' });
    const menu = menuItemsOf(visible[0].items);
    const select = selectItemsOf(visible[0].items);
    expect(menu.map((i) => i.key)).toEqual(['topics']);
    expect(menu[0].label).toBe('全部执行');
    expect(isValidElement(menu[0].icon)).toBe(true);
    expect(select).toEqual([{ value: 'topics', label: '全部执行' }]);
  });
});
