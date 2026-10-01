// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import '../../test/setup-dom.js';
import { DagRunContext } from './context.jsx';

describe('saved DAG execution context', () => {
  it('displays pinned resources and dynamic instance paths, not live pool state', () => {
    render(<DagRunContext context={{ state: 'ready', container_id: 'dag-run-one', workspace: '/workspace', steps: [
      { name: 'build', cwd: '/workspace/build', resource: { type: 'binary', resource: 'tool', version: 3, sha256: 'a'.repeat(64) } },
      { name: 'batch', cwd: '/workspace/batch', dynamic: true, resource: { type: 'agent', name: 'act', sha256: 'b'.repeat(64) } },
    ] }} />);
    expect(screen.getByText('dag-run-one')).toBeTruthy();
    expect(screen.getByText('tool · v3')).toBeTruthy();
    expect(screen.getByText('/workspace/batch/instances/<index>')).toBeTruthy();
    expect(screen.getByText('a'.repeat(64))).toBeTruthy();
  });
  it('does not invent pinned versions during preparation', () => {
    render(<DagRunContext context={{ state: 'preparing', container_id: 'dag-run-one', workspace: '/workspace', steps: [{ name: 'build', cwd: '/workspace/build', resource: null }] }} />);
    expect(screen.getByText('资源准备中，尚未固定版本')).toBeTruthy();
    expect(screen.getByText('准备中')).toBeTruthy();
  });
});
