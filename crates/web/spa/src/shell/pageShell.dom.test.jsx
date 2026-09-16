// @vitest-environment jsdom
// PageShell DOM contract: header copy comes from nav.js PAGE_META (title +
// description), the right-side `extra` actions slot renders with or without a
// title, and `bare` skips the header entirely. A `page` key missing from
// PAGE_META is headerless by design — nav.js HEADERLESS_REASONS declares why
// each such page still has a name ('body-title' or 'menu-only'); the contract
// over the REAL panels lives in shell/headerContract.dom.test.jsx, and the
// mobile-Select half of 'menu-only' in app.dom.test.jsx.

import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import '../test/setup-dom.js';
import { Button } from 'antd';
import { HEADERLESS_PAGES, PAGE_META } from '../nav.js';
import { PageShell } from './pageShell.jsx';

afterEach(() => cleanup());

describe('PageShell', () => {
  it('renders the PAGE_META title, description and children', () => {
    render(<PageShell page="progress">body-content</PageShell>);
    expect(screen.getByRole('heading', { name: '进展' })).toBeTruthy();
    expect(screen.getByText('里程碑进度、进行中 TODO 与最近项目执行')).toBeTruthy();
    expect(screen.getByText('body-content')).toBeTruthy();
  });

  it('renders the extra actions slot on the header row', () => {
    render(<PageShell page="nodes" extra={<Button onClick={() => {}}>刷新节点</Button>} />);
    fireEvent.click(screen.getByRole('button', { name: /刷新节点/ }));
    expect(screen.getByRole('button', { name: /刷新节点/ })).toBeTruthy();
  });

  it('renders an extra-only header row (no title) outside PAGE_META', () => {
    // The titleless-wrapper shape most PageShell mount points use;
    // envs/todoPanel.jsx relies on exactly this for its 新建 action row, so it
    // passes no `page` key at all.
    const { container } = render(<PageShell extra={<Button>新建</Button>}>envs-body</PageShell>);
    expect(container.querySelector('.oc-page-header')).toBeTruthy();
    expect(container.querySelector('.oc-page-title')).toBeNull();
    // antd inserts a space between the two CJK glyphs of a 2-char button.
    expect(screen.getByRole('button', { name: /新\s*建/ })).toBeTruthy();
    expect(screen.getByText('envs-body')).toBeTruthy();
  });

  it('bare skips a header the same PAGE_META page would otherwise render', () => {
    // Anchored on a PAGE_META key on purpose: against a headerless key (the
    // old `page="nodes"`) this case passed even if `bare` was ignored, i.e. it
    // proved nothing. Assert both sides so the prop cannot rot silently.
    const withHeader = render(<PageShell page="progress">x</PageShell>);
    expect(withHeader.container.querySelector('.oc-page-title')?.textContent).toBe('进展');
    withHeader.unmount();

    render(<PageShell page="progress" bare>bare-content</PageShell>);
    expect(screen.queryByRole('heading')).toBeNull();
    expect(screen.queryByText('进展')).toBeNull();
    expect(screen.getByText('bare-content')).toBeTruthy();
  });

  it('unknown pages render bare content without a header', () => {
    render(<PageShell page="no_such_page">unknown-content</PageShell>);
    expect(screen.queryByRole('heading')).toBeNull();
    expect(screen.getByText('unknown-content')).toBeTruthy();
  });

  it('renders the headerless brain page body without any page header', () => {
    render(<PageShell page="brain">brain-body</PageShell>);
    expect(screen.queryByRole('heading')).toBeNull();
    expect(screen.getByText('brain-body')).toBeTruthy();
  });

  it('renders every headerless page body without a page header', () => {
    for (const page of HEADERLESS_PAGES) {
      cleanup();
      render(<PageShell page={page}>body</PageShell>);
      expect(screen.queryByRole('heading')).toBeNull();
      expect(screen.getByText('body')).toBeTruthy();
    }
  });

  it('every PAGE_META page can mount with a heading', () => {
    Object.keys(PAGE_META).forEach((page) => {
      cleanup();
      render(<PageShell page={page}>x</PageShell>);
      expect(screen.getByRole('heading', { name: PAGE_META[page].title })).toBeTruthy();
    });
  });
});
