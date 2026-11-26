// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { getState, setCredentials, setState, HEADERLESS_REASONS, NAV_CATEGORIES, ConfigProvider, Popconfirm, zhCN, dayjs, theme, mountApp } from './test/appShell.jsx';

/// The header brand opens the first-level category tabs in a portal popover
/// (picking one lands on that category's home page; a specific page is then
/// picked in the scoped Sider Menu). Every brand interaction below starts
/// from the closed state, so a click reliably opens it (hover is covered by
/// shell/brandNav.dom.test.jsx).
const BRAND_NAME = /Opencoder Fleet.*打开一级导航/;
async function openBrandMenu() {
  fireEvent.click(screen.getByRole('button', { name: BRAND_NAME }));
  return screen.findByRole('navigation', { name: '一级导航' });
}

/// The scoped Sider page Menu (main.jsx `#fleet-page-menu`).
const siderMenu = () => within(document.getElementById('fleet-page-menu'));

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
    // Brand → 项目 tab lands on the category home (Overview) and clears the link.
    const brandPanel = await openBrandMenu();
    fireEvent.click(within(brandPanel).getByRole('menuitem', { name: '项目' }));
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

  it('renders the brand trigger and the node-category menu on the default page', async () => {
    setCredentials('smoke-token', '');
    await mountApp();
    await screen.findByText('smoke · 管理员');
    const brand = screen.getByRole('button', { name: BRAND_NAME });
    expect(brand.classList.contains('fleet-brand')).toBe(true);
    expect(brand.getAttribute('aria-haspopup')).toBe('menu');
    expect(brand.textContent).toContain('Opencoder Fleet');
    // The old scrollable Sider category strip is gone; navigation now lives
    // in this brand menu (category tabs) and in the scoped Sider Menu below
    // (pages of the active category).
    expect(document.querySelector('.fleet-sidebar [role="tablist"]')).toBeNull();
    // Default page (nodes) scopes the Sider menu to the node category. Icon
    // glyphs carry their own aria-label, so match menuitem names by regex.
    expect(screen.getByRole('menuitem', { name: /节点列表/ })).toBeTruthy();
    // The Sider menu names itself after the active category and exposes the
    // current page to assistive technology (aria-current, not just styling).
    const siderMenu = document.getElementById('fleet-page-menu');
    expect(siderMenu).toBeTruthy();
    expect(siderMenu.getAttribute('role')).toBe('menu');
    expect(siderMenu.getAttribute('aria-label')).toBe('后台管理页面导航');
    expect(within(siderMenu).getByRole('menuitem', { name: /节点列表/ }).getAttribute('aria-current')).toBe('page');
    expect(within(siderMenu).getByRole('menuitem', { name: /用户权限/ }).getAttribute('aria-current')).toBeNull();
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

  it('navigates through the header brand menu to a category home and re-scopes the Sider', async () => {
    setCredentials('smoke-token', '');
    await mountApp();
    await screen.findByText('smoke · 管理员');
    const panel = await openBrandMenu();
    // The brand lists category tabs only: the Agent tab lands on the agent
    // category home (brain), a page outside the current (admin) category.
    expect(within(panel).getAllByRole('menuitem').map((item) => item.textContent)).toEqual(['项目', 'Agent', 'Ontology', '后台管理']);
    expect(within(panel).queryByRole('menuitem', { name: /大脑调度/ })).toBeNull();
    fireEvent.click(within(panel).getByRole('menuitem', { name: 'Agent' }));
    expect(getState().page).toBe('brain');
    expect(screen.getByRole('button', { name: BRAND_NAME }).getAttribute('aria-expanded')).toBe('false');
    // The Sider re-scopes + highlights the menu item (scope it: the closing
    // brand panel still carries the Agent tab label during its motion).
    const brainItem = siderMenu().getByRole('menuitem', { name: /大脑调度/ });
    expect(brainItem.classList.contains('ant-menu-item-selected')).toBe(true);
    expect(brainItem.getAttribute('aria-current')).toBe('page');
    // …and the panel behind it renders (agent-category brain page).
    expect(await screen.findByText('运行历史')).toBeTruthy();
  });

  it('scopes the project category to Overview and the standalone Tag page', async () => {
    setCredentials('smoke-token', '');
    await mountApp();
    await screen.findByText('smoke · 管理员');
    const panel = await openBrandMenu();
    fireEvent.click(within(panel).getByRole('menuitem', { name: '项目' }));
    expect(getState().page).toBe('project');
    // Menuitem names carry the icon glyph's own aria-label → match by regex
    // (same convention as the surrounding landmark assertions).
    const sider = siderMenu();
    expect(await sider.findByRole('menuitem', { name: /Overview$/ })).toBeTruthy();
    expect(sider.getByRole('menuitem', { name: /Tag$/ })).toBeTruthy();
    expect(sider.queryByRole('menuitem', { name: /大脑调度/ })).toBeNull();
    expect(sider.queryByRole('menuitem', { name: /节点列表/ })).toBeNull();
    // A specific non-home page is reached from the scoped Sider, not the brand.
    fireEvent.click(sider.getByRole('menuitem', { name: /Tag$/ }));
    expect(getState().page).toBe('projectTags');
  });
});
