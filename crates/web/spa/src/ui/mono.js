// mono.js - the ONE monospace stack for raw identifiers (ids, uuids, session
// keys, commands, code). Pure data, no React: theme.js imports it so the
// antd side and the CSS side cannot fork, and components import MONO_VAR.
//
// Before this module the console carried five spellings of the same intent
// (three local `MONO` constants, inline literals, `var(--oc-mono, ...)` and
// bare `'monospace'`), so an id column could render in a different face
// depending on which panel you were looking at.

/// Canonical stack. Kept identical to the `--oc-mono` custom property in
/// src/styles/tokens.css (mono.test.js asserts the equality after whitespace
/// normalization). Latin monospace faces first (macOS 'SF Mono', Windows
/// Consolas, Linux 'Liberation Mono'), then the CJK faces: monospace fonts
/// carry no Chinese glyphs, so without an explicit CJK fallback a Chinese
// identifier falls to the browser default (often a serif). The proportional
/// CJK faces only ever supply the CJK glyphs - Latin text has already
/// matched an earlier face.
export const MONO = `ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas,
    'Liberation Mono', 'Noto Sans Mono CJK SC', 'PingFang SC', 'Microsoft YaHei', monospace`;

/// Inline-style value: resolve through the custom property so a future
/// retune is a one-line CSS edit. The fallback is bare `monospace` because a
/// var() fallback cannot itself contain the commas of a font list.
export const MONO_VAR = 'var(--oc-mono, monospace)';
