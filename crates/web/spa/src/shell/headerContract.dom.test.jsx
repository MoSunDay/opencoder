// @vitest-environment jsdom
// Header contract for the fleet IA — this file mounts the REAL panel of every
// menu page, so both regression classes fail here instead of shipping:
//   1. dead config: a PAGE_META entry no panel renders (e.g. a PageShell
//      import that never got wired up) — only project / progress / ownerview
//      panels mount PageShell with their own key, so PAGE_META holds exactly
//      those;
//   2. silently lost title: a page moved into HEADERLESS_REASONS without its
//      declared reason actually holding in the DOM.
// The reasons registry in nav.js:
//   - 'body-title': the panel renders its own title (antd Tabs) in the body;
//   - 'menu-only' : full-bleed / operational page whose sidebar Menu / mobile
//                   Select label is deliberately the only title — the body
//                   must NOT render a title of its own (mutual exclusion, so
//                   flipping a reason between the two can never pass silently).
// nav.test.js only checks set consistency (PAGE_META ∪ HEADERLESS == ALL_PAGES);
// the per-page mounts below are what make the registry truthful.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, render } from '@testing-library/react';

const { apiGetMock, apiPostMock } = vi.hoisted(() => ({
  apiGetMock: vi.fn(),
  apiPostMock: vi.fn(),
}));
vi.mock('../api.js', () => ({
  apiGet: apiGetMock,
  apiPost: apiPostMock,
  apiPatch: vi.fn(),
  apiPut: vi.fn(),
  apiDel: vi.fn(),
  authFetch: vi.fn(),
}));
// Panels may open the live event stream; keep it a no-op so no fetch-retry
// timer outlives the assertions.
vi.mock('../sse.js', () => ({ openStream: vi.fn(() => ({ abort: vi.fn() })) }));

import '../test/setup-dom.js';
import {
  HEADERLESS_PAGES,
  HEADERLESS_REASONS,
  NAV_CATEGORIES,
  PAGE_META,
} from '../nav.js';
import { PANELS } from './panels.jsx';
import { PageShell } from './pageShell.jsx';

/// Every menu page of the full IA, in menu order — the same projection
/// nav.test.js derives (nav.js exports the categories, not a flat list).
const ALL_PAGES = NAV_CATEGORIES.flatMap((c) => c.items.map((i) => i.page));

const KNOWN_REASONS = ['body-title', 'menu-only'];

/// A title rendered inside the body: antd Tabs (tablist) or any heading.
/// Deliberately broader than the 'antd Tabs' wording in HEADERLESS_REASONS —
/// a panel that starts rendering any heading owes the registry a reason
/// update, and the failure messages below say so explicitly.
const bodyTitleOf = (container) => container.querySelector('.ant-tabs-nav, [role="tablist"], h1, h2, h3, h4, h5, h6');

/// The sidebar Menu / mobile Select label — the only title a 'menu-only' page
/// has. Both surfaces read nav.js `item.menu` (menuItemsOf / selectItemsOf), so
/// this covers the copy; that the mobile Select really renders it in the DOM is
/// asserted per menu-only page in app.dom.test.jsx.
const menuLabelOf = (page) => {
  for (const category of NAV_CATEGORIES) {
    const hit = category.items.find((item) => item.page === page);
    if (hit) return hit.menu;
  }
  return '';
};

/// Render a panel and flush its mount effects (loads are async, assertions are not).
const renderPanel = async (page) => {
  const Panel = PANELS[page];
  const view = render(<Panel onNotice={() => {}} />);
  await act(async () => {});
  return view;
};

beforeEach(() => {
  // Panels destructure list fields straight into state (teams.jsx does
  // `setNodes(b.nodes)`, the brain workbench does `setPlans(p.plans)`), so a
  // bare `{}` reply would crash a real panel inside nodeOptions(undefined).
  // The contract under test is the header, not how panels tolerate malformed
  // replies — hand back empty lists for every list field a panel reads.
  apiGetMock.mockResolvedValue({
    nodes: [], teams: [], agents: [], executions: [],
    plans: [], runs: [], capabilities: [], resources: [],
    goals: [], backlog: [], templates: [], workflows: [],
    dialogs: [], skills: [], sessions: [], messages: [], events: [],
  });
  apiPostMock.mockResolvedValue({});
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe('headerless reason registry', () => {
  it('declares a known reason for exactly the headerless pages', () => {
    expect(Object.keys(HEADERLESS_REASONS).sort()).toEqual([...HEADERLESS_PAGES].sort());
    for (const page of HEADERLESS_PAGES) {
      expect(KNOWN_REASONS).toContain(HEADERLESS_REASONS[page]);
    }
  });

  it('gives every non-headerless page a PAGE_META title', () => {
    for (const page of ALL_PAGES) {
      if (HEADERLESS_PAGES.includes(page)) {
        expect(PAGE_META[page]).toBeUndefined();
      }
      else {
        expect(String(PAGE_META[page]?.title || '').length).toBeGreaterThan(0);
      }
    }
  });

  it('renders the PAGE_META header for a header-bearing page', () => {
    // Key-agnostic on purpose: whichever pages PAGE_META keeps, PageShell must
    // render their title — the per-page loop below proves a real panel mounts it.
    const [page] = Object.keys(PAGE_META);
    const { container } = render(<PageShell page={page}>body</PageShell>);
    expect(container.querySelector('.oc-page-header')).toBeTruthy();
    expect(container.querySelector('.oc-page-title')?.textContent).toBe(PAGE_META[page].title);
  });
});

describe('panel registry', () => {
  it('maps every page key to a panel component and nothing else', () => {
    expect(Object.keys(PANELS).sort()).toEqual([...ALL_PAGES].sort());
    for (const page of ALL_PAGES) {
      expect(typeof PANELS[page]).toBe('function');
    }
  });
});

describe('every page keeps exactly one title source', () => {
  for (const page of ALL_PAGES) {
    it(`${page} keeps exactly one title source`, async () => {
      const { container } = await renderPanel(page);
      const meta = PAGE_META[page];
      const title = container.querySelector('.oc-page-title');
      if (meta) {
        // PAGE_META page: PageShell must really render title + desc.
        expect(title?.textContent).toBe(meta.title);
        expect(container.querySelector('.oc-page-desc')?.textContent).toBe(meta.desc);
      }
      else {
        // Headerless page: no PageShell title, and the declared reason must hold.
        expect(title).toBeNull();
        expect(KNOWN_REASONS).toContain(HEADERLESS_REASONS[page]);
        // Mutual exclusion: 'body-title' MUST render its own body title,
        // 'menu-only' MUST NOT — otherwise flipping a reason between the two
        // would be accepted silently regardless of what the panel renders.
        if (HEADERLESS_REASONS[page] === 'body-title') {
          expect(
            bodyTitleOf(container),
            `${page} declares 'body-title' but its panel renders no antd Tabs nav / heading in the body — render one, or declare 'menu-only' if the nav label is the page's only name`,
          ).toBeTruthy();
        }
        else {
          expect(
            bodyTitleOf(container),
            `${page} declares 'menu-only' (sidebar Menu + mobile Select label are the only title) but its panel renders a body title — declare 'body-title' instead`,
          ).toBeNull();
        }
      }
      // Either way the nav label names the page (sidebar Menu / mobile Select).
      expect(menuLabelOf(page).length).toBeGreaterThan(0);
    });
  }
});
