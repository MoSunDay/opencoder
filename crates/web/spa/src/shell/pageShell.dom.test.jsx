// @vitest-environment jsdom
// PageShell DOM contract: the unified page header is driven by nav.js
// PAGE_META (title + description), carries the right-side actions slot, and
// can skip the header entirely (bare) for sub-pages / special layouts.

import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import '../test/setup-dom.js';
import { Button } from 'antd';
import { PAGE_META } from '../nav.js';
import { PageShell } from './pageShell.jsx';

afterEach(() => cleanup());

describe('PageShell', () => {
  it('renders the PAGE_META title, description and children', () => {
    render(<PageShell page="nodes">body-content</PageShell>);
    expect(screen.getByRole('heading', { name: '节点列表' })).toBeTruthy();
    expect(screen.getByText('在线 Opencoder 节点与负载')).toBeTruthy();
    expect(screen.getByText('body-content')).toBeTruthy();
  });

  it('renders the extra actions slot on the header row', () => {
    render(<PageShell page="nodes" extra={<Button onClick={() => {}}>刷新节点</Button>} />);
    fireEvent.click(screen.getByRole('button', { name: /刷新节点/ }));
    expect(screen.getByRole('button', { name: /刷新节点/ })).toBeTruthy();
  });

  it('bare skips the header but keeps the children', () => {
    render(<PageShell page="nodes" bare>bare-content</PageShell>);
    expect(screen.queryByRole('heading')).toBeNull();
    expect(screen.queryByText('在线 Opencoder 节点与负载')).toBeNull();
    expect(screen.getByText('bare-content')).toBeTruthy();
  });

  it('unknown pages render bare content without a header', () => {
    render(<PageShell page="no_such_page">unknown-content</PageShell>);
    expect(screen.queryByRole('heading')).toBeNull();
    expect(screen.getByText('unknown-content')).toBeTruthy();
  });

  it('every PAGE_META page can mount with a heading', () => {
    Object.keys(PAGE_META).forEach((page) => {
      cleanup();
      render(<PageShell page={page}>x</PageShell>);
      expect(screen.getByRole('heading', { name: PAGE_META[page].title })).toBeTruthy();
    });
  });
});
