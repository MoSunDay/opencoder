// pageShell.jsx — the page header of the menu pages nav.js PAGE_META carries
// copy for: Typography Title + secondary description with an `extra` actions
// slot on the right, then the page body. Layout lives in app.css (.oc-page-*);
// this component carries no colors.
//
// It is NOT "the header of every menu page". A `page` key with no PAGE_META
// entry renders no header at all — only the bare `.oc-page` body (plus the
// `extra` row when one is passed). The header-rendering mount points are
// exactly the PAGE_META pages (project/project.jsx `project`,
// project/progressPanel.jsx `progress`, project/ownerViewPanel.jsx
// `ownerview`); the remaining mount points (authoritative list:
// `grep -rn '<PageShell' src`) are titleless wrappers whose page name comes
// from the panel's own antd Tabs ('body-title') or from the sidebar Menu /
// mobile Select label ('menu-only') — nav.js HEADERLESS_REASONS records
// which, and shell/headerContract.dom.test.jsx mounts every panel to keep
// that registry truthful:
//   fleet/teams.jsx (team), fleet/nodes.jsx (nodes),
//   fleet/executions.jsx (topics), agentsConfig.jsx (agents),
//   brain/workbench/index.jsx (brain), envs/todoPanel.jsx (extra row only).

import { Typography } from 'antd';
import { PAGE_META } from '../nav.js';

const { Title, Text } = Typography;

/// PageShell({ page, extra, bare, children })
/// - page: PAGE_META key; a key with no entry renders no header (bare body).
/// - extra: right-aligned actions slot (buttons / selects). On its own it
///   still produces a header row — all a titleless wrapper like
///   envs/todoPanel.jsx wants is that action row.
/// - bare: skip the header entirely (sub-pages / special layouts). No
///   production caller today; kept as the escape hatch and pinned against a
///   real PAGE_META page by pageShell.dom.test.jsx so it cannot rot into a
///   green-but-vacuous prop.
/// - desc is hidden when PAGE_META carries none.
export function PageShell({ page, extra, bare, children }) {
  if (bare) {
    return <>{children}</>;
  }
  const meta = (page && PAGE_META[page]) || {};
  const hasHeader = Boolean(meta.title || extra);
  return (
    <div className="oc-page">
      {hasHeader ? (
        <div className="oc-page-header">
          <div className="oc-page-heading">
            {meta.title ? <Title level={4} className="oc-page-title">{meta.title}</Title> : null}
            {meta.desc ? <Text type="secondary" className="oc-page-desc">{meta.desc}</Text> : null}
          </div>
          {extra ? <div className="oc-page-extra">{extra}</div> : null}
        </div>
      ) : null}
      {children}
    </div>
  );
}
