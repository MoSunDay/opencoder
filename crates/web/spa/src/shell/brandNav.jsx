// brandNav.jsx — the header wordmark is the entry to the COMPLETE navigation
// menu: hovering or clicking the brand button opens one popover holding every
// visible category (group heading = category.label) and page. Picking an item
// closes the panel, then navigates through onNavigate — the same explicit
// goPage path the Sider Menu and the mobile tabs/Select use.
//
// The Sider stays scoped to the active category and mobile keeps its in-content
// tabs + Select (main.jsx). Only the panel content depends on nav.js, so this
// component carries no navigation state beyond the open/closed flag.

import { DownOutlined } from '@ant-design/icons';
import { Menu, Popover } from 'antd';
import { useState } from 'react';
import { groupedMenuItemsOf } from '../nav.js';

export function BrandNavMenu({ categories, selectedPage, onNavigate }) {
  const [open, setOpen] = useState(false);

  /// Close first so the popover never lingers over the page being opened.
  const choosePage = (page) => {
    setOpen(false);
    onNavigate(page);
  };

  const panel = (
    <nav className="fleet-nav-panel" aria-label="完整导航">
      <Menu
        mode="inline"
        selectedKeys={[selectedPage]}
        items={groupedMenuItemsOf(categories)}
        onClick={({ key }) => choosePage(key)}
        style={{ borderInlineEnd: 0 }}
      />
    </nav>
  );

  return (
    <Popover
      open={open}
      onOpenChange={setOpen}
      trigger={['hover', 'click']}
      placement="bottomLeft"
      arrow={false}
      destroyOnHidden
      classNames={{ root: 'fleet-brand-popover' }}
      content={panel}
    >
      <button
        type="button"
        className="fleet-brand"
        aria-label="打开完整导航"
        aria-haspopup="menu"
        aria-expanded={open}
      >
        ⛵ Opencoder Fleet
        <DownOutlined className="fleet-brand-caret" aria-hidden="true" />
      </button>
    </Popover>
  );
}
