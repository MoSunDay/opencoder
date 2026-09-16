// commandMenu.js — pure slash-command + `$skill` + `@agent` catalog for the
// composer.
// TUI parity source: the tui slash commands (act/plan/compact/model/ap/
// annotation/fork/clear_context) and the runner's TEXT control heads
// (crates/session/src/control_cmd.rs): while a drain runs, /act //plan
// travel as ordinary prompts and the runner applies them at the boundary.
// Agent entries (`@name`) mirror the TUI `/agent` picker: picking switches
// the session's primary agent (POST /api/sessions/:id/agent) or, with no
// session yet, stages it onto creation. No React, no fetch — vitest
// exercises the filter directly.
import { fuzzyScore } from './fuzzy.js';

/// The fixed `/` catalog. `kind` drives execution in chat.jsx:
///   agent      → POST /agent (busy: send the text, runner applies it)
///   agentpick  → switch the primary agent to `value` (@-entry pick)
///   agentcmd   → complete the token into `/agent ` for a manual name
///   text       → send as a normal prompt (runner control heads)
///   compact    → POST /compact
///   model      → open the model picker modal
///   ap         → open the autopilot picker (off/ap/review/清除)
///   annotation → open the annotation input modal
///   fork       → POST /fork, select the new session
export const COMMAND_CATALOG = [
  { cmd: '/act', desc: '切换到 act 模式', kind: 'agent', value: 'act' },
  { cmd: '/plan', desc: '切换到 plan 只读模式', kind: 'agent', value: 'plan' },
  {
    cmd: '/agent',
    desc: '切换 primary agent：@ 选择，或输入 /agent <name>',
    kind: 'agentcmd',
  },
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

/// GET /api/agents items (+ builtin primaries merged in by chat.jsx) →
/// `@` entries. `value` is the switch target for POST /api/sessions/:id/agent
/// (or the staged creation `agent`); `desc` carries the server-computed
/// one-line identity (prompt-pool soul.md first line). Only primary-capable
/// cards are selectable — the switch endpoint 400s on anything else, so
/// `primary !== false` keeps the picker honest about what it can switch to.
export function agentsToCommands(agents) {
  const list = Array.isArray(agents) ? agents : [];
  return list
    .filter((a) => a && typeof a.name === 'string' && a.name && a.primary !== false)
    .map((a) => ({ cmd: '@' + a.name, desc: String(a.description || ''), kind: 'agentpick', value: a.name }));
}

/// The trailing `/word` | `$word` | `@word` token of the composer text, or
/// null when the text does not end in one (whitespace-terminated or plain
/// prose). `@` rides the same trailing-token rule as the TUI file-mention
/// sigil — here it opens the agent picker side of the menu instead.
export function lastCommandToken(text) {
  const t = typeof text === 'string' ? text : '';
  const m = t.match(/(\/|\$|@)([^\s]*)$/);
  if (!m) {
    return null;
  }
  return { sigil: m[1], query: m[2] };
}

/// Cap on rendered menu rows — a long `$` tail must not flood the composer.
export const MENU_CAP = 8;

/// Entries matching the LAST `/…`/`$…`/`@…` token of `text`. Fixed commands
/// and skills keep the case-insensitive prefix on `cmd` (sigil included —
/// `$deb` matches `$debug`, never `/debug`); agent entries (`@name`) use the
/// TUI fuzzy matcher instead: subsequence on the agent name (primary) with
/// the description as fallback, so `@cd` matches `coder` and `@wr` matches
/// `writer`. An empty query lists that sigil's whole side; the list caps at
/// MENU_CAP, fuzzy agent hits sorted best-score-first (TUI parity).
export function filterCommands(catalog, text) {
  const token = lastCommandToken(text);
  if (!token) {
    return [];
  }
  const needle = (token.sigil + token.query).toLowerCase();
  const entries = (Array.isArray(catalog) ? catalog : [])
    .filter((e) => e && typeof e.cmd === 'string');
  const agentHits = [];
  const rest = [];
  entries.forEach((e) => {
    if (e.kind === 'agentpick') {
      // `@` entries answer the `@` sigil only — `/act` must never surface
      // `@writer` just because the description fuzzily matches.
      if (token.sigil !== '@') {
        return;
      }
      const q = token.query.toLowerCase();
      const score = fuzzyScore(q, e.value.toLowerCase())
        ?? fuzzyScore(q, String(e.desc || '').toLowerCase());
      if (score !== null) {
        agentHits.push([score, e]);
      }
      return;
    }
    if (e.cmd.toLowerCase().startsWith(needle)) {
      rest.push(e);
    }
  });
  agentHits.sort((a, b) => a[0] - b[0]);
  return rest.concat(agentHits.map(([, e]) => e)).slice(0, MENU_CAP);
}

/// Catalog + skills + agents in one call — chat.jsx's only filter entry point.
export function commandsForInput(text, skills, agents) {
  return filterCommands(
    COMMAND_CATALOG.concat(skillsToCommands(skills), agentsToCommands(agents)),
    text,
  );
}

/// Menu click → new composer text. Skill entries complete the token into
/// `$name `; the `/agent` command entry completes into `/agent ` (the manual
/// name then rides the prompt, runner-parsed); other entries replace it with
/// `cmd ` so arguments can follow.
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
