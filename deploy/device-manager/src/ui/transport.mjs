// Authenticated NodeAPI transport; guest credentials never enter task context.
import {createHash,createHmac,randomBytes} from 'node:crypto';
import {execFile} from 'node:child_process';
import {mkdir,readFile,writeFile,lstat,realpath} from 'node:fs/promises';
import {dirname,join} from 'node:path';
import {promisify} from 'node:util';
import {fail} from '../state.mjs';
const runFile=promisify(execFile);

async function workerContext(config,machine){
 if(!config.ui_workers_file)fail('UI worker inventory unavailable',503);
 const inventory=JSON.parse(await readFile(config.ui_workers_file,'utf8'));
 const rows=inventory.workers;
 const matches=Array.isArray(rows)?rows.filter(row=>row.name===machine&&row.platform==='windows'):[];
 if(matches.length!==1)fail('UI worker identity unavailable',503);
 const worker=matches[0];
 const base=new URL(worker.node_base);
 if(!['http:','https:'].includes(base.protocol)||base.username||base.password||base.search||base.hash||base.pathname!=='/')fail('Invalid UI worker endpoint',503);
 if(!/^[A-Za-z0-9._-]{1,100}$/.test(worker.session_id??''))fail('Invalid UI worker session',503);
 const secret=(await readFile(worker.token_file,'utf8')).trim();
 if(!/^[a-f0-9]{64}$/i.test(secret))fail('UI worker credential unavailable',503);
 return {base,session_id:worker.session_id,secret,worker};
}

function signedHeaders(secret,method,path,body){
 const timestamp=new Date().toISOString().replace(/\.\d{3}Z$/,'Z');
 const nonce=randomBytes(16).toString('hex');
 const hash=createHash('sha256').update(body).digest('hex');
 const signature=createHmac('sha256',secret).update([timestamp,nonce,method,path,hash].join('\n')).digest('hex');
 return {'x-viking-auth-timestamp':timestamp,'x-viking-auth-nonce':nonce,
  'x-viking-auth-signature':signature};
}

export async function uiAction(config,machine,input){
 const {base,session_id,secret}=await workerContext(config,machine);
 const path=`/v1/sessions/${session_id}/actions`;
 const body=Buffer.from(JSON.stringify({action:input.action,arguments:input.arguments,
  request_id:input.operation_id,capture_before:false,capture_after:false}));
 const response=await fetch(new URL(path,base),{method:'POST',body,redirect:'error',
  signal:AbortSignal.timeout(120000),headers:{'content-type':'application/json',
   ...signedHeaders(secret,'POST',path,body)}});
 const value=await response.text();
 if(value.length>131072)fail('UI action response exceeds evidence limit',503);
 if(response.status!==200)fail('UI worker rejected scoped action',503);
 let result;
 try{result=JSON.parse(value);}catch{fail('UI worker returned invalid action result',503);}
 if(!result||typeof result!=='object'||result.error||!result.result||typeof result.result!=='object'||
    result.request_id!==undefined&&result.request_id!==input.operation_id)fail('UI worker did not confirm the registered action',503);
 return result;
}

export async function uiScreenshot(config,machine,input){
 const {base,session_id,secret}=await workerContext(config,machine);
 const path=`/v1/sessions/${session_id}/screenshot`;
 const response=await fetch(new URL(path,base),{method:'GET',redirect:'error',
  signal:AbortSignal.timeout(90000),headers:signedHeaders(secret,'GET',path,Buffer.alloc(0))});
 if(response.status!==200||!response.body)fail('UI worker screenshot failed',503);
 const chunks=[];let total=0;
 for await(const chunk of response.body){total+=chunk.length;if(total>8*1024*1024)fail('UI screenshot exceeds evidence limit',503);chunks.push(chunk);}
 const bytes=Buffer.concat(chunks);
 if(bytes.length<8||!bytes.subarray(0,8).equals(Buffer.from([137,80,78,71,13,10,26,10])))fail('UI worker returned invalid PNG evidence',503);
 const folder=join(config.evidence_root,'ui',input.ui_run_id,'screenshots');
 await mkdir(folder,{recursive:true,mode:0o700});
 const file=`screenshots/${input.operation_id}.png`;
 await writeFile(join(folder,`${input.operation_id}.png`),bytes,{flag:'wx',mode:0o600});
 return {file,sha256:createHash('sha256').update(bytes).digest('hex'),bytes:bytes.length};
}

export async function uiCapture(config,machine,input){
 const {base,session_id,worker}=await workerContext(config,machine);
 const cli=join(dirname(config.ui_workers_file),'driver-windows','cli.py');
 const output=join(config.evidence_root,'ui',input.ui_run_id,'network');
 const name=input.action==='capture-start'?'network_capture_start':'network_capture_stop';
 const args=[cli,name,`output=${output}`];
 if(name==='network_capture_start')args.push(`case_key=${input.case_id}`,'attempt=1','max_seconds=10800');
 const env={...process.env,FLEET_WORKER:machine,FLEET_NODE_BASE:base.origin,
  FLEET_NODE_TOKEN_FILE:worker.token_file,FLEET_SESSION_ID:session_id,
  FLEET_RUN_ROOT:join(config.evidence_root,'ui',input.ui_run_id)};
 let stdout;
 try{({stdout}=await runFile('python3',args,{env,timeout:name==='network_capture_stop'?1800000:360000,
  maxBuffer:131072,windowsHide:true}));}
 catch{fail('UI capture lifecycle failed; device remains reserved',503);}
 let result;
 try{result=JSON.parse(stdout);}catch{fail('UI capture returned invalid receipt',503);}
 if(result?.ok!==true)fail('UI capture did not confirm success',503);
 return {operation:name,capture_id:result.capture_id??null,state:result.state??null};
}

export async function recoverUiEvidence(config,machine,input,work){
 const operation=work.operations[input.operation_id];
 if(!operation||operation.status!=='uncertain'||operation.action!==input.action)fail('UI recovery operation identity mismatch',403);
 const root=join(config.evidence_root,'ui',input.ui_run_id);
 if(input.action==='screenshot'){
  const file=join(root,'screenshots',input.operation_id+'.png');
  const stat=await lstat(file);
  if(!stat.isFile()||stat.isSymbolicLink()||(await realpath(file))!==file||stat.size>8*1024*1024)fail('Uncertain UI screenshot is not owned evidence');
  const bytes=await readFile(file);
  if(bytes.length<8||!bytes.subarray(0,8).equals(Buffer.from([137,80,78,71,13,10,26,10])))fail('Uncertain UI screenshot is invalid');
  return {file:`screenshots/${input.operation_id}.png`,sha256:createHash('sha256').update(bytes).digest('hex'),bytes:bytes.length};
 }
 if(!['capture-start','capture-stop'].includes(input.action))fail('Business UI action has no safe replay proof');
 const file=join(root,'network','manifest.json');
 const stat=await lstat(file);
 if(!stat.isFile()||stat.isSymbolicLink()||(await realpath(file))!==file)fail('Uncertain UI capture manifest is not owned evidence');
 const manifest=JSON.parse(await readFile(file,'utf8'));
 if(manifest.case_key!==input.case_id||manifest.worker!==machine||!manifest.capture_id)fail('Uncertain UI capture identity mismatch');
 if(input.action==='capture-start'&&!['running','stopped','stop_failed'].includes(manifest.state))fail('Uncertain UI capture did not start');
 if(input.action==='capture-stop'&&(!['stopped','stop_failed'].includes(manifest.state)||manifest.cleanup_verified!==true||
    !Object.values(work.operations).some(op=>op.action==='capture-start'&&op.status==='done'&&op.result?.capture_id===manifest.capture_id)))fail('Uncertain UI capture did not confirm cleanup');
 return {operation:input.action==='capture-start'?'network_capture_start':'network_capture_stop',capture_id:manifest.capture_id,state:manifest.state};
}
