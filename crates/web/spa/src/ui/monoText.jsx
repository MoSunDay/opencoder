// monoText.jsx — <MonoText>, the single component for monospace identifier
// runs. Wraps the MONO_VAR convention from ./mono.js so call sites stop
// hand-writing `style={{ fontFamily: ... }}` (and stop getting it wrong).

import { MONO_VAR } from './mono.js';

/// MonoText({ as, style, children, ...rest })
/// - as: element type, defaults to 'span' ('small'/'div' where the old call
///   sites needed block or small-text semantics).
/// - style: merged over the font-family, never replacing the caller's sizing.
/// - rest passes through (className, aria-label, title, ...).
export function MonoText({ as: Tag = 'span', style, children, ...rest }) {
  return <Tag {...rest} style={{ fontFamily: MONO_VAR, ...style }}>{children}</Tag>;
}
