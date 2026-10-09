const assert = require('node:assert/strict');
const { test } = require('node:test');
const { assertResult, assertTaskOutput } = require('./scenario');

test('task conclusions require structured computed values, including Markdown and nested output', () => {
  const marker = 'pcap-example-agent';
  const output = JSON.stringify({ marker, sum: 10, passed: true });
  for (const text of [output, '### verify\n' + output, JSON.stringify({ output_text: output })]) {
    assertResult(text, marker);
  }
  for (const text of [
    marker + ': expected sum 10 but execution produced no result',
    JSON.stringify({ marker, sum: 9, passed: true, expected: 10 }),
    JSON.stringify({ marker, sum: 10, passed: false }),
  ]) assert.throws(() => assertResult(text, marker));
});

test('Brain child outputs accept native text or objects while checking the business result', () => {
  const marker = 'pcap-example-brain';
  const output = { marker, sum: 10, passed: true };
  assertTaskOutput(output, marker);
  assertTaskOutput('```json\n' + JSON.stringify(output) + '\n```\nCalculation finished.', marker);
  assert.throws(() => assertTaskOutput({ ...output, passed: false }, marker));
});

test('nested evidence and quoted braces preserve the actual verdict', () => {
  const marker = 'pcap-example-brain';
  const output = { marker, sum: 10, passed: true,
    verification: 'Compare {a} with "expected"',
    evidence: { expected: { marker, sum: 10, passed: true }, inputs: [2, 3, 5] } };
  for (const value of [output, '```json\n' + JSON.stringify(output, null, 2) + '\n```',
    JSON.stringify({ output_text: JSON.stringify(output) })]) assertTaskOutput(value, marker);
  assert.throws(() => assertTaskOutput({ ...output, sum: 9 }, marker));
  assert.throws(() => assertTaskOutput({ ...output, passed: false }, marker));
  assert.throws(() => assertTaskOutput({ ...output, marker: 'other-input' }, marker));
  assert.throws(() => assertTaskOutput({ evidence: output.evidence }, marker));
});
