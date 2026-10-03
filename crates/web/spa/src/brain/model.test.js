import { describe, expect, it } from 'vitest';
import { capabilityForm, capabilityTarget, needsTargetSave } from './model.js';

describe('capability execution contracts', () => {
  it('saves a new default target and detects contract edits without changing the executor', () => {
    const target = capabilityTarget({ target_kind: 'agent', target: ' act ' });
    expect(needsTargetSave(null, target)).toBe(true);
    expect(needsTargetSave(target, target)).toBe(false);
    const edited = { ...target, required_inputs: ['revision'], required_outputs: ['passed'] };
    expect(needsTargetSave(target, edited)).toBe(true);
    expect(needsTargetSave(edited, target)).toBe(true);
    expect(needsTargetSave(edited, { ...edited })).toBe(false);
  });

  it('loads and saves the input and output requirements with the target', () => {
    const target = { kind: 'operator', target: 'act', required_inputs: ['revision'], required_outputs: ['passed', 'failures'] };
    const form = capabilityForm({}, target);
    expect(capabilityTarget(form)).toEqual(target);
    expect(capabilityTarget({ ...form, required_inputs: [' revision ', ''], required_outputs: [] }))
      .toEqual({ kind: 'operator', target: 'act', required_inputs: ['revision'] });
  });
});
