// vitest unit tests for the pure slash/$skill command catalog (commandMenu.js).
// No DOM, no mocks — the filter is the contract chat.jsx's composer menu
// renders from.
import { describe, expect, it } from 'vitest';
import {
  COMMAND_CATALOG, MENU_CAP, commandsForInput, filterCommands,
  lastCommandToken, replaceToken, skillsToCommands, stripLastToken,
} from './commandMenu.js';

const SKILLS = [
  { name: 'debug', description: '调试技能', enabled: true },
  { name: 'review', description: '评审技能', enabled: false },
];

describe('catalog shape (TUI parity contract)', () => {
  it('carries the nine fixed slash entries with their kinds', () => {
    expect(COMMAND_CATALOG.map((e) => e.cmd)).toEqual([
      '/act', '/plan', '/act_clear_context', '/clear_context', '/compact',
      '/model', '/ap', '/annotation', '/fork',
    ]);
    const byCmd = Object.fromEntries(COMMAND_CATALOG.map((e) => [e.cmd, e]));
    expect(byCmd['/act'].kind).toBe('agent');
    expect(byCmd['/act'].value).toBe('act');
    expect(byCmd['/plan'].value).toBe('plan');
    expect(byCmd['/act_clear_context'].kind).toBe('text');
    expect(byCmd['/clear_context'].kind).toBe('text');
    expect(byCmd['/compact'].kind).toBe('compact');
    expect(byCmd['/model'].kind).toBe('model');
    expect(byCmd['/ap'].kind).toBe('ap');
    expect(byCmd['/annotation'].kind).toBe('annotation');
    expect(byCmd['/fork'].kind).toBe('fork');
  });

  it('converts GET /api/skills items into $ entries', () => {
    const entries = skillsToCommands(SKILLS);
    expect(entries).toEqual([
      { cmd: '$debug', desc: '调试技能', kind: 'skill', value: 'debug' },
      { cmd: '$review', desc: '评审技能', kind: 'skill', value: 'review' },
    ]);
    expect(skillsToCommands(undefined)).toEqual([]);
    expect(skillsToCommands([{ name: '' }, null, 'x'])).toEqual([]);
  });
});

describe('lastCommandToken', () => {
  it('extracts only the trailing / or $ token', () => {
    expect(lastCommandToken('帮我 /mod')).toEqual({ sigil: '/', query: 'mod' });
    expect(lastCommandToken('/plan $de')).toEqual({ sigil: '$', query: 'de' });
    expect(lastCommandToken('/')).toEqual({ sigil: '/', query: '' });
    expect(lastCommandToken('plain text')).toBeNull();
    expect(lastCommandToken('ended with a slash / ')).toBeNull();
    expect(lastCommandToken('')).toBeNull();
    expect(lastCommandToken(undefined)).toBeNull();
  });
});

describe('filterCommands', () => {
  it('prefix-matches case-insensitively', () => {
    expect(filterCommands(COMMAND_CATALOG, '/mo').map((e) => e.cmd)).toEqual(['/model']);
    expect(filterCommands(COMMAND_CATALOG, '/MO').map((e) => e.cmd)).toEqual(['/model']);
    expect(filterCommands(COMMAND_CATALOG, '/COMPACT').map((e) => e.cmd)).toEqual(['/compact']);
  });

  it('matches against the LAST token only', () => {
    expect(filterCommands(COMMAND_CATALOG, '/plan /mo').map((e) => e.cmd)).toEqual(['/model']);
    expect(filterCommands(COMMAND_CATALOG, '/mo /pl').map((e) => e.cmd)).toEqual(['/plan']);
    // A token followed by prose is no longer a command query.
    expect(filterCommands(COMMAND_CATALOG, '/mo 然后')).toEqual([]);
  });

  it('keeps $ and / queries strictly separated', () => {
    const catalog = COMMAND_CATALOG.concat(skillsToCommands(SKILLS));
    expect(filterCommands(catalog, '$deb').map((e) => e.cmd)).toEqual(['$debug']);
    expect(filterCommands(catalog, '$DE').map((e) => e.cmd)).toEqual(['$debug']);
    expect(filterCommands(catalog, '$deb').some((e) => e.cmd.startsWith('/'))).toBe(false);
    expect(filterCommands(catalog, '/de').some((e) => e.cmd.startsWith('$'))).toBe(false);
  });

  it('returns the full side on an empty query, capped at MENU_CAP', () => {
    const all = filterCommands(COMMAND_CATALOG, '/');
    expect(all).toHaveLength(MENU_CAP); // 9 fixed commands → capped at 8
    expect(all.length).toBeLessThanOrEqual(MENU_CAP);
    const skills = filterCommands(skillsToCommands(SKILLS), '$');
    expect(skills.map((e) => e.cmd)).toEqual(['$debug', '$review']);
    expect(filterCommands([], '/')).toEqual([]);
    expect(filterCommands(undefined, '/mo')).toEqual([]);
  });
});

describe('commandsForInput', () => {
  it('filters the combined catalog + skills by the composer text', () => {
    expect(commandsForInput('/act', SKILLS).map((e) => e.cmd))
      .toEqual(['/act', '/act_clear_context']); // /act is a prefix of both
    expect(commandsForInput('$rev', SKILLS).map((e) => e.cmd)).toEqual(['$review']);
    expect(commandsForInput('no token here', SKILLS)).toEqual([]);
  });
});

describe('replaceToken / stripLastToken', () => {
  it('completes skill tokens in place with a trailing space', () => {
    expect(replaceToken('帮我 $deb', { cmd: '$debug', kind: 'skill', value: 'debug' })).toBe('帮我 $debug ');
    expect(replaceToken('$deb', { cmd: '$debug', kind: 'skill', value: 'debug' })).toBe('$debug ');
  });

  it('replaces command tokens with cmd + space', () => {
    expect(replaceToken('/mo', { cmd: '/model', kind: 'model' })).toBe('/model ');
    expect(replaceToken('run /pl', { cmd: '/plan', kind: 'agent', value: 'plan' })).toBe('run /plan ');
  });

  it('strips the token and tolerates missing tokens', () => {
    expect(stripLastToken('run /pl')).toBe('run ');
    expect(stripLastToken('/mo')).toBe('');
    expect(stripLastToken('plain')).toBe('plain');
    expect(stripLastToken(undefined)).toBe('');
  });
});
