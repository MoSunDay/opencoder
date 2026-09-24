import test from 'node:test';
import assert from 'node:assert/strict';
import {getJson,remote} from '../src/remote.mjs';
import {mkdtemp,writeFile,rm} from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import {createServer} from 'node:http';

const ok=()=>({ok:true,json:async()=>({status:'running'})});
function clock(fetch){let t=0;return {now:()=>t,sleep:async ms=>{t+=ms;},fetch};}
test('read-only transient HTTP retries and then uses actual response',async()=>{
 let calls=0;const d=clock(async(url,options)=>{
  assert.equal(options.redirect,'error');assert.equal(options.method,undefined);
  return ++calls<3?{ok:false,status:503}:ok();
 });
 assert.deepEqual(await getJson('http://local','secret',10000,d),{status:'running'});
 assert.equal(calls,3);assert.equal(d.now(),750);
});
test('transient failures consume one total ten-second budget',async()=>{
 let calls=0;const d=clock(async()=>{calls++;return {ok:false,status:502};});
 await assert.rejects(getJson('http://local','secret',10000,d),e=>e.code==='upstream_http'&&e.upstream_status===502&&e.attempts===8);
 assert.ok(d.now()<=10000);assert.equal(calls,8);
});
test('slow attempts do not reset deadline',async()=>{
 let t=0,calls=0;const d={now:()=>t,sleep:async ms=>{t+=ms;},fetch:async()=>{calls++;t+=4000;throw Object.assign(Error(),{cause:{code:'ECONNRESET'}});}};
 await assert.rejects(getJson('http://local','secret',8500,d),e=>e.code==='upstream_transport');
 assert.equal(calls,2);assert.equal(t,8500);
});
test('non-transient HTTP including auth, absent and rate limit never retries',async()=>{
 for(const status of [400,401,403,404,409,429,500]){
  let calls=0;const d=clock(async()=>{calls++;return {ok:false,status};});
  await assert.rejects(getJson('http://local','secret',10000,d),e=>e.upstream_status===status&&e.attempts===1);
  assert.equal(calls,1);
 }
});
test('malformed successful content is permanent and no response body leaks',async()=>{
 let calls=0;const d=clock(async()=>{calls++;return {ok:true,json:async()=>{throw Error('secret body');}};});
 await assert.rejects(getJson('http://local','secret',10000,d),e=>e.code==='upstream_content'&&!e.message.includes('secret'));
 assert.equal(calls,1);
});
test('unknown network error is not guessed transient',async()=>{
 let calls=0;await assert.rejects(getJson('http://local','secret',10000,clock(async()=>{calls++;throw Error('secret data');})),e=>e.code==='upstream_query'&&!e.message.includes('secret'));
 assert.equal(calls,1);
});
test('actual fetch timeout terminates inside one bounded deadline',async()=>{
 const server=createServer(()=>{});await new Promise(r=>server.listen(0,'127.0.0.1',r));
 try{const start=Date.now();await assert.rejects(getJson(`http://127.0.0.1:${server.address().port}`,'secret',60),e=>e.code==='upstream_transport');assert.ok(Date.now()-start<600);}
 finally{server.closeAllConnections();await new Promise(r=>server.close(r));}
});
test('actual upstream identity mismatch is not retried or called terminal',async()=>{
 let calls=0;const dir=await mkdtemp(path.join(os.tmpdir(),'device-remote-'));await writeFile(path.join(dir,'token'),'test-secret');
 const server=createServer((req,res)=>{calls++;res.end(JSON.stringify({execution:{id:'other',status:'done'}}));});await new Promise(r=>server.listen(0,'127.0.0.1',r));
 try{const api=remote({server:{endpoint:`http://127.0.0.1:${server.address().port}`,token_file:path.join(dir,'token')}});await assert.rejects(api.dag('expected'),e=>e.code==='upstream_identity');assert.equal(calls,1);}
 finally{server.closeAllConnections();await new Promise(r=>server.close(r));await rm(dir,{recursive:true,force:true});}
});
