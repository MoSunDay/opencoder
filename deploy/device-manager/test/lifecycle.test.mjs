import test from 'node:test';
import assert from 'node:assert/strict';
import {writeFile} from 'node:fs/promises';
import {join} from 'node:path';
import {ADMIN,fixture} from './support/fixture.mjs';
import {FILES} from '../src/receipts.mjs';
import {openStore} from '../src/store.mjs';
import {service} from '../src/service.mjs';

const allocatorInput={dag_id:'dag-one',step_id:'allocate',session_id:'allocator-dag-one',role:'allocate',target_step:'execute',count:1};
const finish=(f,session,input)=>f.api.handle('POST',`/v1/step-sessions/${session}/finish`,input,ADMIN);

test('finished allocator cannot reserve devices or re-register its scope',async t=>{
 const f=await fixture(t);
 const scope=await f.api.handle('POST','/v1/step-sessions',allocatorInput,ADMIN);
 await finish(f,scope.session_id,{dag_id:'dag-one',step_id:'allocate',instance_id:null,outcome:'done'});
 await assert.rejects(f.api.handle('POST','/v1/reservations',allocatorInput,scope.capability),/ended|terminal/i);
 await assert.rejects(f.api.handle('POST','/v1/step-sessions',allocatorInput,ADMIN),/ended|terminal/i);
 assert.equal(Object.keys(f.store.read().reservations).length,0);
});

test('unknown DAG state cannot authorize registration, claim or new work',async t=>{
 const f=await fixture(t),r=await f.allocate(),a=await f.claim(r);
 for(const status of [undefined,null,'unknown','unexpected']){
  f.remote.status=status;
  await assert.rejects(f.api.handle('POST','/v1/step-sessions',{...allocatorInput,session_id:'new'},ADMIN),/state|status/i);
  await assert.rejects(f.claim(r),/state|status/i);
  await assert.rejects(f.work(r,a),/state|status/i);
 }
 assert.equal(f.store.read().reservations[r.reservation_id].assignments[0].works.length,0);
});

test('rebound session can only recover work owned by the prior session',async t=>{
 const f=await fixture(t),r=await f.allocate(),a=await f.claim(r);
 await f.work(r,a);
 await finish(f,a.session_id,{dag_id:r.dag_id,step_id:r.target_step,instance_id:'0',outcome:'error'});
 const b=await f.claim(r,'0','replacement');
 assert.equal(b.recovery_only,true);
 assert.equal((await f.work(r,b)).recovery_only,true);
 await assert.rejects(f.work(r,b,'nh-next'),/Previous/);
 await f.complete(r,b,'nh-case-one',await f.proofs(r,a));
 assert.equal((await f.work(r,b,'nh-next')).recovery_only,false);
});

test('altered restoration evidence blocks the next case before any work is registered',async t=>{
 const f=await fixture(t),r=await f.allocate(),a=await f.claim(r);
 await f.work(r,a);await f.complete(r,a,'nh-case-one',await f.proofs(r,a));
 await writeFile(join(f.config.evidence_root,'nh-case-one',FILES.cleanup),'{}');
 await assert.rejects(f.work(r,a,'nh-next'),/changed|recovery/i);
 assert.equal(f.store.read().reservations[r.reservation_id].assignments[0].works.length,1);
});

test('delayed running response cannot start work after reconciliation observed termination',async t=>{
 const f=await fixture(t),r=await f.allocate(),a=await f.claim(r);
 let resume,entered;
 const waiting=new Promise(resolve=>{entered=resolve;});
 f.remote.dag=()=>{entered();return new Promise(resolve=>{resume=resolve;});};
 const pending=f.work(r,a);
 await waiting;
 f.remote.dag=async()=> 'cancelled';
 await f.api.poll();
 const rejected=assert.rejects(pending,/terminal|new work/i);
 resume('running');await rejected;
 assert.equal(f.store.read().reservations[r.reservation_id].assignments[0].works.length,0);
 assert.equal(f.store.read().devices[a.machine].reservation_id,r.reservation_id);
});

test('allocator session ending while upstream is queried fences the in-flight reservation',async t=>{
 const f=await fixture(t),scope=await f.api.handle('POST','/v1/step-sessions',allocatorInput,ADMIN);
 let resume,entered;const waiting=new Promise(resolve=>{entered=resolve;});
 f.remote.dag=()=>{entered();return new Promise(resolve=>{resume=resolve;});};
 const pending=f.api.handle('POST','/v1/reservations',allocatorInput,scope.capability);
 await waiting;
 await finish(f,scope.session_id,{dag_id:'dag-one',step_id:'allocate',instance_id:null,outcome:'done'});
 const rejected=assert.rejects(pending,/ended|terminal/i);
 resume('running');await rejected;
 assert.equal(Object.keys(f.store.read().reservations).length,0);
});

test('terminal observation survives restart and rejects stale allocation and work responses',async t=>{
 const f=await fixture(t),r=await f.allocate(),a=await f.claim(r);
 f.remote.status='cancelled';await f.api.poll();
 const store=await openStore(f.file),api=service(f.config,store,f.remote,ADMIN);
 f.remote.status='running';
 await assert.rejects(api.handle('POST',`/v1/reservations/${r.reservation_id}/work`,{...f.body(a),native_run_id:'nh-late'},a.capability),/terminal/);
 await assert.rejects(api.handle('POST','/v1/step-sessions',{...allocatorInput,session_id:'late-allocator',target_step:'other'},ADMIN),/terminal/);
 assert.equal(store.read().devices[a.machine].reservation_id,r.reservation_id);
});

test('concurrent pool exhaustion, retry, recovery and reuse preserve exclusive ownership',async t=>{
 const f=await fixture(t,18);
 const results=await Promise.allSettled(Array.from({length:30},(_,i)=>f.allocate('dag-'+i)));
 const reservations=results.filter(r=>r.status==='fulfilled').map(r=>r.value);
 assert.equal(reservations.length,18);
 assert.equal(new Set(reservations.map(r=>r.assignments[0].machine)).size,18);
 assert(results.filter(r=>r.status==='rejected').every(r=>r.reason.status===409));
 const r=reservations[0];
 const retried=await Promise.all(Array.from({length:20},()=>f.allocate(r.dag_id)));
 assert(retried.every(value=>value.reservation_id===r.reservation_id));
 const claims=await Promise.allSettled(Array.from({length:12},(_,i)=>f.claim(r,'0','competing-'+i)));
 assert.equal(claims.filter(r=>r.status==='fulfilled').length,1);
 const a=claims.find(r=>r.status==='fulfilled').value;
 await f.work(r,a);await f.complete(r,a,'nh-case-one',await f.proofs(r,a));
 f.remote.status='done';await f.api.poll();
 assert(Object.values(f.store.read().devices).every(d=>d.reservation_id===null));
 f.remote.status='running';
 const reused=await f.allocate('dag-reused',18);
 assert.equal(new Set(reused.assignments.map(a=>a.machine)).size,18);
 assert(reused.assignments.every(a=>a.generation===2));
 await assert.rejects(f.work(r,a),/generation/);
 const state=f.store.read();
 for(const device of Object.values(state.devices)){
  const owners=Object.values(state.reservations).flatMap(r=>r.assignments).filter(a=>!a.released&&a.machine===device.machine);
  assert.equal(owners.length,1);assert.equal(owners[0].generation,device.generation);
 }
});

test('failed mutation rolls back completely and does not block later reservations',async t=>{
 const f=await fixture(t,1),before=f.store.read();
 await assert.rejects(f.store.transaction(state=>{state.devices['win-02'].reservation_id='broken';throw Error('abort');}),/abort/);
 assert.deepEqual(f.store.read(),before);
 assert.deepEqual((await openStore(f.file)).read(),before);
 const r=await f.allocate();assert.equal(r.assignments[0].machine,'win-02');
});

test('mismatched device ledger prevents an assignment from claiming or starting work',async t=>{
 const f=await fixture(t),r=await f.allocate(),a=await f.claim(r);
 await f.store.transaction(state=>{state.devices[a.machine].generation++;});
 await assert.rejects(f.work(r,a),/ownership/);
 await assert.rejects(f.claim(r),/ownership/);
});
