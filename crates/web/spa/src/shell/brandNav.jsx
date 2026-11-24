// brandNav.jsx — the header wordmark is the entry to the COMPLETE navigation
// menu: hovering or clicking the brand button opens one popover holding every
// visible category (group heading = category.label) and page. Picking an item
// closes the panel, returns focus to the trigger, then navigates through
// onNavigate — the same explicit goPage path the Sider Menu and the mobile
// tabs/Select use.
//
// Keyboard / screen-reader contract (low-risk, on top of antd Popover):
//   - the button names itself with the visible wordmark and links to the menu
//     through aria-controls while open;
//   - ArrowDown opens the panel, or moves into the first item when already
//     open; keyboard/AT activation (click detail 0) also moves focus inside;
//   - hovering never steals focus;
//   - Escape from the panel or the open button closes it and refocuses the
//     wordmark;
//   - each menuitem carries aria-current="page" for the active page.
//
// The Sider stays scoped to the active category and mobile keeps its in-content
// tabs + Select (main.jsx). Only the panel content depends on nav.js, so this
// component carries no navigation state beyond the open/closed flag.

import { DownOutlined } from '@ant-design/icons';
import { Menu, Popover } from 'antd';
import { useEffect, useRef, useState } from 'react';
import { groupedMenuItemsOf, withCurrentPage } from '../nav.js';

const MENU_ID = 'fleet-brand-menu';
const COARSE_POINTER_QUERY = '(pointer: coarse)';

/// Guarded probe: jsdom (tests) and older browsers may not implement
/// matchMedia, and an unknown pointer stays on the hover-friendly default.
function isCoarsePointer() {
  try {
    return Boolean(window.matchMedia?.(COARSE_POINTER_QUERY)?.matches);
  } catch {
    return false;
  }
}

/// The panel renders through a portal, so resolve it from the document by its
/// stable id instead of a component ref.
function focusFirstMenuItem() {
  const first = document.getElementById(MENU_ID)?.querySelector('[role="menuitem"]');
  first?.focus();
  return Boolean(first);
}

export function BrandNavMenu({ categories, selectedPage, onNavigate }) {
  const [open, setOpen] = useState(false);
  // Resolve the input capability once per mount; the pointer type does not
  // change while the shell is open.
  const [coarsePointer] = useState(() => isCoarsePointer());
  const brandRef = useRef(null);
  // Set by keyboard/AT opens only: pointer/hover opens must keep focus where
  // the user left it.
  const focusOnOpenRef = useRef(false);
  const focusTimerRef = useRef(null);

  /// Move focus into the first menuitem once it exists. The panel mounts
  /// through a portal (and antd's own click handler may flush the open state
  /// before our trigger handlers run), so retry for a bounded number of ticks
  /// instead of trusting a single post-commit pass.
  const scheduleMenuFocus = () => {
    focusOnOpenRef.current = true;
    clearTimeout(focusTimerRef.current);
    let attempts = 0;
    const attempt = () => {
      if (focusFirstMenuItem() || attempts >= 5) {
        focusOnOpenRef.current = false;
        return;
      }
      attempts += 1;
      focusTimerRef.current = setTimeout(attempt, 16);
    };
    attempt();
  };

  useEffect(() => () => clearTimeout(focusTimerRef.current), []);

  /// Close first so the popover never lingers over the page being opened,
  /// then hand focus back to the trigger before navigating.
  const choosePage = (page) => {
    clearTimeout(focusTimerRef.current);
    focusOnOpenRef.current = false;
    setOpen(false);
    brandRef.current?.focus();
    onNavigate(page);
  };

  const closeAndRestoreFocus = () => {
    clearTimeout(focusTimerRef.current);
    focusOnOpenRef.current = false;
    setOpen(false);
    brandRef.current?.focus();
  };

  const handleOpenChange = (nextOpen) => {
    if (!nextOpen) {
      clearTimeout(focusTimerRef.current);
      focusOnOpenRef.current = false;
    }
    setOpen(nextOpen);
  };

  const handleTriggerClick = (event) => {
    // Keyboard activation (Enter/Space) and assistive technology report
    // detail 0; pointer clicks never do. An open panel keeps the Popover's
    // normal toggle-to-close behavior.
    if (event.detail !== 0 || open) return;
    scheduleMenuFocus();
  };

  const handleTriggerKeyDown = (event) => {
    if (event.key === 'Escape' && open) {
      event.preventDefault();
      closeAndRestoreFocus();
      return;
    }
    if (event.key !== 'ArrowDown') return;
    event.preventDefault();
    if (open) {
      focusFirstMenuItem();
      return;
    }
    setOpen(true);
    scheduleMenuFocus();
  };

  const handlePanelKeyDown = (event) => {
    if (event.key !== 'Escape') return;
    event.stopPropagation();
    closeAndRestoreFocus();
  };

  // Safety net for opens driven by state (e.g. Popover toggling before the
  // keyboard trigger handler ran): only keyboard-requested opens focus.
  useEffect(() => {
    if (open && focusOnOpenRef.current) scheduleMenuFocus();
  }, [open]);

  const panel = (
    <nav className="fleet-nav-panel" aria-label="完整导航" onKeyDown={handlePanelKeyDown}>
      <Menu
        id={MENU_ID}
        mode="inline"
        selectedKeys={[selectedPage]}
        items={withCurrentPage(groupedMenuItemsOf(categories), selectedPage)}
        onClick={({ key }) => choosePage(key)}
        style={{ borderInlineEnd: 0 }}
      />
    </nav>
  );

  return (
    <Popover
      open={open}
      onOpenChange={handleOpenChange}
      trigger={coarsePointer ? ['click'] : ['hover', 'click']}
      placement="bottomLeft"
      arrow={false}
      destroyOnHidden
      classNames={{ root: 'fleet-brand-popover' }}
      content={panel}
    >
      <button
        ref={brandRef}
        type="button"
        className="fleet-brand"
        aria-label="Opencoder Fleet：打开完整导航"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? MENU_ID : undefined}
        onClick={handleTriggerClick}
        onKeyDown={handleTriggerKeyDown}
      >
        <span aria-hidden="true">⛵</span> Opencoder Fleet
        <DownOutlined className="fleet-brand-caret" aria-hidden="true" />
      </button>
    </Popover>
  );
}
