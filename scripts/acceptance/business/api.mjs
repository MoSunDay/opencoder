// The real API and bridge, with only outbound delivery replaced by a local sink.
import { appendFile, writeFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';
const [root, release] = process.argv.slice(2);
const { readConfig } = await import(pathToFileURL(`${release}/dist/src/service/config.js`));
const { startApi } = await import(pathToFileURL(`${release}/dist/src/service/server.js`));
const { defaultOperations } = await import(pathToFileURL(`${release}/dist/src/service/worker.js`));
const config = await readConfig(`${root}/runtime/business.json`);
const record = async (type, job) => {
  if (job.analysis.status !== 'done') throw Error('Delivery preceded completed analysis');
  await appendFile(`${root}/evidence/delivery.jsonl`, JSON.stringify({ type, jobId: job.id,
    analysis: job.analysis.status, attempt: job.analysis.attempts }) + '\n', { mode: 0o600 });
};
const operations = { ...defaultOperations,
  analyze: async () => { throw Error('API must dispatch through opencoder'); },
  createGroup: async (_, job) => { await record('local-create', job); return {
    ticketId: 'e2e-local-ticket', chatId: 'e2e-local-chat', robotId: 'e2e-local-robot', memberCount: 2 }; },
  ensureMembers: async (_, job) => { await record('local-members', job); return job.delivery.group; },
  pushResult: async (_, job) => { await record('local-delivery', job); return 'e2e-local-message'; },
};
const service = await startApi(config, operations);
await writeFile(`${root}/runtime/api-ready.json`, JSON.stringify({ port: service.port }));
let closing = false;
const close = async () => { if (closing) return; closing = true; await service.close(); };
process.once('SIGTERM', () => close().catch(e => { console.error(e); process.exitCode = 1; }));
process.once('SIGINT', () => close().catch(e => { console.error(e); process.exitCode = 1; }));
