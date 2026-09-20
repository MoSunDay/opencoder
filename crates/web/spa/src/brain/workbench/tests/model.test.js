import { expect, it } from 'vitest';
import { graph, launchBody, statusOf } from '../model.js';
import { repairPlan } from '../editor/model.js';
it('projects inputs, instances, multiple outputs and semantic loop routes with stable identities', () => {
 const plan = repairPlan([{ id: 'act', kind: 'agent', target: 'act', summary: '执行' }]);
 const a = graph(plan, [{ id: 'fix', counts: { running: 1 } }]); const b = graph(plan, [{ id: 'fix', counts: { succeeded: 1 } }]);
 expect(a.nodes.map((n) => [n.id, n.position])).toEqual(b.nodes.map((n) => [n.id, n.position]));
 for (const id of ['fix', 'verify', 'input:document', 'output:verification', 'route:after-verify']) expect(a.nodes.some((n) => n.id === id)).toBe(true);
 expect(a.edges).toContainEqual(expect.objectContaining({ source: 'route:after-verify', target: 'input:feedback' }));
 expect(statusOf({ counts: { failed: 1, succeeded: 9 } })).toBe('failed');
});
it('submits the engineering description as one-level KV inputs and uses only the v3 scheduler request', () => {
 const values = { mode: 'dynamic', objective: ' Goal ', node: 'n', references: ['ref@2'], engineering: [{ key: ' repo ', value: 'x/y' }, { key: 'parallel', value: '3' }, { key: 'verified', value: 'true' }, { key: 'document', value: '{"name":"需求","markdown":"# 正文"}' }, { key: 'raw', value: 'not json' }, { key: '   ', value: 'ignored' }, { key: 'empty', value: '' }] };
 expect(launchBody(values, 'b')).toMatchObject({ schema_version: 3, objective: 'Goal', capability_ids: [], max_rounds: 32 });
 for (const field of ['mode', 'plan', 'references']) expect(launchBody(values, 'b')).not.toHaveProperty(field);
 expect(launchBody(values, 'b').inputs).toEqual({ repo: 'x/y', parallel: 3, verified: true, document: { name: '需求', markdown: '# 正文' }, raw: 'not json', empty: '' });
 expect(launchBody({ ...values, capability_ids: ['dag-test'], max_rounds: 4 }, 'b')).toMatchObject({ capability_ids: ['dag-test'], max_rounds: 4 });
 expect(launchBody({ ...values, engineering: [] }, 'b').inputs).toEqual({ request: 'Goal' });
 expect(() => launchBody({ ...values, engineering: [{ key: 'repo', value: '1' }, { key: ' repo ', value: '2' }] }, 'b')).toThrow('工程参数名重复');
});
