import test from 'node:test';
import assert from 'node:assert/strict';
import {rm,mkdir,writeFile,readFile,symlink} from 'node:fs/promises';
import {join,dirname} from 'node:path';
import {openStore} from '../src/store.mjs';
import {service} from '../src/service.mjs';
import {FILES} from '../src/receipts.mjs';
import {digest} from '../src/state.mjs';
import {ADMIN,fixture} from './support/fixture.mjs';
test('batch competition is all-or-none; quarantine is never assigned',async t=>{
 const f=await fixture(t,3),results=await Promise.allSettled([f.allocate('dag-a',2),f.allocate('dag-b',2)]);
 assert.equal(results.filter(r=>r.status==='fulfilled').length,1);
 const s=f.store.read();assert.equal(Object.values(s.devices).filter(d=>d.reservation_id).length,2);assert.equal(Object.keys(s.reservations).length,1);
});
test('Harness quarantine file is persisted before allocation and cannot be cleared by file removal',async t=>{
 const f=await fixture(t,3),file=join(f.root,'quarantine','win-02.json');
 await mkdir(dirname(file),{recursive:true});await writeFile(file,'{"idle_verified":false}');
 await f.api.poll();
 assert.equal(f.store.read().devices['win-02'].quarantine,'native_harness_quarantine');
 const r=await f.allocate('dag-quarantine',1);
 assert.equal(r.assignments[0].machine,'win-03');
 assert.equal(f.store.read().devices['win-02'].quarantine,'native_harness_quarantine');
 await rm(file);
 assert.equal(f.store.read().devices['win-02'].quarantine,'native_harness_quarantine');
});
test('allocation and claim survive process-store restart with identical identities',async t=>{
 const f=await fixture(t),r=await f.allocate(),a=await f.claim(r);
 const store=await openStore(f.file),api=service(f.config,store,f.remote,ADMIN);
 const same=await api.handle('POST',`/v1/reservations/${r.reservation_id}/claims`,f.body(a),ADMIN);
 assert.equal(same.capability,a.capability);assert.equal(same.machine,a.machine);assert.equal((await f.allocate()).reservation_id,r.reservation_id);
});
test('root token is not an allocator scope; scope count cannot expand',async t=>{
 const f=await fixture(t);await assert.rejects(f.api.handle('POST','/v1/reservations',{dag_id:'dag-one',target_step:'execute',count:1},ADMIN),/capability/);
 const scope=await f.api.handle('POST','/v1/step-sessions',{dag_id:'dag-one',step_id:'allocate',session_id:'alloc',role:'allocate',target_step:'execute',count:1},ADMIN);
 await assert.rejects(f.api.handle('POST','/v1/reservations',{dag_id:'dag-one',target_step:'execute',count:2},scope.capability),/mismatch/);
});
test('one session cannot bind two devices',async t=>{
 const f=await fixture(t),r=await f.allocate('dag-one',2);await f.claim(r,'0','same');await assert.rejects(f.claim(r,'1','same'),/identity|another/);
});
test('old session cannot be evicted until trusted host terminal proof; old generation is fenced',async t=>{
 const f=await fixture(t),r=await f.allocate(),a=await f.claim(r);f.remote.sessionError=true;
 await assert.rejects(f.claim(r,'0','replacement'),/unknown/);
 await f.api.handle('POST',`/v1/step-sessions/${a.session_id}/finish`,{dag_id:r.dag_id,step_id:r.target_step,instance_id:'0',outcome:'cancelled'},ADMIN);
 const b=await f.claim(r,'0','replacement');assert.equal(b.generation,a.generation+1);assert.notEqual(b.capability,a.capability);
 await assert.rejects(f.work(r,a),/generation/);await f.work(r,b);
});
test('case completion is not release and next case waits for verified receipts',async t=>{
 const f=await fixture(t),r=await f.allocate(),a=await f.claim(r);await f.work(r,a);
 await assert.rejects(f.work(r,a,'nh-case-two'),/Previous/);
 await f.complete(r,a,'nh-case-one',await f.proofs(r,a));assert.equal(f.store.read().devices[a.machine].reservation_id,r.reservation_id);
 await assert.rejects(f.work(r,a),/restored/);await f.work(r,a,'nh-case-two');
 f.remote.status='done';await f.api.poll();assert.equal(f.store.read().devices[a.machine].reservation_id,r.reservation_id);
});
test('cancelled DAG releases recovered device independently of another incomplete one',async t=>{
 const f=await fixture(t),r=await f.allocate('dag-one',2),a=await f.claim(r,'0'),b=await f.claim(r,'1');
 await f.work(r,a,'nh-case-a');await f.work(r,b,'nh-case-b');await f.complete(r,a,'nh-case-a',await f.proofs(r,a,'nh-case-a'));
 f.remote.status='cancelled';await f.api.poll();const s=f.store.read();assert.equal(s.devices[a.machine].reservation_id,null);assert.equal(s.devices[b.machine].reservation_id,r.reservation_id);
});
test('unknown server state does not release; never-claimed assignment releases on terminal',async t=>{
 const f=await fixture(t),r=await f.allocate();f.remote.error=true;await f.api.poll();assert.equal(f.store.read().devices[r.assignments[0].machine].reservation_id,r.reservation_id);
 f.remote.error=false;f.remote.status='error';await f.api.poll();assert.equal(f.store.read().devices[r.assignments[0].machine].reservation_id,null);
});
test('claimed without work requires session termination; unknown session cannot block unrelated device',async t=>{
 const f=await fixture(t),r=await f.allocate('dag-one',2),a=await f.claim(r);f.remote.status='cancelled';f.remote.sessionError=true;await f.api.poll();
 assert.equal(f.store.read().devices[a.machine].reservation_id,r.reservation_id);assert.equal(f.store.read().devices[r.assignments[1].machine].reservation_id,null);
 await f.api.handle('POST',`/v1/step-sessions/${a.session_id}/finish`,{dag_id:r.dag_id,step_id:r.target_step,instance_id:'0',outcome:'cancelled'},ADMIN);await f.api.poll();assert.equal(f.store.read().devices[a.machine].reservation_id,null);
});
test('bad receipt hashes, mismatched owner and arbitrary paths fail closed',async t=>{
 const f=await fixture(t),r=await f.allocate(),a=await f.claim(r);await f.work(r,a);const receipts=await f.proofs(r,a);
 await assert.rejects(f.complete(r,a,'nh-case-one',{...receipts,owner:'0'.repeat(64)}),/changed/);
 await assert.rejects(f.complete(r,a,'nh-case-one',{...receipts,'../outside':'0'.repeat(64)}),/Unknown/);
 const file=join(f.config.evidence_root,'nh-case-one',FILES.owner),x=JSON.parse(await readFile(file));x.machine='win-19';const bytes=JSON.stringify(x);await writeFile(file,bytes);receipts.owner=digest(bytes);
 await assert.rejects(f.complete(r,a,'nh-case-one',receipts),/owner/);
});
test('evidence modified after completion cannot release at DAG termination',async t=>{
 const f=await fixture(t),r=await f.allocate(),a=await f.claim(r);await f.work(r,a);await f.complete(r,a,'nh-case-one',await f.proofs(r,a));
 await writeFile(join(f.config.evidence_root,'nh-case-one',FILES.cleanup),'{}');f.remote.status='done';await f.api.poll();assert.equal(f.store.read().devices[a.machine].reservation_id,r.reservation_id);
});
test('symlink receipt is rejected even with matching bytes',async t=>{
 const f=await fixture(t),r=await f.allocate(),a=await f.claim(r);await f.work(r,a);const receipts=await f.proofs(r,a);const file=join(f.config.evidence_root,'nh-case-one',FILES.cleanup),target=join(f.root,'external');await writeFile(target,await readFile(file));await rm(file);await symlink(target,file);
 await assert.rejects(f.complete(r,a,'nh-case-one',receipts),/symlink/);
});
test('cancelled DAG only permits recovering existing work, including trusted session rebind',async t=>{
 const f=await fixture(t),r=await f.allocate(),a=await f.claim(r);await f.work(r,a);f.remote.status='cancelled';
 assert.equal((await f.work(r,a)).recovery_only,true);await assert.rejects(f.work(r,a,'nh-new-case'),/new work/);
 await f.api.handle('POST',`/v1/step-sessions/${a.session_id}/finish`,{dag_id:r.dag_id,step_id:r.target_step,instance_id:'0',outcome:'cancelled'},ADMIN);
 const b=await f.claim(r,'0','recovery-session');assert.equal(b.recovery_only,true);
 await f.complete(r,b,'nh-case-one',await f.proofs(r,a));await f.api.poll();assert.equal(f.store.read().devices[a.machine].reservation_id,null);
});
test('early interruption proves no capture, without fabricating capture receipts',async t=>{
 const f=await fixture(t),r=await f.allocate(),a=await f.claim(r);await f.work(r,a);const receipts=await f.proofs(r,a);
 for(const name of ['capture','manifest','proxy','firewall','restored']){await rm(join(f.config.evidence_root,'nh-case-one',FILES[name]));delete receipts[name];}
 const values={run:{run_id:'nh-case-one',machine:{name:a.machine},task_start_attempted:false},cleanup:{restoration_verified:true,original_untouched:true}};
 for(const [name,value] of Object.entries(values)){const bytes=JSON.stringify(value);await writeFile(join(f.config.evidence_root,'nh-case-one',FILES[name]),bytes);receipts[name]=digest(bytes);}
 await f.complete(r,a,'nh-case-one',receipts);f.remote.status='error';await f.api.poll();assert.equal(f.store.read().devices[a.machine].reservation_id,null);
});
test('incomplete capture startup cannot use untouched shortcut',async t=>{
 const f=await fixture(t),r=await f.allocate(),a=await f.claim(r);await f.work(r,a);const receipts=await f.proofs(r,a);
 await rm(join(f.config.evidence_root,'nh-case-one',FILES.capture));delete receipts.capture;
 for(const [name,value] of Object.entries({run:{run_id:'nh-case-one',machine:{name:a.machine},task_start_attempted:false},cleanup:{restoration_verified:true,original_untouched:true}})){
  const bytes=JSON.stringify(value);await writeFile(join(f.config.evidence_root,'nh-case-one',FILES[name]),bytes);receipts[name]=digest(bytes);
 }
 await assert.rejects(f.complete(r,a,'nh-case-one',receipts),/Interrupted capture/);
});
test('failed capture with verified shutdown releases capacity without erasing failure',async t=>{
 const f=await fixture(t),r=await f.allocate(),a=await f.claim(r);await f.work(r,a);const receipts=await f.proofs(r,a),p=join(f.config.evidence_root,'nh-case-one',FILES.manifest);
 const value=JSON.parse(await readFile(p));value.state='stop_failed';const bytes=JSON.stringify(value);await writeFile(p,bytes);receipts.manifest=digest(bytes);
 assert.equal((await f.complete(r,a,'nh-case-one',receipts)).capture_failed,true);f.remote.status='error';await f.api.poll();assert.equal(f.store.read().devices[a.machine].reservation_id,null);
});
test('capture-only prepare failure uses real collector cleanup, no supervisor receipts',async t=>{
 const f=await fixture(t),r=await f.allocate(),a=await f.claim(r);await f.work(r,a);const receipts=await f.proofs(r,a);
 for(const name of ['proxy','restored']){await rm(join(f.config.evidence_root,'nh-case-one',FILES[name]));delete receipts[name];}
 async function change(name,value){const bytes=JSON.stringify(value);await writeFile(join(f.config.evidence_root,'nh-case-one',FILES[name]),bytes);receipts[name]=digest(bytes);}
 await change('run',{run_id:'nh-case-one',machine:{name:a.machine},task_start_attempted:false});await change('cleanup',{restoration_verified:true,original_untouched:true});
 const manifest=JSON.parse(await readFile(join(f.config.evidence_root,'nh-case-one',FILES.manifest)));manifest.integrity={https:{cleanup:{restored:true,collectorAlive:false,active_marker_removed:true}}};
 for(const invalid of [{restored:false,collectorAlive:false,active_marker_removed:true},{restored:true,collectorAlive:true,active_marker_removed:true},{restored:true,collectorAlive:false}]){
  await change('manifest',{...manifest,integrity:{https:{cleanup:invalid}}});await assert.rejects(f.complete(r,a,'nh-case-one',receipts),/Capture-only/);
 }
 await change('manifest',{...manifest,cleanup_verified:false});await assert.rejects(f.complete(r,a,'nh-case-one',receipts),/cleanup\/ownership/);
 await change('manifest',{...manifest,capture_id:'different'});await assert.rejects(f.complete(r,a,'nh-case-one',receipts),/cleanup\/ownership/);
 await change('manifest',manifest);await f.complete(r,a,'nh-case-one',receipts);f.remote.status='error';await f.api.poll();assert.equal(f.store.read().devices[a.machine].reservation_id,null);
});
test('capture-only proof cannot substitute for started supervisor restoration',async t=>{
 const f=await fixture(t),r=await f.allocate(),a=await f.claim(r);await f.work(r,a);const receipts=await f.proofs(r,a);
 for(const name of ['proxy','restored']){await rm(join(f.config.evidence_root,'nh-case-one',FILES[name]));delete receipts[name];}
 const p=join(f.config.evidence_root,'nh-case-one',FILES.manifest),manifest=JSON.parse(await readFile(p));manifest.integrity={https:{cleanup:{restored:true,collectorAlive:false,noop_verified:true}}};const bytes=JSON.stringify(manifest);await writeFile(p,bytes);receipts.manifest=digest(bytes);
 await assert.rejects(f.complete(r,a,'nh-case-one',receipts),/Task\/application/);
});
