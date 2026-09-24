import {createHash, randomBytes} from 'node:crypto';
import {terminal,active,dagStatus} from './lifecycle.mjs';
export {terminal} from './lifecycle.mjs';
export const MACHINES=Array.from({length:18},(_,i)=>`win-${String(i+2).padStart(2,'0')}`);
export function fail(message,status=409){throw Object.assign(new Error(message),{status});}
export const digest=value=>createHash('sha256').update(value).digest('hex');
export function initial(){return {version:1,devices:Object.fromEntries(MACHINES.map(machine=>[machine,{machine,generation:0,reservation_id:null,quarantine:'awaiting_legacy_handoff'}])),reservations:{}};}
export function allocate(state,input,observed){
 const {dag_id,target_step,count}=input,work_type=input.work_type??'native';
 if(!/^[A-Za-z0-9._-]{1,128}$/.test(dag_id||'')||!/^[A-Za-z0-9._-]{1,128}$/.test(target_step||'')||!Number.isInteger(count)||count<1||count>Object.keys(state.devices).length)fail('Invalid allocation input',400);
 const id=digest(JSON.stringify([dag_id,target_step])).slice(0,32),old=state.reservations[id];
 if(!active(dagStatus(state,dag_id,observed)))fail('DAG is terminal; allocation cannot proceed');
 if(!['native','ui'].includes(work_type))fail('Invalid work type',400);
 if(work_type==='ui'&&(!Array.isArray(input.case_ids)||input.case_ids.length<count||new Set(input.case_ids).size!==input.case_ids.length||input.case_ids.some(id=>typeof id!=='string'||!/^[A-Za-z0-9._-]{1,100}$/.test(id))))fail('Invalid UI case ownership',400);
 if(work_type==='ui'&&(!Array.isArray(input.case_specs)||input.case_specs.length!==input.case_ids.length||input.case_specs.some((row,i)=>row?.case_id!==input.case_ids[i]||!/^[a-f0-9]{64}$/.test(row.spec_sha256??'')||!/^[A-Za-z0-9._-]{1,100}$/.test(row.input_version??''))))fail('Invalid UI specification ownership',400);
 if(old){if(old.count!==count||(old.work_type??'native')!==work_type||JSON.stringify(old.case_ids??[])!==JSON.stringify(input.case_ids??[])||JSON.stringify(old.case_specs??[])!==JSON.stringify(input.case_specs??[]))fail('Allocation input changed');return old;}
 const available=Object.values(state.devices).filter(d=>!d.reservation_id&&!d.quarantine);
 if(available.length<count)fail('Insufficient available devices');
 const assignments=available.slice(0,count).map((d,i)=>{d.generation++;d.reservation_id=id;return {instance_id:String(i),machine:d.machine,generation:d.generation,session_id:null,capability:null,case_ids:work_type==='ui'?input.case_ids.filter((_,j)=>j%count===i):undefined,works:[],released:false};});
 return state.reservations[id]={reservation_id:id,dag_id,target_step,count,work_type,case_ids:work_type==='ui'?input.case_ids:undefined,case_specs:work_type==='ui'?input.case_specs:undefined,assignments,created_at:new Date().toISOString()};
}
export function assignment(state,id,input){
 const r=state.reservations[id];if(!r)fail('Reservation not found',404);
 const a=r.assignments.find(a=>a.instance_id===input.instance_id);if(!a)fail('Instance not found',404);
 if(a.generation!==input.generation||a.released)fail('Stale ownership generation');
 const device=state.devices[a.machine];
 if(!device||device.reservation_id!==id||device.generation!==a.generation)fail('Device ownership does not match assignment');
 return [r,a];
}
export function claim(state,id,input,oldSessionTerminal=false){
 const [r,a]=assignment(state,id,input);
 if(typeof input.session_id!=='string'||!input.session_id||input.session_id.length>256)fail('Invalid session',400);
 for(const other of Object.values(state.reservations).flatMap(r=>r.assignments))if(other!==a&&!other.released&&other.session_id===input.session_id)fail('Session already owns another device');
 if(a.session_id&&a.session_id!==input.session_id){
  if(!oldSessionTerminal)fail('Previous session is not confirmed terminal');
  a.generation++;state.devices[a.machine].generation=a.generation;
 }
 if(a.session_id!==input.session_id){a.session_id=input.session_id;a.capability=randomBytes(32).toString('hex');}
 return {reservation_id:r.reservation_id,...a};
}
export function scoped(state,id,input,capability){
 const [r,a]=assignment(state,id,input);
 if(terminal(state.sessions?.[a.session_id]?.outcome)||!capability||a.capability!==capability||a.session_id!==input.session_id)fail('Capability does not own this assignment',403);
 return [r,a];
}
export function startWork(state,id,input,capability){
 const [r,a]=scoped(state,id,input,capability);
 if(r.work_type==='ui')fail('Native work cannot use a UI reservation',403);
 if(!/^nh-[a-zA-Z0-9-]{1,100}$/.test(input.native_run_id||''))fail('Invalid native run ID',400);
 const same=a.works.find(w=>w.native_run_id===input.native_run_id);if(same){if(same.recovery_verified)fail('Case already restored; no further device actions');return same;}
 if(a.works.some(w=>!w.recovery_verified))fail('Previous case recovery is incomplete');
 for(const other of Object.values(state.reservations).flatMap(r=>r.assignments).flatMap(a=>a.works))if(other.native_run_id===input.native_run_id)fail('Native run belongs to another assignment');
 const work={native_run_id:input.native_run_id,session_id:a.session_id,generation:a.generation,started_at:new Date().toISOString(),recovery_verified:false};a.works.push(work);return work;
}
export function completeWork(state,id,input,capability,proof){
 const [r,a]=scoped(state,id,input,capability),w=a.works.find(w=>w.native_run_id===input.native_run_id);
 if(r.work_type==='ui')fail('Native completion cannot use a UI reservation',403);
 if(!w)fail('Work was not registered');
 if(w.recovery_verified){if(JSON.stringify(Object.entries(w.receipts).sort())!==JSON.stringify(Object.entries(input.receipts).sort()))fail('Completion proof changed');return w;}
 if(!proof.verified)fail('Recovery evidence incomplete');
 Object.assign(w,{recovery_verified:true,receipts:input.receipts,capture_failed:proof.capture_failed,completed_at:new Date().toISOString()});return w;
}
export function reclaim(state,id,dagStatus,sessionStates={}){
 const r=state.reservations[id];if(!r||!terminal(dagStatus))return [];
 const released=[];
 for(const a of r.assignments){
  if(a.released)continue;
  const safe=a.works.length?a.works.every(w=>w.recovery_verified):(!a.session_id||terminal(sessionStates[a.session_id]));
  if(!safe)continue;
  const d=state.devices[a.machine];
  if(d.reservation_id!==id||d.generation!==a.generation)continue;
  d.reservation_id=null;a.released=true;a.capability=null;a.released_at=new Date().toISOString();released.push(a.machine);
 }
 return released;
}
export function publicValue(value){return JSON.parse(JSON.stringify(value,(k,v)=>k==='capability'?undefined:v));}
