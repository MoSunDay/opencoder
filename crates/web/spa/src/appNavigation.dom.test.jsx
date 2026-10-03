// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { App, LoginModal, bootUrlCredential, clearCredentials, embeddedBase, getState, setCredentials, setState, HEADERLESS_REASONS, NAV_CATEGORIES, ConfigProvider, Popconfirm, zhCN, dayjs, theme, installFetchRouter, jsonResponse, deprecationHits, mountApp } from './test/appShell.jsx';

describe('App shell navigation and layout', () => {
  it('renders antd built-ins in zh-CN under the shell ConfigProvider', async () => {
    // main.jsx wraps the whole shell (login Modal included) in
    // <ConfigProvider theme={theme} locale={zhCN}>. Its default zh-CN button
    // texts only surface inside built-in popups (Popconfirm/Modal), none of
    // which are reachable in a gated shell — so this probe mounts one under
    // the exact same theme + locale imports and pins the default texts.
    render(
      <ConfigProvider theme={theme} locale={zhCN}>
        <Popconfirm title="确认删除？" open>
          <span>probe</span>
        </Popconfirm>
      </ConfigProvider>
    );
    // antd inserts a space between the two CJK glyphs of each default
    // button (autoInsertSpaceInButton), so match with \s* like 连 接 above.
    expect(await screen.findByRole('button', { name: /确\s*定/ })).toBeTruthy();
    expect(screen.getByRole('button', { name: /取\s*消/ })).toBeTruthy();
    // dayjs locale is a main.jsx module side effect (relative dates arrive
    // in a later iteration); importing the shell must have switched it.
    expect(dayjs.locale()).toBe('zh-cn');
  });

  it('requires an explicit execution node on the chat page', async () => {
    setCredentials('smoke-token', '');
    setState({ page: 'chat' });
    await mountApp();
    expect(await screen.findByText(/选中节点后输入提示词，即新建对话/)).toBeTruthy();
    expect(screen.getByText('请先选择执行节点')).toBeTruthy();
    expect(screen.getByPlaceholderText('输入提示词，Enter 发送，Shift+Enter 换行').disabled).toBe(true);
  });

  it('docks the chat sheet to the viewport bottom: flush pane class + sheet wrapper', async () => {
    setCredentials('smoke-token', '');
    setState({ page: 'chat' });
    await mountApp();
    expect(await screen.findByText(/选中节点后输入提示词，即新建对话/)).toBeTruthy();
    // SHEET_PAGES pairing: the pane drops its bottom padding and the panel is
    // wrapped in the white sheet that squares off against the screen edge.
    expect(document.querySelector('.fleet-content').className).toContain('fleet-content--flush');
    expect(document.querySelector('.fleet-content > .fleet-sheet')).toBeTruthy();
  });

  it('keeps the padded pane on card-bearing pages (no flush, no sheet)', async () => {
    setCredentials('smoke-token', '');
    setState({ page: 'nodes' });
    await mountApp();
    expect(await screen.findByText('暂无 Opencoder 节点')).toBeTruthy();
    expect(document.querySelector('.fleet-content').className).not.toContain('fleet-content--flush');
    expect(document.querySelector('.fleet-content > .fleet-sheet')).toBeNull();
  });

  it('renders the brand and the node-category menu on the default page', async () => {
    setCredentials('smoke-token', '');
    await mountApp();
    await screen.findByText('smoke · 管理员');
    expect(screen.getByText(/Opencoder Fleet/)).toBeTruthy();
    // Default page (nodes) scopes the Sider menu to the node category. Icon
    // glyphs carry their own aria-label, so match menuitem names by regex.
    expect(screen.getByRole('menuitem', { name: /节点列表/ })).toBeTruthy();
    // Pages of other categories stay out of the scoped menu. The chat page's
    // nav label is now「Agent」(renamed from Operator), so both names must be
    // absent here.
    expect(screen.queryByRole('menuitem', { name: /Operator/ })).toBeNull();
    expect(screen.queryByRole('menuitem', { name: /Agent/ })).toBeNull();
    expect(screen.queryByRole('menuitem', { name: /DAG 工作流/ })).toBeNull();
  });

  it('drops the retired desktop Segmented nav from the Content top', async () => {
    setCredentials('smoke-token', '');
    await mountApp();
    await screen.findByText('smoke · 管理员');
    // The old always-on page Segmented (fleet-desktop-nav) is gone for good.
    expect(document.querySelector('.fleet-desktop-nav')).toBeNull();
    const categories = document.querySelector('.fleet-content .fleet-mobile-nav[role="tablist"]');
    expect(categories).toBeTruthy();
    expect(within(categories).getByRole('tab', { name: '节点', selected: true })).toBeTruthy();
  });

  it('names every menu-only page in the mobile page Select', async () => {
    // 'menu-only' pages have no page header and no body title: the sidebar
    // Menu item and, on a narrow viewport, the mobile Select label are the
    // ONLY page names left — this case guards BOTH surfaces. Delete either
    // one (or stop feeding it nav labels) and those pages go nameless with
    // every other suite still green — headerContract.dom.test.jsx can only
    // see the nav copy, not these DOM surfaces (it renders panels, not
    // <App/>).
    // Coupling note: this case mounts the real team/topics/chat/nodes panels,
    // so a failure to MOUNT a panel (or an antd deprecation reported by the
    // file-level afterEach) is not a navigation-contract failure — read the
    // panel's own suite first.
    setCredentials('smoke-token', '');
    const menuOnly = Object.keys(HEADERLESS_REASONS)
      .filter((page) => HEADERLESS_REASONS[page] === 'menu-only');
    expect(menuOnly.length, 'no page declares the menu-only reason any more — delete this case rather than loosening it').toBeGreaterThan(0);
    const labelOf = (page) => NAV_CATEGORIES.flatMap((c) => c.items)
      .find((i) => i.page === page)?.menu || '';
    // None of today's menu-only labels carries a regex special, but escape
    // anyway so a future label still matches literally.
    const escapeRegExp = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

    for (const page of menuOnly) {
      cleanup();
      setState({ page });
      await mountApp();
      await act(async () => {});
      const select = document.querySelector('.fleet-content .ant-select.fleet-mobile-nav');
      expect(select, `mobile page Select missing while on ${page}`).toBeTruthy();
      // antd 6 renders the picked label into .ant-select-content (antd 5 used
      // .ant-select-selection-item) — accept either so the guard outlives a
      // class rename, but never an empty one.
      const shown = select.querySelector('.ant-select-content, .ant-select-selection-item');
      expect(
        shown?.textContent,
        `mobile page Select shows no label for ${page} (expected ${labelOf(page)})`,
      ).toBe(labelOf(page));
      // The sidebar half of the promise: the Sider Menu is scoped to the
      // page's category and icon glyphs add their own aria-label, so match
      // by regex within the sider. Anchored at the end: the chat page label
      // 「Agent」is a prefix of「Agent 配置」, and both live in the same scoped
      // menu — an unanchored match would be ambiguous.
      const sider = within(document.querySelector('.fleet-sidebar'));
      expect(
        sider.queryByRole('menuitem', { name: new RegExp(escapeRegExp(labelOf(page)) + '$') }),
        `sidebar Menu has no item for ${page} (expected ${labelOf(page)})`,
      ).toBeTruthy();
    }
  });

  it('switches categories via the Sider Segmented and lands on the home page', async () => {
    setCredentials('smoke-token', '');
    await mountApp();
    await screen.findByText('smoke · 管理员');
    // Scope to the Sider Segmented — its mobile twin also sits in the DOM
    // (hidden only by the media query), so unscoped text queries would hit
    // both.
    const sider = within(document.querySelector('.fleet-sidebar'));
    fireEvent.click(sider.getByText('Agent'));
    // A category click navigates to its first page (brain)…
    expect(getState().page).toBe('brain');
    // …re-scopes + highlights the menu item…
    const brainItem = screen.getByRole('menuitem', { name: /大脑调度/ });
    expect(brainItem.classList.contains('ant-menu-item-selected')).toBe(true);
    // …and the panel behind it renders (agent-category page).
    expect(await screen.findByText('工作台')).toBeTruthy();
  });

  it('scopes the project category to its workbench', async () => {
    setCredentials('smoke-token', '');
    await mountApp();
    await screen.findByText('smoke · 管理员');
    const sider = within(document.querySelector('.fleet-sidebar'));
    fireEvent.click(sider.getByText('项目'));
    expect(getState().page).toBe('project');
    // Menuitem names carry the icon glyph's own aria-label → match by regex
    // (same convention as the surrounding landmark assertions).
    expect(await screen.findByRole('menuitem', { name: /项目/ })).toBeTruthy();
    expect(screen.queryByRole('menuitem', { name: /大脑调度/ })).toBeNull();
    expect(screen.queryByRole('menuitem', { name: /节点列表/ })).toBeNull();
  });
});
