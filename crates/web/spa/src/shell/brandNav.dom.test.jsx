// @vitest-environment jsdom
// Header brand menu contract: the wordmark is a real button that opens ONE
// popover with the complete visible IA (every category group + page) on hover
// or click; picking an item closes the panel and navigates to that page key.

import '../test/setup-dom.js';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { NAV_CATEGORIES } from '../nav.js';
import { BrandNavMenu } from './brandNav.jsx';

const ALL_ITEMS = NAV_CATEGORIES.flatMap((category) => category.items);
const escapeLabel = (label) => label.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

function mountMenu(selectedPage = 'nodes') {
  const onNavigate = vi.fn();
  render(<BrandNavMenu categories={NAV_CATEGORIES} selectedPage={selectedPage} onNavigate={onNavigate} />);
  const brand = screen.getByRole('button', { name: /Opencoder Fleet.*打开完整导航/ });
  return { brand, onNavigate };
}

async function openByHover(brand) {
  fireEvent.mouseEnter(brand);
  return screen.findByRole('navigation', { name: '完整导航' });
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

  it('opens the complete IA on hover: every category group and page', async () => {
    const { brand } = mountMenu('nodes');
    expect(brand.getAttribute('aria-expanded')).toBe('false');
    const panel = await openByHover(brand);
    expect(panel.classList.contains('fleet-nav-panel')).toBe(true);
    expect(panel.closest('.ant-popover').classList.contains('fleet-brand-popover')).toBe(true);
    expect(brand.getAttribute('aria-expanded')).toBe('true');
    expect(document.querySelectorAll('.fleet-nav-panel')).toHaveLength(1);

    const scoped = within(panel);
    for (const category of NAV_CATEGORIES) {
      const heading = scoped.getByText(category.label, { selector: '.fleet-nav-group-label' });
      expect(heading.closest('.ant-menu-item-group-title')).toBeTruthy();
      expect(heading.querySelector('.anticon')).toBeTruthy();
    }
    for (const item of ALL_ITEMS) {
      const name = new RegExp(`${escapeLabel(item.menu)}$`);
      expect(scoped.getByRole('menuitem', { name }), `missing menu item ${item.menu}`).toBeTruthy();
    }
    expect(scoped.getAllByRole('menuitem')).toHaveLength(ALL_ITEMS.length);
    // The current page is highlighted inside the complete menu.
    expect(scoped.getByRole('menuitem', { name: /节点列表$/ }).classList.contains('ant-menu-item-selected')).toBe(true);
    // Hover is not an explicit keyboard/AT open: focus stays where it was.
    expect(document.activeElement).toBe(document.body);
  });

  it('opens on click (touch/keyboard) and closes after choosing a page', async () => {
    const { brand, onNavigate } = mountMenu('nodes');
    fireEvent.click(brand);
    const panel = await screen.findByRole('navigation', { name: '完整导航' });
    fireEvent.click(within(panel).getByRole('menuitem', { name: /用户权限$/ }));
    expect(onNavigate).toHaveBeenCalledTimes(1);
    expect(onNavigate).toHaveBeenCalledWith('users');
    // Choosing a page restores focus to the trigger before navigating.
    expect(document.activeElement).toBe(brand);
    await waitFor(() => expect(brand.getAttribute('aria-expanded')).toBe('false'));
    await waitFor(() => expect(screen.queryByRole('navigation', { name: '完整导航' })).toBeNull(), { timeout: 2500 });
  });

  it('closes a hover-opened panel after choosing a page from the full IA', async () => {
    const { brand, onNavigate } = mountMenu('nodes');
    const panel = await openByHover(brand);
    fireEvent.click(within(panel).getByRole('menuitem', { name: /DAG 工作流$/ }));
    expect(onNavigate).toHaveBeenCalledWith('dag');
    await waitFor(() => expect(brand.getAttribute('aria-expanded')).toBe('false'));
    await waitFor(() => expect(screen.queryByRole('navigation', { name: '完整导航' })).toBeNull(), { timeout: 2500 });
  });

  it('honors the permission-filtered category list it is handed', async () => {
    const onNavigate = vi.fn();
    render(
      <BrandNavMenu
        categories={NAV_CATEGORIES.filter((category) => category.key !== 'admin')}
        selectedPage="project"
        onNavigate={onNavigate}
      />
    );
    const panel = await openByHover(screen.getByRole('button', { name: /Opencoder Fleet.*打开完整导航/ }));
    const scoped = within(panel);
    expect(scoped.getAllByRole('menuitem')).toHaveLength(ALL_ITEMS.filter((item) => item.page !== 'users' && item.page !== 'nodes').length);
    expect(scoped.queryByRole('menuitem', { name: /用户权限$/ })).toBeNull();
    expect(scoped.queryByText('后台管理', { selector: '.ant-menu-item-group-title' })).toBeNull();
  });

  it('moves focus into the first menu item on keyboard open and restores it on Escape', async () => {
    const { brand } = mountMenu('nodes');
    fireEvent.click(brand, { detail: 0 });
    const panel = await screen.findByRole('navigation', { name: '完整导航' });
    const firstItem = within(panel).getAllByRole('menuitem')[0];
    await waitFor(() => expect(document.activeElement).toBe(firstItem));

    fireEvent.keyDown(firstItem, { key: 'Escape' });
    await waitFor(() => expect(document.activeElement).toBe(brand));
    expect(brand.getAttribute('aria-expanded')).toBe('false');

    // ArrowDown on the closed trigger opens and moves into the menu again.
    fireEvent.keyDown(brand, { key: 'ArrowDown' });
    const reopened = await screen.findByRole('navigation', { name: '完整导航' });
    await waitFor(() => expect(document.activeElement).toBe(within(reopened).getAllByRole('menuitem')[0]));

    // Escape on the open trigger is the same close-and-restore path.
    fireEvent.keyDown(brand, { key: 'Escape' });
    await waitFor(() => expect(brand.getAttribute('aria-expanded')).toBe('false'));
  });

  it('links the trigger to the menu and marks the current page', async () => {
    const { brand } = mountMenu('nodes');
    expect(brand.getAttribute('aria-controls')).toBeNull();
    fireEvent.click(brand);
    const panel = await screen.findByRole('navigation', { name: '完整导航' });
    expect(brand.getAttribute('aria-controls')).toBe('fleet-brand-menu');
    const menu = document.getElementById('fleet-brand-menu');
    expect(menu).toBeTruthy();
    expect(menu.getAttribute('role')).toBe('menu');
    expect(within(menu).getByRole('menuitem', { name: /节点列表$/ }).getAttribute('aria-current')).toBe('page');
    expect(within(menu).getByRole('menuitem', { name: /用户权限$/ }).getAttribute('aria-current')).toBeNull();
    expect(within(panel).getAllByRole('menuitem').filter((item) => item.getAttribute('aria-current') === 'page')).toHaveLength(1);
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
      expect(await screen.findByRole('navigation', { name: '完整导航' })).toBeTruthy();
    } finally {
      matchMedia.mockRestore();
    }
  });
});
