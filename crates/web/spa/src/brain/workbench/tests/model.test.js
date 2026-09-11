import { describe, expect, it } from 'vitest';
import { dependencies, graph, launchBody, newPlan, newStep, statusOf } from '../model.js';
describe('ontology projection', () => {
  it('derives edges from control, data, condition and batch dependencies', () => {
    const step = { ...newStep('join'), depends_on: ['a'], inputs: { input: { binding: { source: 'output', step: 'b' } } }, when: { value: { source: 'output', step: 'a' } }, foreach: { items: { source: 'output', step: 'c' } } };
    expect(dependencies(step)).toEqual(['a', 'b', 'c']);
  });
  it('keeps stable graph identities while statuses change and shows ontology ports', () => {
    const plan = newPlan(); plan.inputs = { name: { schema: { type: 'string' } } }; plan.steps[0].inputs.name = { schema: { type: 'string' }, binding: { source: 'input', name: 'name' } };
    const a = graph(plan, [{ id: 'step-1', counts: { running: 1 } }]);
    const b = graph(plan, [{ id: 'step-1', counts: { succeeded: 1 } }]);
    expect(a.nodes.map((n) => [n.id, n.position])).toEqual(b.nodes.map((n) => [n.id, n.position]));
    const ontology = graph(plan, [], true); expect(ontology.nodes.map((n) => n.id)).toEqual(['step-1', 'input:name', 'result:result']);
    expect(statusOf({ counts: { failed: 1, succeeded: 9 } })).toBe('failed');
  });
  it('dynamic planning uses explicit references and fixed mode pins an exact version', () => {
    const values = { mode: 'dynamic', objective: ' Goal ', node: 'node-a', plan: 'fixed@3', references: ['ref@2'], inputs: '{"x":4}' };
    expect(launchBody(values, 'brain-a')).toMatchObject({ mode: 'dynamic', plan: null, references: [{ id: 'ref', version: 2 }], inputs: { x: 4 } });
    expect(launchBody({ ...values, mode: 'fixed' }, 'brain-a')).toMatchObject({ plan: { id: 'fixed', version: 3 }, references: [] });
    expect(() => launchBody({ ...values, inputs: 'invalid' }, 'brain-a')).toThrow();
  });
});
