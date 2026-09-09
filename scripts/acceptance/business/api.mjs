// The real API and bridge, with only outbound delivery replaced by a local sink.
import { appendFile, writeFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';
import { deliveryAdapter } from './validation/delivery.mjs';
const [root, release] = process.argv.slice(2);
const { readConfig } = await import(pathToFileURL(`${release}/dist/src/service/config.js`));
const { startApi } = await import(pathToFileURL(`${release}/dist/src/service/server.js`));
const { defaultOperations } = await import(pathToFileURL(`${release}/dist/src/service/worker.js`));
const config = await readConfig(`${root}/runtime/business.json`);
const tickets = typeof defaultOperations.createTicket === 'function';
const model = tickets ? await import(pathToFileURL(`${release}/dist/src/service/viking/model.js`)) : {};
const adapter = deliveryAdapter(defaultOperations,
  row => appendFile(`${root}/evidence/delivery.jsonl`, JSON.stringify(row) + '\n', { mode: 0o600 }), model.vikingUrl);
const service = await startApi(config, adapter.operations, adapter.regressionAnalyze, adapter.regressionCreateTicket);
await writeFile(`${root}/runtime/api-ready.json`, JSON.stringify({ port: service.port }));
let closing = false;
const close = async () => { if (closing) return; closing = true; await service.close(); };
process.once('SIGTERM', () => close().catch(e => { console.error(e); process.exitCode = 1; }));
process.once('SIGINT', () => close().catch(e => { console.error(e); process.exitCode = 1; }));
