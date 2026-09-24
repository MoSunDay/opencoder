import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdir,writeFile} from 'node:fs/promises';
import {dirname,join} from 'node:path';
import {fixture,ADMIN} from './support/fixture.mjs';
import {digest} from '../src/state.mjs';

test('UI completion independently checks frozen assertions, capture, restoration, and hashes',async t=>{
 const f=await fixture(t,1),case_id='BITS-1-a';
 const spec={case_key:case_id,explicit_semantic_review:{execution_assertions:[{id:'assert-a'}]}};
 const specBytes=Buffer.from(JSON.stringify(spec)),specHash=digest(specBytes);
 const cases=[{case_id,spec_sha256:specHash,input_version:'v1'}];
 const scope=await f.api.handle('POST','/v1/step-sessions',{
  dag_id:'dag-ui-proof',step_id:'allocate',session_id:'ui-allocator-proof',role:'allocate',
  target_step:'execute',count:1,work_type:'ui',case_ids:[case_id],case_specs:cases},ADMIN);
 const r=await f.api.handle('POST','/v1/reservations',{
  dag_id:'dag-ui-proof',target_step:'execute',count:1,work_type:'ui',case_ids:[case_id],case_specs:cases},scope.capability);
 const a=await f.claim(r,'0','ui-proof-session');
 const ui_run_id='ui-'+digest(JSON.stringify([r.reservation_id,a.instance_id,case_id])).slice(0,40);
 const input={...f.body(a),ui_run_id,execution_id:r.dag_id,case_id,spec_sha256:specHash,input_version:'v1'};
 await f.api.handle('POST',`/v1/reservations/${r.reservation_id}/ui-work`,input,a.capability);
 const folder=join(f.config.evidence_root,'ui',ui_run_id),hashes={};
 async function evidence(name,path,value){const bytes=Buffer.isBuffer(value)?value:Buffer.from(JSON.stringify(value));
  const file=join(folder,path);await mkdir(dirname(file),{recursive:true});await writeFile(file,bytes);hashes[name]=digest(bytes);return file;}
 const owner={reservation_id:r.reservation_id,dag_id:r.dag_id,instance_id:a.instance_id,
  session_id:a.session_id,generation:a.generation,machine:a.machine,ui_run_id,case_id,
  spec_sha256:specHash,input_version:'v1'};
 await evidence('owner','owner.json',owner);await evidence('spec','case-spec.json',specBytes);
 const screenshot=Buffer.from('current-run screenshot');const shotPath=join(folder,'screenshots','final.png');
 await mkdir(dirname(shotPath),{recursive:true});await writeFile(shotPath,screenshot);
 const network=Buffer.from('current-run network capture');const networkPath=join(folder,'network','private','traffic.jsonl');
 await mkdir(dirname(networkPath),{recursive:true});await writeFile(networkPath,network);
 const result={case_id,ui_run_id,status:'failed',assertions:[{id:'assert-a',status:'failed',detail:'observed mismatch',evidence:['screenshots/final.png','network/private/traffic.jsonl']}],
  screenshots:[{file:'screenshots/final.png',sha256:digest(screenshot)}],
  network_files:[{file:'network/private/traffic.jsonl',sha256:digest(network)}]};
 await evidence('result','result.json',result);
 await evidence('capture','network/manifest.json',{case_key:case_id,worker:a.machine,capture_id:'capture-one',state:'stopped',cleanup_verified:true,
  integrity:{cdp:{complete_jsonl:true}},cdp_stop:{cleanup:{collectorAlive:false}},
  coverage:{full_app_capture:true},files:[{path:'private/traffic.jsonl',sha256:digest(network)}]});
 await evidence('recovery','recovery.json',{ui_run_id,machine:a.machine,application_restored:true,
  task_removed:true,proxy_restored:true,firewall_removed:true,collector_alive:false});
 const route=`/v1/reservations/${r.reservation_id}/ui-completions`;
 await assert.rejects(f.api.handle('POST',route,{...input,receipts:hashes},a.capability),/registered operations/);
 await f.store.transaction(state=>{
  const operations=state.reservations[r.reservation_id].assignments[0].works[0].operations;
  operations['op-start']={action:'capture-start',status:'done',result:{capture_id:'capture-one'}};
  operations['op-stop']={action:'capture-stop',status:'done',result:{capture_id:'capture-one'}};
  operations['op-shot']={action:'screenshot',status:'done',result:result.screenshots[0]};
 });
 await f.store.transaction(state=>{state.reservations[r.reservation_id].assignments[0].works[0].operations['op-shot'].result.sha256='0'.repeat(64);});
 await assert.rejects(f.api.handle('POST',route,{...input,receipts:hashes},a.capability),/registered capture operations/);
 await f.store.transaction(state=>{state.reservations[r.reservation_id].assignments[0].works[0].operations['op-shot'].result.sha256=result.screenshots[0].sha256;});
 await f.store.transaction(state=>{state.reservations[r.reservation_id].assignments[0].works[0].operations['op-unknown']={action:'click',status:'uncertain'};});
 await assert.rejects(f.api.handle('POST',route,{...input,receipts:hashes},a.capability),/manager reconciliation/);
 await f.store.transaction(state=>{delete state.reservations[r.reservation_id].assignments[0].works[0].operations['op-unknown'];});
 const resultPath=join(folder,'result.json');
 const missingProof={...result,assertions:[{...result.assertions[0],evidence:[]}]};
 const missingBytes=JSON.stringify(missingProof);await writeFile(resultPath,missingBytes);
 await assert.rejects(f.api.handle('POST',route,{...input,receipts:{...hashes,result:digest(missingBytes)}},a.capability),/assertion evidence/);
 await writeFile(resultPath,JSON.stringify(result));
 await assert.rejects(f.api.handle('POST',route,{...input,receipts:{...hashes,owner:'0'.repeat(64)}},a.capability),/evidence changed/);
 assert.equal(f.store.read().devices[a.machine].reservation_id,r.reservation_id);
 const completed=await f.api.handle('POST',route,{...input,receipts:hashes},a.capability);
 assert.equal(completed.case_status,'failed');assert.equal(completed.evidence_complete,true);
 await writeFile(shotPath,'tampered');f.remote.status='done';await f.api.poll();
 assert.equal(f.store.read().devices[a.machine].reservation_id,r.reservation_id);
 assert.equal(f.store.read().reservations[r.reservation_id].assignments[0].works[0].recovery_verified,false);
 await writeFile(shotPath,screenshot);
 await f.api.handle('POST',route,{...input,receipts:hashes},a.capability);
 await f.api.poll();
 assert.equal(f.store.read().devices[a.machine].reservation_id,null);
});
