import test from 'node:test';
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {createHash,createHmac} from 'node:crypto';
import {mkdir,readFile,writeFile} from 'node:fs/promises';
import {join} from 'node:path';
import {fixture,ADMIN} from './support/fixture.mjs';
import {digest} from '../src/state.mjs';

test('scoped UI action is signed, persisted once, and never replayed after a lost outcome',async t=>{
 const f=await fixture(t,2),secret='c'.repeat(64),tokenFile=join(f.root,'worker-token');
 await writeFile(tokenFile,secret);
 let calls=0,failNext=false,falseSuccess=false;
 const node=createServer(async(req,res)=>{
  const chunks=[];for await(const chunk of req)chunks.push(chunk);const body=Buffer.concat(chunks);
  const stamp=req.headers['x-viking-auth-timestamp'],nonce=req.headers['x-viking-auth-nonce'];
  const signed=[stamp,nonce,req.method,req.url,createHash('sha256').update(body).digest('hex')].join('\n');
  assert.equal(req.headers['x-viking-auth-signature'],createHmac('sha256',secret).update(signed).digest('hex'));
  calls++;
  if(req.method==='GET'){
   res.writeHead(200,{'content-type':'image/png'});
   res.end(Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]),Buffer.from('screenshot')]));
  }else{
   res.writeHead(failNext?503:200,{'content-type':'application/json'});
   res.end(JSON.stringify({error:falseSuccess?'action failed':null,result:{},request_id:JSON.parse(body).request_id}));
  }
 });
 await new Promise(resolve=>node.listen(0,'127.0.0.1',resolve));
 t.after(()=>new Promise(resolve=>node.close(resolve)));
 const scope=await f.api.handle('POST','/v1/step-sessions',{
  dag_id:'dag-ui-action',step_id:'allocate',session_id:'allocator-ui',role:'allocate',
  target_step:'execute',count:1,work_type:'ui',case_ids:['BITS-1-a'],
  case_specs:[{case_id:'BITS-1-a',spec_sha256:'a'.repeat(64),input_version:'v1'}]},ADMIN);
 const r=await f.api.handle('POST','/v1/reservations',{
  dag_id:'dag-ui-action',target_step:'execute',count:1,work_type:'ui',case_ids:['BITS-1-a'],
  case_specs:[{case_id:'BITS-1-a',spec_sha256:'a'.repeat(64),input_version:'v1'}]},scope.capability);
 const a=await f.claim(r,'0','ui-action-session');
 f.config.ui_workers_file=join(f.root,'workers.json');
 await writeFile(f.config.ui_workers_file,JSON.stringify({workers:[{name:a.machine,platform:'windows',
  node_base:`http://127.0.0.1:${node.address().port}/`,token_file:tokenFile,session_id:'guest'}]}));
 const work={...f.body(a),ui_run_id:'ui-'+digest(JSON.stringify([r.reservation_id,a.instance_id,'BITS-1-a'])).slice(0,40),execution_id:'dag-ui-action',
  case_id:'BITS-1-a',spec_sha256:'a'.repeat(64),input_version:'v1'};
 await f.api.handle('POST',`/v1/reservations/${r.reservation_id}/ui-work`,work,a.capability);
 const route=`/v1/reservations/${r.reservation_id}/ui-actions`;
 const action={...f.body(a),ui_run_id:work.ui_run_id,case_id:work.case_id,
  operation_id:'op-1',action:'list-windows',arguments:{}};
 await assert.rejects(f.api.handle('POST',route,{...action,operation_id:'op-click-before',action:'click',arguments:{x:1,y:2}},a.capability),/capture must start/);
 assert.equal((await f.api.handle('POST',route,action,a.capability)).status,'done');
 assert.equal((await f.api.handle('POST',route,action,a.capability)).status,'done');
 assert.equal(calls,1);
 await assert.rejects(f.api.handle('POST',route,{...action,arguments:{foreign:true}},a.capability),/identity changed/);
 await assert.rejects(f.api.handle('POST',route,{...action,operation_id:'op-cross',instance_id:'1'},a.capability));
 assert.equal(calls,1);
 const screenshot={...action,operation_id:'op-shot',action:'screenshot'};
 const shotRoute=`/v1/reservations/${r.reservation_id}/ui-screenshots`;
 await assert.rejects(f.api.handle('POST',route,screenshot,a.capability),/route mismatch/);
 const shot=await f.api.handle('POST',shotRoute,screenshot,a.capability);
 assert.equal(shot.status,'done');
 const bytes=await readFile(join(f.config.evidence_root,'ui',work.ui_run_id,shot.result.file));
 assert.equal(digest(bytes),shot.result.sha256);
 assert.equal((await f.api.handle('POST',shotRoute,screenshot,a.capability)).result.sha256,shot.result.sha256);
 assert.equal(calls,2);
 const driver=join(f.root,'driver-windows');await mkdir(driver);
 await writeFile(join(driver,'cli.py'),"import json,sys\nprint(json.dumps({'ok':True,'capture_id':'cap-test','state':'running' if sys.argv[1]=='network_capture_start' else 'stopped'}))\n");
 const captureRoute=`/v1/reservations/${r.reservation_id}/ui-capture`;
 const started=await f.api.handle('POST',captureRoute,{...action,operation_id:'op-capture-start',action:'capture-start'},a.capability);
 assert.equal(started.result.state,'running');
 assert.equal((await f.api.handle('POST',route,{...action,operation_id:'op-click',action:'click',arguments:{x:1,y:2}},a.capability)).status,'done');
 assert.equal((await f.api.handle('POST',captureRoute,{...action,operation_id:'op-capture-stop',action:'capture-stop'},a.capability)).result.state,'stopped');
 failNext=true;
 await assert.rejects(f.api.handle('POST',route,{...action,operation_id:'op-uncertain'},a.capability));
 await assert.rejects(f.api.handle('POST',route,{...action,operation_id:'op-uncertain'},a.capability),/pending or uncertain/);
 assert.equal(calls,4);
 const reconcileRoute=`/v1/reservations/${r.reservation_id}/ui-reconcile`;
 await assert.rejects(f.api.handle('POST',reconcileRoute,{...action,operation_id:'op-uncertain'},a.capability),/no safe replay proof/);
 const png=Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]),Buffer.from('persisted')]);
 await writeFile(join(f.config.evidence_root,'ui',work.ui_run_id,'screenshots','op-recovered.png'),png);
 await f.store.transaction(state=>{state.reservations[r.reservation_id].assignments[0].works[0].operations['op-recovered']={action:'screenshot',status:'uncertain'};});
 const recovered=await f.api.handle('POST',reconcileRoute,{...action,operation_id:'op-recovered',action:'screenshot'},a.capability);
 assert.equal(recovered.result.sha256,digest(png));assert.equal(calls,4);
 const manifest=join(f.config.evidence_root,'ui',work.ui_run_id,'network','manifest.json');
 await mkdir(join(f.config.evidence_root,'ui',work.ui_run_id,'network'),{recursive:true});
 await writeFile(manifest,JSON.stringify({case_key:work.case_id,worker:a.machine,capture_id:'cap-test',state:'stopped',cleanup_verified:true}));
 await f.store.transaction(state=>{state.reservations[r.reservation_id].assignments[0].works[0].operations['op-capture-stop'].status='uncertain';});
 const stopped=await f.api.handle('POST',reconcileRoute,{...action,operation_id:'op-capture-stop',action:'capture-stop'},a.capability);
 assert.equal(stopped.result.capture_id,'cap-test');assert.equal(calls,4);
 failNext=false;falseSuccess=true;
 await assert.rejects(f.api.handle('POST',route,{...action,operation_id:'op-false-success'},a.capability),/did not confirm/);
 assert.equal(f.store.read().reservations[r.reservation_id].assignments[0].works[0].operations['op-false-success'].status,'uncertain');
});
