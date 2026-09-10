// Plain node unit tests: no jsdom, no React — only the pure cascade helpers.
// fetchTargetOptions gets a fake apiGet keyed by path so each test also pins
// the endpoint used per kind.
import { describe, expect, it, vi } from 'vitest';
import { TARGET_ENDPOINTS, fetchTargetOptions, targetOptionsFrom } from './targetOptions.js';

const fixtures = {
  '/api/agents': { agents: [{ name: 'act' }, { name: 'custom-agent' }, null, { other: 1 }, { name: '  ' }] },
  '/api/teams': { teams: [{ name: 'crew-a' }, { name: 'crew-b' }, {}] },
  '/api/dag/defs': [{ id: 'dag-build' }, null, { id: '' }, { name: 'not-an-id' }],
  '/api/todo/templates': {
    templates: [
      { name: 'release', versions: [{ version: 'v1', note: 'init' }, { version: 'v2', note: 'next' }] },
      { name: 'audit', versions: [{ version: 'v1', note: '' }] },
      null,
      { versions: [{ version: 'v9' }] },
      { name: 'empty' },
    ],
  },
};
const fetchAll = vi.fn(async (path) => fixtures[path]);

describe('targetOptionsFrom', () => {
  it('extracts agent names', () => {
    expect(targetOptionsFrom('agent', fixtures['/api/agents']))
      .toEqual([{ value: 'act', label: 'act' }, { value: 'custom-agent', label: 'custom-agent' }]);
  });

  it('extracts team names', () => {
    expect(targetOptionsFrom('team', fixtures['/api/teams']))
      .toEqual([{ value: 'crew-a', label: 'crew-a' }, { value: 'crew-b', label: 'crew-b' }]);
  });

  it('reads dag defs from a bare array and from a {defs} wrapper', () => {
    const expected = [{ value: 'dag-build', label: 'dag-build' }];
    expect(targetOptionsFrom('dag', fixtures['/api/dag/defs'])).toEqual(expected);
    expect(targetOptionsFrom('dag', { defs: fixtures['/api/dag/defs'] })).toEqual(expected);
    expect(targetOptionsFrom('dag', { defs: 'nope' })).toEqual([]);
  });

  it('joins todo template name with every version', () => {
    expect(targetOptionsFrom('todos', fixtures['/api/todo/templates'])).toEqual([
      { value: 'release/v1', label: 'release/v1' },
      { value: 'release/v2', label: 'release/v2' },
      { value: 'audit/v1', label: 'audit/v1' },
    ]);
  });

  it('filters missing and odd entries instead of throwing', () => {
    expect(targetOptionsFrom('agent', null)).toEqual([]);
    expect(targetOptionsFrom('agent', { agents: 'oops' })).toEqual([]);
    expect(targetOptionsFrom('todos', { templates: [{ versions: [] }] })).toEqual([]);
    expect(targetOptionsFrom('dag', undefined)).toEqual([]);
  });

  it('returns [] for an unknown kind', () => {
    expect(targetOptionsFrom('project', fixtures['/api/agents'])).toEqual([]);
    expect(targetOptionsFrom(undefined, fixtures['/api/agents'])).toEqual([]);
  });
});

describe('fetchTargetOptions', () => {
  it('hits the endpoint registered for each kind and normalizes the payload', async () => {
    for (const kind of ['agent', 'team', 'dag', 'todos']) {
      const options = await fetchTargetOptions(fetchAll, kind);
      expect(options.length).toBeGreaterThan(0);
      expect(fetchAll).toHaveBeenLastCalledWith(TARGET_ENDPOINTS[kind]);
    }
    await expect(fetchTargetOptions(fetchAll, 'agent')).resolves.toEqual([
      { value: 'act', label: 'act' }, { value: 'custom-agent', label: 'custom-agent' },
    ]);
  });

  it('resolves [] for an unknown kind without calling apiGet', async () => {
    const apiGet = vi.fn();
    expect(await fetchTargetOptions(apiGet, 'operator')).toEqual([]);
    expect(apiGet).not.toHaveBeenCalled();
  });
});
