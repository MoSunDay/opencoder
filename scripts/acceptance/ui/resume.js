const assert = require('node:assert/strict');

function initializeReport(previous, coverage) {
  if (!previous) return { passed: false, coverage, artifacts: {}, checks: [] };
  assert.deepEqual(previous.coverage, coverage, 'acceptance scope changed; use a new evidence directory');
  return { ...structuredClone(previous), passed: false, error: undefined };
}

function verifyArtifact(previous, current, name) {
  assert.equal(typeof previous, 'string', `previous artifact is missing: ${name}`);
  assert.equal(previous, current, `artifact changed: ${name}; use a new evidence directory`);
}

function recordCheck(report, result) {
  const index = report.checks.findIndex((item) => item.name === result.name);
  if (index < 0) report.checks.push(result);
  else {
    const old = report.checks[index];
    const { attempts, ...attempt } = old;
    report.checks[index] = { ...result, attempts: [...(attempts || []), attempt] };
  }
}

module.exports = { initializeReport, verifyArtifact, recordCheck };
