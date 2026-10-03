const assert = require('node:assert/strict');
const { test } = require('node:test');
const { initializeReport, verifyArtifact, recordCheck } = require('./resume');

test('resumption requires the same scope and the same binary bytes', () => {
  assert.throws(() => initializeReport({ coverage: { graph: ['ontology'] } }, { graph: ['different-case'] }), /scope changed/);
  assert.throws(() => verifyArtifact(undefined, 'current', 'server'), /missing/);
  assert.throws(() => verifyArtifact('old', 'new', 'server'), /artifact changed/);
  verifyArtifact('same', 'same', 'server');
});

test('previous failed checks stay failed until rerun and their evidence is retained', () => {
  const coverage = { graph: ['ontology'] };
  const previous = { passed: false, coverage, artifacts: {}, checks: [{ name: 'ontology', passed: false, log: '/first.log' }] };
  const report = initializeReport(previous, coverage);
  assert.equal(report.checks[0].passed, false);
  recordCheck(report, { name: 'ontology', passed: true, log: '/second.log' });
  assert.equal(report.checks.length, 1);
  assert.equal(report.checks[0].passed, true);
  assert.equal(report.checks[0].attempts[0].log, '/first.log');
  assert.equal(previous.checks[0].passed, false);
});
