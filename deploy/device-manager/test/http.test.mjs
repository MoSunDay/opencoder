import test from 'node:test';
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {mkdtemp,writeFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {start} from '../src/server.mjs';
async function listen(server){await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));return server.address().port;}
test('HTTP auth scopes, no secret in public output, and real remote status format',async t=>{
 const dir=await mkdtemp(join(tmpdir(),'device-http-'));t.after(()=>rm(dir,{recursive:true,force:true}));
 const admin='a'.repeat(40),serverToken='b'.repeat(40);
 await writeFile(join(dir,'admin'),admin,{mode:0o600});await writeFile(join(dir,'server'),serverToken,{mode:0o600});
 const upstream=createServer((req,res)=>{assert.equal(req.headers.authorization,'Bearer '+serverToken);res.setHeader('content-type','application/json');res.end(JSON.stringify({execution:{id:'dag-http',status:'running'}}));});
 const upstreamPort=await listen(upstream);t.after(()=>new Promise(resolve=>upstream.close(resolve)));
 const probe=createServer();const port=await listen(probe);await new Promise(resolve=>probe.close(resolve));
 const live=await start({listen_host:'127.0.0.1',port,token_file:join(dir,'admin'),state_file:join(dir,'state'),evidence_root:dir,poll_ms:60000,server:{endpoint:`http://127.0.0.1:${upstreamPort}`,token_file:join(dir,'server')}});
 t.after(()=>live.close());await live.store.transaction(s=>{s.devices['win-19'].quarantine=null;});
 const endpoint=`http://127.0.0.1:${port}`;
 const call=(path,token,body)=>fetch(endpoint+path,{method:body?'POST':'GET',headers:token?{Authorization:'Bearer '+token,'content-type':'application/json'}:{},body:body?JSON.stringify(body):undefined});
 assert.equal((await call('/v1/devices')).status,401);
 const malformed=await fetch(endpoint+'/v1/step-sessions',{method:'POST',headers:{Authorization:'Bearer invalid','content-type':'application/json'},body:'{'});
 assert.equal(malformed.status,401);
 const allocatorResponse=await call('/v1/step-sessions',admin,{dag_id:'dag-http',step_id:'allocate',session_id:'allocator',role:'allocate',target_step:'execute',count:1});assert.equal(allocatorResponse.status,200);const allocator=await allocatorResponse.json();
 const response=await call('/v1/reservations',allocator.capability,{dag_id:'dag-http',target_step:'execute',count:1});assert.equal(response.status,200);const reservation=await response.json();assert.equal(reservation.assignments[0].machine,'win-19');
 assert.equal((await call('/v1/devices',allocator.capability)).status,401);
 const text=await (await call(`/v1/reservations/${reservation.reservation_id}/status`,allocator.capability)).text();assert(!text.includes(admin));assert(!text.includes(allocator.capability));assert(!text.includes('capability'));
});
