import { expect, it } from 'vitest';
import { capabilityFor, currentRound, normalizeEvent, roundsOf } from './v3Model.js';

const view = {
  run: { round: 2 },
  capabilities: [{ capability_id: 'dag-check', kind: 'dag', target: 'check', version: '1' }],
  rounds: [
    { round: 1, operations: [{ execution_id: 'agent-1', capability_id: 'agent-act' }] },
    { round: 2, operations: [{ execution_id: 'dag-1', capability_id: 'dag-check' }] },
  ],
};

it('groups rounds and resolves capability metadata without execution bodies', () => {
  expect(currentRound(view)).toBe(2);
  expect(roundsOf(view).map((round) => round.round)).toEqual([1, 2]);
  expect(capabilityFor(view, view.rounds[1].operations[0])).toMatchObject({ kind: 'dag', target: 'check' });
  expect(capabilityFor(view, { capability_id: 'missing', execution_kind: 'agent' })).toMatchObject({ kind: 'agent' });
});

it('normalizes scheduler event rows at the UI boundary', () => {
  expect(normalizeEvent({ seq: 7, event_type: 'operation_terminal', execution_id: 'dag-1', at_ms: 9 })).toEqual({
    seq: 7, event: 'operation_terminal', data: expect.objectContaining({ execution_id: 'dag-1' }), ts: 9,
  });
});
