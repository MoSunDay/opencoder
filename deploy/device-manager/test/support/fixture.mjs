import {mkdtemp,rm,mkdir,writeFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join,dirname} from 'node:path';
import {openStore} from '../../src/store.mjs';
import {service} from '../../src/service.mjs';
import {FILES} from '../../src/receipts.mjs';
import {digest} from '../../src/state.mjs';
export const ADMIN='host-private-token-not-for-model';
export async function fixture(t,count=3){
 const root=await mkdtemp(join(tmpdir(),'device-api-'));t.after(()=>rm(root,{recursive:true,force:true}));
 const file=join(root,'state.json'),store=await openStore(file),config={evidence_root:join(root,'runs')};await mkdir(config.evidence_root);
 await store.transaction(s=>{for(const d of Object.values(s.devices).slice(0,count))d.quarantine=null;});
 const remote={status:'running',sessionStatus:'running',async dag(){if(this.error)throw Error('query unavailable');return this.status;},async session(){if(this.sessionError)throw Error('old session unknown');return this.sessionStatus;}};
 const api=service(config,store,remote,ADMIN);
 async function allocate(dag='dag-one',number=1){
  const scope=await api.handle('POST','/v1/step-sessions',{dag_id:dag,step_id:'allocate',session_id:'allocator-'+dag,role:'allocate',target_step:'execute',count:number},ADMIN);
  return api.handle('POST','/v1/reservations',{dag_id:dag,target_step:'execute',count:number},scope.capability);
 }
 async function claim(r,index='0',session='session-'+index){const a=r.assignments.find(a=>a.instance_id===index);return api.handle('POST',`/v1/reservations/${r.reservation_id}/claims`,{instance_id:index,generation:a.generation,session_id:session},ADMIN);}
 const body=a=>({instance_id:a.instance_id,generation:a.generation,session_id:a.session_id});
 async function work(r,a,id='nh-case-one'){return api.handle('POST',`/v1/reservations/${r.reservation_id}/work`,{...body(a),native_run_id:id},a.capability);}
 async function proofs(r,a,id='nh-case-one'){
  const values={owner:{reservation_id:r.reservation_id,dag_id:r.dag_id,instance_id:a.instance_id,session_id:a.session_id,machine:a.machine,generation:a.generation,native_run_id:id},run:{run_id:id,machine:{name:a.machine},task_start_attempted:true},cleanup:{restoration_verified:true,task_removed:true},release:{vm:a.machine,owner:'native-harness-'+id,case_key:id,native_restoration_verified:true},capture:{capture_id:'capture-'+id},manifest:{capture_id:'capture-'+id,case_key:id,worker:a.machine,state:'stopped',cleanup_verified:true},proxy:{restored:true,active_marker_removed:true},firewall:{removed:true},restored:{restored:true}};
  const receipts={};for(const [name,value] of Object.entries(values)){const p=join(config.evidence_root,id,FILES[name]);await mkdir(dirname(p),{recursive:true});const bytes=JSON.stringify(value);await writeFile(p,bytes);receipts[name]=digest(bytes);}
  return receipts;
 }
 const complete=(r,a,id,receipts)=>api.handle('POST',`/v1/reservations/${r.reservation_id}/completions`,{...body(a),native_run_id:id,receipts},a.capability);
 return {root,file,store,config,remote,api,allocate,claim,work,proofs,complete,body};
}
