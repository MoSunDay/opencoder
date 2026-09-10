// mono.js — the ONE monospace stack for raw identifiers (ids, uuids, session
// keys, commands, code). Pure data, no React: theme.js imports it so the
// antd side and the CSS side cannot fork, and components import MONO_VAR.
//
// Before this module the console carried five spellings of the same intent
// (three local `MONO` constants, inline literals, `var(--oc-mono, ...)` and
// bare `'monospace'`), so an id column could render in a different face
// depending on which panel you were looking at.

/// Canonical stack. Kept identical to the `--oc-mono` custom property in
/// app.css (theme.test.js asserts the equality after whitespace
/// normalization), and deliberately broader than the old JS constants: it
/// covers macOS ('SF Mono') and Linux ('Liberation Mono'), where the console
/// actually runs.
export const MONO = `ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas,
    'Liberation Mono', monospace`;

/// Inline-style value: resolve through the custom property so a future
/// retune is a one-line CSS edit. The fallback is bare `monospace` because a
/// var() fallback cannot itself contain the commas of a font list.
export const MONO_VAR = 'var(--oc-mono, monospace)';
