import { expect, it } from 'vitest';
import { capabilityOptions } from './selection.js';
const generated = { id: 'generated', kind: 'agent', target: 'worker' };
const stored = { ...generated, id: 'stored' };
it('uses server selections and keeps only the currently selected legacy ID', () => {
  expect(capabilityOptions([generated, stored], ['stored']).map((c) => c.value)).toEqual(['stored']);
  expect(capabilityOptions([generated, stored], ['stored'], 'generated').map((c) => c.value)).toEqual(['generated', 'stored']);
  expect(capabilityOptions([generated, stored], []).length).toBe(0);
  expect(capabilityOptions([generated, stored], undefined).length).toBe(2);
});
it('labels different input and output contracts for the same target', () => {
  const options = capabilityOptions([stored, { ...generated, required_inputs: ['source'], required_outputs: ['report'] }]);
  expect(options[0].label).toContain('必填输入：无');
  expect(options[1].label).toContain('必填输入：source');
  expect(options[1].label).toContain('必填输出：report');
});
