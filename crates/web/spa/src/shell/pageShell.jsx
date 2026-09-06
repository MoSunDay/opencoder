// pageShell.jsx — the unified page header for every menu page: Typography
// Title + secondary description from nav.js PAGE_META (the IA's single copy
// source) with an `extra` actions slot on the right, then the page body.
// Layout lives in app.css (.oc-page-*); this component carries no colors.

import { Typography } from 'antd';
import { PAGE_META } from '../nav.js';

const { Title, Text } = Typography;

/// PageShell({ page, extra, bare, children })
/// - page: PAGE_META key; unknown keys render no header (bare content).
/// - extra: right-aligned actions slot (buttons / selects).
/// - bare: skip the header entirely (sub-pages / special layouts).
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
