// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { getState, setCredentials, setState, HEADERLESS_REASONS, NAV_CATEGORIES, ConfigProvider, Popconfirm, zhCN, dayjs, theme, mountApp } from './test/appShell.jsx';

describe('App shell navigation and layout', () => {
  it('opens an execution link before restoring a saved project page', async () => {
    window.history.replaceState(null, '', '/?execution=agent-history&execution_kind=agent');
    localStorage.setItem('oc_nav_page', JSON.stringify('project'));
    setCredentials('smoke-token', '');
    await mountApp();
    await screen.findByText('smoke · 管理员');
    expect(getState().page).toBe('topics');
    expect(screen.getByRole('menuitem', { name: /全部执行/ }).classList.contains('ant-menu-item-selected')).toBe(true);
    expect(screen.getByText('agent-history', { selector: '.ant-drawer-title' }).closest('[role="dialog"]')).toBeTruthy();
  });

  it('keeps an ID-only execution route ahead of later homepage state restoration', async () => {
    window.history.replaceState(null, '', '/#execution=operator-history');
    localStorage.setItem('oc_nav_page', JSON.stringify('project'));
    setCredentials('smoke-token', '');
    await mountApp();
    await screen.findByText('smoke · 管理员');
    act(() => setState({ page: 'project' }));
    expect(screen.getByText('operator-history', { selector: '.ant-drawer-title' })).toBeTruthy();
    expect(screen.getByRole('menuitem', { name: /全部执行/ }).classList.contains('ant-menu-item-selected')).toBe(true);
    fireEvent.click(within(document.querySelector('.fleet-sidebar')).getByRole('tab', { name: '项目', exact: true }));
    await waitFor(() => expect(window.location.hash).toBe(''));
    expect(getState().page).toBe('project');
  });

  it('opens a generic execution ID when navigation arrives after the homepage mounted', async () => {
    localStorage.setItem('oc_nav_page', JSON.stringify('project'));
    setCredentials('smoke-token', '');
    await mountApp();
    await screen.findByText('smoke · 管理员');
    act(() => {
      window.history.pushState(null, '', '/#execution=late-history');
      window.dispatchEvent(new HashChangeEvent('hashchange'));
    });
    expect(await screen.findByText('late-history', { selector: '.ant-drawer-title' })).toBeTruthy();
    expect(getState().page).toBe('topics');
  });

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
    expect(within(categories).getByRole('tab', { name: '后台管理', selected: true })).toBeTruthy();
  });

  it.each(Object.keys(HEADERLESS_REASONS).filter((page) => HEADERLESS_REASONS[page] === 'menu-only'))(
    'names %s in the mobile page Select and sidebar', async (page) => {
      // These pages rely on the sidebar and mobile selector for their name.
      setCredentials('smoke-token', '');
      const label = NAV_CATEGORIES.flatMap((c) => c.items).find((i) => i.page === page)?.menu || '';
      const escapedLabel = label.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
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
        `mobile page Select shows no label for ${page} (expected ${label})`,
      ).toBe(label);
      // Anchor the label so Agent cannot match Agent 配置.
      const sider = within(document.querySelector('.fleet-sidebar'));
      expect(
        sider.queryByRole('menuitem', { name: new RegExp(escapedLabel + '$') }),
        `sidebar Menu has no item for ${page} (expected ${label})`,
      ).toBeTruthy();
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

  it('scopes the project category to Overview and the standalone Tag page', async () => {
    setCredentials('smoke-token', '');
    await mountApp();
    await screen.findByText('smoke · 管理员');
    const sider = within(document.querySelector('.fleet-sidebar'));
    fireEvent.click(sider.getByText('项目'));
    expect(getState().page).toBe('project');
    // Menuitem names carry the icon glyph's own aria-label → match by regex
    // (same convention as the surrounding landmark assertions).
    expect(await screen.findByRole('menuitem', { name: /Overview$/ })).toBeTruthy();
    expect(screen.getByRole('menuitem', { name: /Tag$/ })).toBeTruthy();
    expect(screen.queryByRole('menuitem', { name: /大脑调度/ })).toBeNull();
    expect(screen.queryByRole('menuitem', { name: /节点列表/ })).toBeNull();
  });
});
