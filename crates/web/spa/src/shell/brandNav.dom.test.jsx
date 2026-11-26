// @vitest-environment jsdom
// Header brand menu contract: the wordmark is a real button that opens ONE
// popover listing ONLY the first-level tabs (one per category: icon + name,
// no child pages) on hover or click; picking a tab closes the panel and
// navigates to that category's home page. The pages of the active category
// stay in the Sider Menu (appNavigation.dom.test.jsx).

import '../test/setup-dom.js';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { NAV_CATEGORIES, categoryHome } from '../nav.js';
import { BrandNavMenu } from './brandNav.jsx';

const CATEGORY_LABELS = NAV_CATEGORIES.map((category) => category.label);
// Page labels belong to the Sider / mobile Select and must never show up as
// brand tabs. The chat page label collides with the "Agent" category name,
// so it is checked through the exact tab count instead of by name.
const PAGE_LABELS = NAV_CATEGORIES.flatMap((category) => category.items.map((item) => item.menu))
  .filter((label) => !CATEGORY_LABELS.includes(label));
const escapeLabel = (label) => label.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
const BRAND_NAME = /Opencoder Fleet.*打开一级导航/;
const PANEL_NAME = '一级导航';

function mountMenu(selectedPage = 'nodes') {
  const onNavigate = vi.fn();
  render(<BrandNavMenu categories={NAV_CATEGORIES} selectedPage={selectedPage} onNavigate={onNavigate} />);
  const brand = screen.getByRole('button', { name: BRAND_NAME });
  return { brand, onNavigate };
}

async function openByHover(brand) {
  fireEvent.mouseEnter(brand);
  return screen.findByRole('navigation', { name: PANEL_NAME });
}

/// The popover holds exactly the category tabs and nothing else: one flat
/// menuitem per category (in IA order), no group headings, no page labels.
function expectOnlyCategoryTabs(panel) {
  const scoped = within(panel);
  const items = scoped.getAllByRole('menuitem');
  expect(items).toHaveLength(CATEGORY_LABELS.length);
  CATEGORY_LABELS.forEach((label, index) => {
    expect(scoped.getByRole('menuitem', { name: label }), `missing category tab ${label}`).toBe(items[index]);
    const icon = items[index].querySelector('.anticon');
    expect(icon, `category tab ${label} has no icon`).toBeTruthy();
    expect(icon.getAttribute('aria-hidden')).toBe('true');
  });
  for (const page of PAGE_LABELS) {
    expect(scoped.queryByRole('menuitem', { name: new RegExp(`${escapeLabel(page)}$`) }), `page label leaked into the brand menu: ${page}`).toBeNull();
  }
  expect(panel.querySelectorAll('.ant-menu-item-group, .ant-menu-item-group-title')).toHaveLength(0);
  return items;
}

describe('header brand menu', () => {
  it('renders the wordmark as a menu trigger button', () => {
    const { brand } = mountMenu();
    expect(brand.tagName).toBe('BUTTON');
    expect(brand.getAttribute('type')).toBe('button');
    expect(brand.classList.contains('fleet-brand')).toBe(true);
    expect(brand.getAttribute('aria-haspopup')).toBe('menu');
    expect(brand.getAttribute('aria-expanded')).toBe('false');
    expect(brand.textContent).toContain('Opencoder Fleet');
    expect(brand.textContent).toContain('⛵');
    // Closed: no panel is mounted before the first open.
    expect(document.querySelector('.fleet-nav-panel')).toBeNull();
  });

  it('opens the first-level tabs on hover: exactly the four categories, no child pages', async () => {
    const { brand } = mountMenu('nodes');
    expect(brand.getAttribute('aria-expanded')).toBe('false');
    const panel = await openByHover(brand);
    expect(panel.classList.contains('fleet-nav-panel')).toBe(true);
    expect(panel.closest('.ant-popover').classList.contains('fleet-brand-popover')).toBe(true);
    expect(brand.getAttribute('aria-expanded')).toBe('true');
    expect(document.querySelectorAll('.fleet-nav-panel')).toHaveLength(1);

    const items = expectOnlyCategoryTabs(panel);
    expect(items.map((item) => item.textContent)).toEqual(CATEGORY_LABELS);
    // The current page (nodes → 后台管理) highlights its category tab.
    expect(within(panel).getByRole('menuitem', { name: '后台管理' }).classList.contains('ant-menu-item-selected')).toBe(true);
    expect(within(panel).getByRole('menuitem', { name: '项目' }).classList.contains('ant-menu-item-selected')).toBe(false);
    // Hover is not an explicit keyboard/AT open: focus stays where it was.
    expect(document.activeElement).toBe(document.body);
  });

  it('opens on click (touch/keyboard), navigates to the category home and closes', async () => {
    const { brand, onNavigate } = mountMenu('nodes');
    fireEvent.click(brand);
    const panel = await screen.findByRole('navigation', { name: PANEL_NAME });
    fireEvent.click(within(panel).getByRole('menuitem', { name: 'Agent' }));
    expect(onNavigate).toHaveBeenCalledTimes(1);
    // A tab navigates to its category's home page, never to a page chosen
    // inside the panel (there are none).
    expect(onNavigate).toHaveBeenCalledWith(categoryHome('agent'));
    expect(onNavigate).toHaveBeenCalledWith('brain');
    // Choosing a tab restores focus to the trigger before navigating.
    expect(document.activeElement).toBe(brand);
    await waitFor(() => expect(brand.getAttribute('aria-expanded')).toBe('false'));
    await waitFor(() => expect(screen.queryByRole('navigation', { name: PANEL_NAME })).toBeNull(), { timeout: 2500 });
  });

  it('closes a hover-opened panel after choosing a tab and lands on that category home', async () => {
    const { brand, onNavigate } = mountMenu('nodes');
    const panel = await openByHover(brand);
    fireEvent.click(within(panel).getByRole('menuitem', { name: '项目' }));
    expect(onNavigate).toHaveBeenCalledTimes(1);
    expect(onNavigate).toHaveBeenCalledWith(categoryHome('project'));
    expect(onNavigate).toHaveBeenCalledWith('project');
    await waitFor(() => expect(brand.getAttribute('aria-expanded')).toBe('false'));
    await waitFor(() => expect(screen.queryByRole('navigation', { name: PANEL_NAME })).toBeNull(), { timeout: 2500 });
  });

  it('honors the permission-filtered category list it is handed', async () => {
    const onNavigate = vi.fn();
    render(
      <BrandNavMenu
        categories={NAV_CATEGORIES.filter((category) => category.key !== 'admin')}
        selectedPage="dag"
        onNavigate={onNavigate}
      />
    );
    const panel = await openByHover(screen.getByRole('button', { name: BRAND_NAME }));
    const scoped = within(panel);
    expect(scoped.getAllByRole('menuitem').map((item) => item.textContent)).toEqual(['项目', 'Agent', 'Ontology']);
    expect(scoped.queryByRole('menuitem', { name: '后台管理' })).toBeNull();
    expect(panel.querySelectorAll('.ant-menu-item-group-title')).toHaveLength(0);
    // A non-home page still resolves to its category: dag → Agent tab current.
    expect(scoped.getByRole('menuitem', { name: 'Agent' }).getAttribute('aria-current')).toBe('page');
    expect(scoped.getByRole('menuitem', { name: '项目' }).getAttribute('aria-current')).toBeNull();
  });

  it('moves focus into the first menu item on keyboard open and restores it on Escape', async () => {
    const { brand } = mountMenu('nodes');
    fireEvent.click(brand, { detail: 0 });
    const panel = await screen.findByRole('navigation', { name: PANEL_NAME });
    const firstItem = within(panel).getAllByRole('menuitem')[0];
    await waitFor(() => expect(document.activeElement).toBe(firstItem));

    fireEvent.keyDown(firstItem, { key: 'Escape' });
    await waitFor(() => expect(document.activeElement).toBe(brand));
    expect(brand.getAttribute('aria-expanded')).toBe('false');

    // ArrowDown on the closed trigger opens and moves into the menu again.
    fireEvent.keyDown(brand, { key: 'ArrowDown' });
    const reopened = await screen.findByRole('navigation', { name: PANEL_NAME });
    await waitFor(() => expect(document.activeElement).toBe(within(reopened).getAllByRole('menuitem')[0]));

    // Escape on the open trigger is the same close-and-restore path.
    fireEvent.keyDown(brand, { key: 'Escape' });
    await waitFor(() => expect(brand.getAttribute('aria-expanded')).toBe('false'));
  });

  it('links the trigger to the menu and marks the current category tab', async () => {
    const { brand } = mountMenu('nodes');
    expect(brand.getAttribute('aria-controls')).toBeNull();
    fireEvent.click(brand);
    const panel = await screen.findByRole('navigation', { name: PANEL_NAME });
    expect(brand.getAttribute('aria-controls')).toBe('fleet-brand-menu');
    const menu = document.getElementById('fleet-brand-menu');
    expect(menu).toBeTruthy();
    expect(menu.getAttribute('role')).toBe('menu');
    // nodes belongs to 后台管理: that tab is current, every other tab is not.
    expect(within(menu).getByRole('menuitem', { name: '后台管理' }).getAttribute('aria-current')).toBe('page');
    expect(within(menu).getByRole('menuitem', { name: '项目' }).getAttribute('aria-current')).toBeNull();
    expect(within(panel).getAllByRole('menuitem').filter((item) => item.getAttribute('aria-current') === 'page')).toHaveLength(1);
    expectOnlyCategoryTabs(panel);
  });

  it('opens on click but not on hover when the pointer is coarse', async () => {
    const matchMedia = vi.spyOn(window, 'matchMedia').mockImplementation((query) => ({
      matches: query === '(pointer: coarse)',
      media: String(query),
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: () => {},
      removeEventListener: () => {},
      dispatchEvent: () => false,
    }));
    try {
      const { brand } = mountMenu('nodes');
      fireEvent.mouseEnter(brand);
      // Wait past any hover-open delay: a coarse pointer must stay closed.
      await act(async () => { await new Promise((resolve) => { setTimeout(resolve, 250); }); });
      expect(brand.getAttribute('aria-expanded')).toBe('false');
      expect(document.querySelector('.fleet-nav-panel')).toBeNull();

      fireEvent.click(brand);
      expect(await screen.findByRole('navigation', { name: PANEL_NAME })).toBeTruthy();
    } finally {
      matchMedia.mockRestore();
    }
  });
});
