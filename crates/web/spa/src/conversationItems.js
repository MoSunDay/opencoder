// conversationItems.js — pure mapping from chat dialog rows to
// @ant-design/x Conversations items. Kept DOM-free so the whole contract is
// guarded by a node-env unit suite (conversationItems.test.js).
//
// Keying contract: the Conversations key IS the session id — item clicks feed
// straight into openDialog(session_id), and activeKey is the panel's
// dialogSel. Rows without a session_id have no identity to open, so they are
// skipped (both dialog sources — GET /api/sessions?limit=50 and
// GET /api/nodes/:id/dialogs — always emit session_id; the skip only guards
// against dirty/partial rows). Labels come from the frozen dialogLabel
// helper: title wins, else id head + …, else '(untitled)'.

import { dialogLabel } from './format.js';

export function dialogsToItems(dialogs) {
  return (Array.isArray(dialogs) ? dialogs : [])
    .filter((d) => d && d.session_id)
    .map((d) => ({ key: d.session_id, label: dialogLabel(d) }));
}
