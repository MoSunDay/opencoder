const assert = require('node:assert/strict');
const { test } = require('node:test');
const { CASES, verifyCoverage } = require('./scope');
const pages = ['project', 'brain', 'topics', 'schedules', 'dag', 'todos', 'team', 'chat', 'agents', 'nodes'];

test('every registered page has a functional case, separate from responsive snapshots', () => {
  assert.equal(Object.keys(verifyCoverage(pages)).length, 10);
  assert(CASES.some((item) => item.terminal));
});
test('new pages fail the gate until they have functional acceptance', () => assert.throws(() => verifyCoverage([...pages, 'new-page']), /missing=new-page/));
test('removed pages cannot silently stay in the acceptance matrix', () => assert.throws(() => verifyCoverage(pages.filter((page) => page !== 'chat')), /unknown=chat/));
test('duplicate case names cannot overwrite evidence', () => assert.throws(() => verifyCoverage(pages, [...CASES, CASES[0]]), /duplicate/));
