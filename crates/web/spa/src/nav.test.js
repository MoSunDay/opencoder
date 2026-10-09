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
  HEADERLESS_PAGES,
  HEADERLESS_REASONS,
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

describe('categoryOf', () => {
  it('maps every declared page back to its own category', () => {
    for (const item of ALL_ITEMS) {
      expect(categoryOf(item.page)).toBe(item.category);
    }
  });

  it('falls back to the default category for unknown pages', () => {
    expect(DEFAULT_CATEGORY).toBe('admin');
    expect(categoryOf('nope')).toBe('admin');
    expect(categoryOf('')).toBe('admin');
    expect(categoryOf(undefined)).toBe('admin');
    // Case-sensitive: not a nodes alias.
    expect(categoryOf('Nodes')).toBe('admin');
  });
});

describe('categoryHome / pagesOf', () => {
  it('returns the first page of each category', () => {
    expect(categoryHome('project')).toBe('project');
    expect(categoryHome('agent')).toBe('brain');
    expect(categoryHome('admin')).toBe('nodes');
  });

  it('falls back to the default category home for unknown keys', () => {
    expect(categoryHome('nope')).toBe(DEFAULT_PAGE);
  });

  it('lists category pages in menu order', () => {
    expect(pagesOf('project')).toEqual(['project']);
    // schedules（调度）挂在「全部执行」之后：cron 台账是执行面的时间维度。
    expect(pagesOf('agent')).toEqual(['brain', 'topics', 'schedules', 'dag', 'todos', 'team', 'chat', 'agents']);
    expect(pagesOf('admin')).toEqual(['nodes', 'users', 'tokens']);
  });
});

describe('menuOf / selectOptionsOf scoping', () => {
  it('builds antd Menu items keyed by page with matching labels', () => {
    const items = menuOf('admin');
    expect(items.map((i) => i.key)).toEqual(['nodes', 'users', 'tokens']);
    expect(items.map((i) => i.label)).toEqual(['节点列表', '用户权限', 'Token 管理']);
    expect(items.every((i) => isValidElement(i.icon))).toBe(true);
  });

  it('never leaks other categories into a menu', () => {
    const labels = menuOf('project').map((i) => i.label);
    expect(labels).toEqual(['项目']);
    for (const foreign of ['大脑调度', '全部执行', '节点列表']) {
      expect(labels).not.toContain(foreign);
    }
  });

  it('mirrors the same scoping as Select options for the mobile picker', () => {
    expect(selectOptionsOf('agent')).toEqual([
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
  it('exposes the categories as tab options', () => {
    expect(CATEGORY_OPTIONS).toEqual([
      { value: 'project', label: '项目' },
      { value: 'agent', label: 'Agent' },
      { value: 'ontology', label: 'Ontology' },
      { value: 'admin', label: '后台管理' },
    ]);
  });

  it('covers every page key exactly', () => {
    const expected = [...ALL_PAGES].sort();
    expect([...Object.keys(PAGE_META), ...HEADERLESS_PAGES].sort()).toEqual(expected);
  });

  it('keeps headerless pages out of PAGE_META while they stay in the IA', () => {
    for (const page of HEADERLESS_PAGES) {
      expect(PAGE_META[page]).toBeUndefined();
      expect(ALL_PAGES).toContain(page);
    }
  });

  it('declares a headerless reason for every page missing from PAGE_META', () => {
    for (const page of HEADERLESS_PAGES) {
      expect(['body-title', 'menu-only']).toContain(HEADERLESS_REASONS[page]);
    }
    expect(Object.keys(HEADERLESS_REASONS).sort()).toEqual([...HEADERLESS_PAGES].sort());
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
    expect(allowedPages({ role })).toEqual([...pagesOf('project'), ...pagesOf('agent'), ...pagesOf('ontology')]);
    expect(menuItemsOf(visible[0].items).map((item) => item.key)).toEqual(['project']);
    expect(selectItemsOf(visible[0].items)).toEqual([{ value: 'project', label: '项目' }]);
  });
});
