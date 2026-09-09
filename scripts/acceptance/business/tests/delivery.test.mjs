import test from 'node:test';
import assert from 'node:assert/strict';
import { deliveryAdapter } from '../validation/delivery.mjs';

const forbidden = async () => { throw Error('Unexpected outbound operation'); };
test('legacy delivery is local and only follows completed analysis', async () => {
  const records = [];
  const adapter = deliveryAdapter({ analyze: forbidden, createGroup: forbidden,
    ensureMembers: forbidden, pushResult: forbidden }, row => records.push(row));
  const job = { id: 'job-local', analysis: { status: 'running', attempts: 1 } };
  await assert.rejects(adapter.operations.createGroup({}, job), /preceded/);
  assert.equal(records.length, 0);
  job.analysis.status = 'done';
  job.delivery = { group: await adapter.operations.createGroup({}, job) };
  assert.equal(await adapter.operations.ensureMembers({}, job), job.delivery.group);
  assert.equal(await adapter.operations.pushResult({}, job), 'e2e-local-message');
  assert.deepEqual(records.map(row => row.type), ['local-create', 'local-members', 'local-delivery']);
  await assert.rejects(adapter.operations.analyze(), /dispatch through opencoder/);
  await assert.rejects(adapter.regressionAnalyze(), /dispatch through opencoder/);
});

test('evaluation and regression ticket callbacks never call the real outbound operation', async () => {
  const records = [];
  const url = id => 'https://tickets.example.test/' + id;
  const adapter = deliveryAdapter({ analyze: forbidden, createTicket: forbidden }, row => records.push(row), url);
  await assert.rejects(adapter.operations.createTicket({}, { id: 'incomplete' }), /preceded/);
  const receipt = await adapter.operations.createTicket({}, { id: 'job-local', analysis: { status: 'done' } });
  assert.equal(receipt.viking_url, url(receipt.ticket_id));
  assert.deepEqual(await adapter.regressionCreateTicket({}, { key: 'regression:reg-local' }), receipt);
  assert.deepEqual(records.map(row => row.jobId), ['job-local', null]);
  await assert.rejects(adapter.operations.analyze(), /dispatch through opencoder/);
  await assert.rejects(adapter.regressionAnalyze(), /dispatch through opencoder/);
});

test('unknown delivery contracts and missing ticket URL contracts fail before starting', () => {
  assert.throws(() => deliveryAdapter({ analyze: forbidden }, () => {}), /Unknown/);
  assert.throws(() => deliveryAdapter({ createTicket: forbidden }, () => {}), /URL contract/);
});
