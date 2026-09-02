// commandMenu.js — pure slash-command + `$skill` catalog for the composer.
// TUI parity source: the tui slash commands (act/plan/compact/model/ap/
// annotation/fork/clear_context) and the runner's TEXT control heads
// (crates/session/src/control_cmd.rs): while a drain runs, /act //plan
// travel as ordinary prompts and the runner applies them at the boundary.
// No React, no fetch — vitest exercises the filter directly.

/// The fixed `/` catalog. `kind` drives execution in chat.jsx:
///   agent      → POST /agent (busy: send the text, runner applies it)
///   text       → send as a normal prompt (runner control heads)
///   compact    → POST /compact
///   model      → open the model picker modal
///   ap         → open the autopilot picker (off/ap/review/清除)
///   annotation → open the annotation input modal
///   fork       → POST /fork, select the new session
export const COMMAND_CATALOG = [
  { cmd: '/act', desc: '切换到 act 模式', kind: 'agent', value: 'act' },
  { cmd: '/plan', desc: '切换到 plan 只读模式', kind: 'agent', value: 'plan' },
  { cmd: '/act_clear_context', desc: '清空上下文并切换 act（保留 plan）', kind: 'text' },
  { cmd: '/clear_context', desc: '清空上下文', kind: 'text' },
  { cmd: '/compact', desc: '压缩上下文', kind: 'compact' },
  { cmd: '/model', desc: '切换模型', kind: 'model' },
  { cmd: '/ap', desc: 'autopilot 模式 (off/ap/review)', kind: 'ap' },
  { cmd: '/annotation', desc: '设置批注', kind: 'annotation' },
  { cmd: '/fork', desc: 'fork 当前会话', kind: 'fork' },
];

/// GET /api/skills items → `$` entries. Disabled skills still list (the TUI
/// completes them too; the runner rejects at admission with its own error).
export function skillsToCommands(skills) {
  const list = Array.isArray(skills) ? skills : [];
  return list
    .filter((s) => s && typeof s.name === 'string' && s.name)
    .map((s) => ({ cmd: '$' + s.name, desc: String(s.description || ''), kind: 'skill', value: s.name }));
}

/// The trailing `/word` | `$word` token of the composer text, or null when
/// the text does not end in one (whitespace-terminated or plain prose).
export function lastCommandToken(text) {
  const t = typeof text === 'string' ? text : '';
  const m = t.match(/(\/|\$)([^\s]*)$/);
  if (!m) {
    return null;
  }
  return { sigil: m[1], query: m[2] };
}

/// Cap on rendered menu rows — a long `$` tail must not flood the composer.
export const MENU_CAP = 8;

/// Entries matching the LAST `/…`/`$…` token of `text`, case-insensitive
/// prefix on `cmd` (sigil included — `$deb` matches `$debug`, never `/debug`).
/// An empty query lists that sigil's whole side; both lists cap at MENU_CAP.
export function filterCommands(catalog, text) {
  const token = lastCommandToken(text);
  if (!token) {
    return [];
  }
  const needle = (token.sigil + token.query).toLowerCase();
  return (Array.isArray(catalog) ? catalog : [])
    .filter((e) => e && typeof e.cmd === 'string')
    .filter((e) => e.cmd.toLowerCase().startsWith(needle))
    .slice(0, MENU_CAP);
}

/// Catalog + skills in one call — chat.jsx's only filter entry point.
export function commandsForInput(text, skills) {
  return filterCommands(COMMAND_CATALOG.concat(skillsToCommands(skills)), text);
}

/// Menu click → new composer text. Skill entries complete the token into
/// `$name `; other entries replace it with `cmd ` so arguments can follow.
export function replaceToken(text, entry) {
  const t = typeof text === 'string' ? text : '';
  const token = lastCommandToken(t);
  if (!token) {
    return t;
  }
  const head = t.slice(0, t.length - (token.sigil + token.query).length);
  const inserted = entry && entry.kind === 'skill'
    ? '$' + String((entry && entry.value) || '')
    : String((entry && entry.cmd) || '');
  return head + inserted + ' ';
}

/// Composer text minus the trailing `/…`/`$…` token — chat.jsx clears the
/// token before executing a picked command so nothing stale is left behind.
export function stripLastToken(text) {
  const t = typeof text === 'string' ? text : '';
  const token = lastCommandToken(t);
  return token ? t.slice(0, t.length - (token.sigil + token.query).length) : t;
}
