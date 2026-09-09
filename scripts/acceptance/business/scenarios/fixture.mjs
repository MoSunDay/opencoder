// Controlled inputs; the business API, Runner, model and test commands stay real.
import { createServer } from 'node:http';
import { execFileSync } from 'node:child_process';
import { writeFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';
const [release] = process.argv.slice(2);
const { e2eFixture } = await import(pathToFileURL(`${release}/dist/test/regression/e2e/fixture.js`));
const fixture = await e2eFixture();
// The production Runner snapshots the workspace revision as well as each repo.
// Keep its root commit minimal; test repositories retain their own real history.
await writeFile(`${fixture.root}/README.md`, 'Controlled business acceptance workspace.\n');
for (const args of [['init', '-q'], ['add', 'README.md'],
  ['-c', 'user.name=Acceptance', '-c', 'user.email=acceptance@example.test', 'commit', '-qm', 'acceptance workspace']]) {
  execFileSync('git', args, { cwd: fixture.root, stdio: 'pipe' });
}
const input = [{ case_id: 'positive', case_status: 'success', execution_status: 'completed',
  turns: [{ turn_id: 'positive-turn', sequence: 1, turn_completion: { status: 'completed' },
    user_input: { prompt: '检查已完成的空画布', attachments: [] }, tool_calls: [],
    turn_result: { status: 'completed', assistant_reply: '检查完成，画布为空。' } }],
  draft: { version: '1.1.0', document: { id: 'positive-canvas', nodes: [], edges: [], document: { task_list: [] } } } }];
const source = createServer((_, response) => {
  response.writeHead(200, { 'content-type': 'application/json' }); response.end(JSON.stringify(input));
});
await new Promise(resolve => source.listen(0, '127.0.0.1', resolve));
const origin = `http://127.0.0.1:${source.address().port}`;
const revisions = Object.fromEntries(Object.entries(fixture.config.repositories).map(([name, directory]) =>
  [name, execFileSync('git', ['rev-parse', 'HEAD'], { cwd: directory, encoding: 'utf8' }).trim()]));
console.log(JSON.stringify({ root: fixture.root, config: { ...fixture.config, allowedOrigins: [origin] },
  requests: { 'eval-diagnose': { eventId: 'positive', caseId: 'positive', revisions, input: { kind: 'results', url: origin } },
    'regression-test': fixture.fixedRequest }, input }));
// Termination closes the source socket; fixture files and databases are retained.
process.once('SIGTERM', () => process.exit(0));
process.once('SIGINT', () => process.exit(0));
