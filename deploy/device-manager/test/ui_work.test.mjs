import test from 'node:test';
import assert from 'node:assert/strict';
import {fixture,ADMIN} from './support/fixture.mjs';
import {startUiWork} from '../src/ui/work.mjs';
import {reclaim,digest} from '../src/state.mjs';

test('UI reservation binds cases and rejects native work and cross-case registration',async t=>{
 const f=await fixture(t,2),cases=['BITS-001-a','BITS-002-b'],
  specs=cases.map((case_id,i)=>({case_id,spec_sha256:(i?'b':'a').repeat(64),input_version:'v1'}));
 const scope=await f.api.handle('POST','/v1/step-sessions',{
  dag_id:'dag-ui',step_id:'allocate',session_id:'ui-allocator',role:'allocate',
  target_step:'execute',count:2,work_type:'ui',case_ids:cases,case_specs:specs},ADMIN);
 const r=await f.api.handle('POST','/v1/reservations',{
  dag_id:'dag-ui',target_step:'execute',count:2,work_type:'ui',case_ids:cases,case_specs:specs},scope.capability);
 assert.deepEqual(r.assignments.map(a=>a.case_ids),[['BITS-001-a'],['BITS-002-b']]);
 const a=await f.claim(r,'0','ui-session');
 const input={...f.body(a),ui_run_id:'ui-'+digest(JSON.stringify([r.reservation_id,a.instance_id,'BITS-001-a'])).slice(0,40),execution_id:'dag-ui',
  case_id:'BITS-001-a',spec_sha256:'a'.repeat(64),input_version:'v1'};
 assert.throws(()=>startUiWork(f.store.read(),r.reservation_id,{...input,case_id:'BITS-002-b'},a.capability),/identity mismatch/);
 assert.throws(()=>startUiWork(f.store.read(),r.reservation_id,{...input,spec_sha256:'c'.repeat(64)},a.capability),/identity mismatch/);
 assert.equal((await f.api.handle('POST',`/v1/reservations/${r.reservation_id}/work`,
  {...f.body(a),native_run_id:'nh-case'},a.capability).then(()=>null,e=>e.status)),403);
 const work=await f.api.handle('POST',`/v1/reservations/${r.reservation_id}/ui-work`,input,a.capability);
 assert.equal(work.kind,'ui');
 await f.store.transaction(s=>{
  assert.deepEqual(reclaim(s,r.reservation_id,'done'),['win-03']);
  assert.equal(s.devices['win-02'].reservation_id,r.reservation_id);
 });
 assert.throws(()=>startUiWork(f.store.read(),r.reservation_id,{...input,generation:a.generation-1},a.capability),/Stale ownership/);
});
