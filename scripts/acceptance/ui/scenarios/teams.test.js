const assert = require('node:assert/strict');
const { test } = require('node:test');
const { teamAnswer } = require('./teams');

test('Team fixture follows real captain schemas and chooses a listed member', () => {
  const plan = JSON.parse(teamAnswer({ messages: [{ role: 'user', content: '规划下一轮讨论的核心问题\n- node_id: member-a（default）' }] }));
  assert.deepEqual(plan.participants, ['member-a']);
});
test('Team fixture does not replay an old plan when the current prompt requests closing', () => {
  const closing = JSON.parse(teamAnswer({ messages: [
    { role: 'user', content: '规划下一轮讨论的核心问题\n- node_id: member-a（default）' },
    { role: 'user', content: [{ type: 'text', text: '讨论判断话题是否可以收尾' }] },
  ] }));
  assert.equal(closing.complete, true);
});
test('Team fixture returns the member answer for ordinary Agent traffic', () => {
  assert.equal(teamAnswer({ messages: [{ role: 'user', content: 'ordinary prompt' }] }), 'browser node-owned answer');
});
