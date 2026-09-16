// vitest unit tests for the pure slash/$skill command catalog (commandMenu.js).
// No DOM, no mocks — the filter is the contract chat.jsx's composer menu
// renders from.
import { describe, expect, it } from 'vitest';
import {
  COMMAND_CATALOG, MENU_CAP, agentsToCommands, commandsForInput, filterCommands,
  lastCommandToken, replaceToken, skillsToCommands, stripLastToken,
} from './commandMenu.js';

const SKILLS = [
  { name: 'debug', description: '调试技能', enabled: true },
  { name: 'review', description: '评审技能', enabled: false },
];

const AGENTS = [
  { name: 'writer', description: 'Writer soul: small diffs.', primary: true },
  { name: 'coder', description: 'Custom agent coder', primary: true },
];

describe('agentsToCommands', () => {
  it('maps GET /api/agents cards to @ entries carrying name + description', () => {
    expect(agentsToCommands(AGENTS)).toEqual([
      { cmd: '@writer', desc: 'Writer soul: small diffs.', kind: 'agentpick', value: 'writer' },
      { cmd: '@coder', desc: 'Custom agent coder', kind: 'agentpick', value: 'coder' },
    ]);
    expect(agentsToCommands(undefined)).toEqual([]);
    expect(agentsToCommands([{ name: '' }, null, 'x'])).toEqual([]);
  });

  it('drops non-primary cards the switch endpoint would reject', () => {
    expect(agentsToCommands([
      { name: 'writer', primary: true },
      { name: 'ghost', primary: false },
      { name: 'legacy' },
    ]).map((e) => e.cmd)).toEqual(['@writer', '@legacy']);
  });
});

describe('catalog shape (TUI parity contract)', () => {
  it('carries the ten fixed slash entries with their kinds', () => {
    expect(COMMAND_CATALOG.map((e) => e.cmd)).toEqual([
      '/act', '/plan', '/agent', '/act_clear_context', '/clear_context', '/compact',
      '/model', '/ap', '/annotation', '/fork',
    ]);
    const byCmd = Object.fromEntries(COMMAND_CATALOG.map((e) => [e.cmd, e]));
    expect(byCmd['/act'].kind).toBe('agent');
    expect(byCmd['/act'].value).toBe('act');
    expect(byCmd['/plan'].value).toBe('plan');
    expect(byCmd['/agent'].kind).toBe('agentcmd');
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

  it('extracts the trailing @ token for the agent picker side', () => {
    expect(lastCommandToken('@wr')).toEqual({ sigil: '@', query: 'wr' });
    expect(lastCommandToken('帮我 @writer')).toEqual({ sigil: '@', query: 'writer' });
    expect(lastCommandToken('@')).toEqual({ sigil: '@', query: '' });
    // A mid-text @ (email) only yields a token when it is the trailing
    // token — a@b.com ends in one, but nothing matches it (no agent named
    // b.com), so the menu stays empty.
    expect(lastCommandToken('mail a@b.com')).toEqual({ sigil: '@', query: 'b.com' });
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
    expect(all).toHaveLength(MENU_CAP); // 10 fixed commands → capped at 8
    expect(all.length).toBeLessThanOrEqual(MENU_CAP);
    const skills = filterCommands(skillsToCommands(SKILLS), '$');
    expect(skills.map((e) => e.cmd)).toEqual(['$debug', '$review']);
    expect(filterCommands([], '/')).toEqual([]);
    expect(filterCommands(undefined, '/mo')).toEqual([]);
  });

  it('lists @ agent entries on an empty query and fuzzies the name (desc fallback)', () => {
    const catalog = agentsToCommands(AGENTS);
    // `@` alone lists the whole agent side in catalog order.
    expect(filterCommands(catalog, '@').map((e) => e.cmd)).toEqual(['@writer', '@coder']);
    // Fuzzy subsequence on the agent name: "cd" is NOT a prefix of "coder"
    // but IS a subsequence — the TUI fuzzy matcher must hit it.
    expect(filterCommands(catalog, '@cd').map((e) => e.cmd)).toEqual(['@coder']);
    expect(filterCommands(catalog, '@wr').map((e) => e.cmd)).toEqual(['@writer']);
    expect(filterCommands(catalog, '@CD').map((e) => e.cmd)).toEqual(['@coder']);
    // No name hit falls back to the description subsequence.
    const descOnly = agentsToCommands([{ name: 'ops', description: 'the coder one', primary: true }]);
    expect(filterCommands(descOnly, '@der').map((e) => e.cmd)).toEqual(['@ops']);
    // Misses produce an empty menu; fixed commands never fuzzy-match.
    expect(filterCommands(catalog, '@zzz')).toEqual([]);
    expect(filterCommands(catalog, '/wr')).toEqual([]);
  });

  it('sorts fuzzy agent hits best-score-first and caps at MENU_CAP', () => {
    const many = agentsToCommands([
      { name: 'coder', primary: true },      // prefix match → best score
      { name: 'coderTwo', primary: true },
      { name: 'coderThree', primary: true },
      { name: 'coderFour', primary: true },
      { name: 'coderFive', primary: true },
      { name: 'coderSix', primary: true },
      { name: 'coderSeven', primary: true },
      { name: 'co_d', primary: true },       // scattered → worse score
      { name: 'c_o_d_r', primary: true },    // worst score → capped out
      { name: 'sandy', primary: true },      // not a subsequence → dropped
    ]);
    const hits = filterCommands(many, '@cod');
    expect(hits.length).toBe(MENU_CAP);
    // 'coder' (compact prefix) must rank ahead of scattered 'co_d'/'c_o_d_r'.
    expect(hits[0].cmd).toBe('@coder');
    expect(hits.slice(0, 7).every((e) => e.cmd.startsWith('@coder'))).toBe(true);
    expect(hits[7].cmd).toBe('@co_d');
    expect(hits.some((e) => e.cmd === '@sandy')).toBe(false);
    expect(hits.some((e) => e.cmd === '@c_o_d_r')).toBe(false);
  });

  it('keeps @ queries away from / and $ sides', () => {
    const catalog = COMMAND_CATALOG.concat(skillsToCommands(SKILLS), agentsToCommands(AGENTS));
    expect(filterCommands(catalog, '@wr').every((e) => e.kind === 'agentpick')).toBe(true);
    expect(filterCommands(catalog, '/agent').every((e) => e.cmd.startsWith('/'))).toBe(true);
  });
});

describe('commandsForInput', () => {
  it('filters the combined catalog + skills by the composer text', () => {
    expect(commandsForInput('/act', SKILLS).map((e) => e.cmd))
      .toEqual(['/act', '/act_clear_context']); // /act is a prefix of both
    expect(commandsForInput('$rev', SKILLS).map((e) => e.cmd)).toEqual(['$review']);
    expect(commandsForInput('no token here', SKILLS)).toEqual([]);
  });

  it('filters the combined catalog + skills + agents by the composer text', () => {
    // Third arg is the raw GET /api/agents cards (as chat.jsx passes them).
    expect(commandsForInput('@wr', SKILLS, AGENTS).map((e) => e.cmd))
      .toEqual(['@writer']);
    // No agents argument → legacy two-arg call keeps working (no @ entries).
    expect(commandsForInput('@wr', SKILLS)).toEqual([]);
    expect(commandsForInput('/agent', SKILLS, AGENTS).map((e) => e.cmd))
      .toEqual(['/agent']);
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

  it('completes the /agent command token in place for a manual name', () => {
    expect(replaceToken('/agent', { cmd: '/agent', kind: 'agentcmd' })).toBe('/agent ');
    expect(replaceToken('run /agen', { cmd: '/agent', kind: 'agentcmd' })).toBe('run /agent ');
  });

  it('strips the token and tolerates missing tokens', () => {
    expect(stripLastToken('run /pl')).toBe('run ');
    expect(stripLastToken('/mo')).toBe('');
    expect(stripLastToken('plain')).toBe('plain');
    expect(stripLastToken(undefined)).toBe('');
  });
});
