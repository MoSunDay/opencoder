// UI work identity is distinct from native-harness runs.
import {scoped,fail,digest} from '../state.mjs';

const id=value=>typeof value==='string'&&/^[A-Za-z0-9._-]{1,128}$/.test(value);
const sha=value=>typeof value==='string'&&/^[a-f0-9]{64}$/.test(value);

export function startUiWork(state,reservation,input,capability){
 const [r,a]=scoped(state,reservation,input,capability);
 if(r.work_type!=='ui')fail('UI work requires a UI reservation',403);
 const expected='ui-'+digest(JSON.stringify([reservation,a.instance_id,input.case_id])).slice(0,40);
 const frozen=r.case_specs?.find(row=>row.case_id===input.case_id);
 if(input.ui_run_id!==expected||!id(input.case_id)||!sha(input.spec_sha256)||!id(input.input_version)||input.execution_id!==r.dag_id||!a.case_ids?.includes(input.case_id)||frozen?.spec_sha256!==input.spec_sha256||frozen?.input_version!==input.input_version)fail('UI work identity mismatch',400);
 const existing=a.works.find(w=>w.ui_run_id===input.ui_run_id);
 if(existing){
  if(['case_id','spec_sha256','input_version','execution_id'].some(key=>existing[key]!==input[key]))fail('UI work identity changed');
  if(existing.recovery_verified)fail('UI work already restored');
  return existing;
 }
 if(a.works.some(w=>!w.recovery_verified))fail('Previous work recovery is incomplete');
 if(Object.values(state.reservations).some(res=>res.assignments.some(row=>row.works.some(w=>w.ui_run_id===input.ui_run_id))))fail('UI run belongs to another assignment');
 const work={kind:'ui',ui_run_id:input.ui_run_id,execution_id:input.execution_id,
  case_id:input.case_id,spec_sha256:input.spec_sha256,input_version:input.input_version,
  session_id:a.session_id,generation:a.generation,started_at:new Date().toISOString(),
  recovery_verified:false,operations:{}};
 a.works.push(work);return work;
}

export function uiWork(state,reservation,input,capability){
 const [r,a]=scoped(state,reservation,input,capability);
 if(r.work_type!=='ui')fail('UI work requires a UI reservation',403);
 const work=a.works.find(w=>w.kind==='ui'&&w.ui_run_id===input.ui_run_id);
 if(!work||work.case_id!==input.case_id)fail('UI work ownership mismatch',403);
 return [r,a,work];
}

export function completeUiWork(state,reservation,input,capability,proof){
 const [,a,work]=uiWork(state,reservation,input,capability);
 if(work.recovery_verified){
  if(JSON.stringify(work.receipts)!==JSON.stringify(input.receipts))fail('UI completion proof changed');
  return work;
 }
 if(!proof.verified)fail('UI recovery evidence incomplete');
 Object.assign(work,{recovery_verified:true,receipts:input.receipts,
  case_status:proof.case_status,evidence_complete:proof.evidence_complete,
  completed_at:new Date().toISOString()});
 return work;
}

export function beginUiAction(state,reservation,input,capability){
 const [r,a,work]=uiWork(state,reservation,input,capability);
 if(work.recovery_verified)fail('Restored UI work cannot operate',403);
 if((work.session_id!==a.session_id||work.generation!==a.generation)&&input.action!=='capture-stop')fail('Rebound UI work requires recovery-only operations',403);
 if(!/^op-[A-Za-z0-9-]{1,100}$/.test(input.operation_id??'')
    ||typeof input.action!=='string'||!['list-windows','ui-snapshot','click','double-click','key','hotkey','type','bash','screenshot','capture-start','capture-stop'].includes(input.action)
    ||!input.arguments||typeof input.arguments!=='object'||Array.isArray(input.arguments))fail('Invalid UI action',400);
 const mutating=['click','double-click','key','hotkey','type','bash'];
 if(mutating.includes(input.action)&&!Object.values(work.operations).some(op=>op.action==='capture-start'&&op.status==='done'))fail('UI capture must start before business actions');
 if(input.action==='capture-stop'&&!Object.values(work.operations).some(op=>op.action==='capture-start'))fail('UI capture was never started');
 if(['capture-start','capture-stop'].includes(input.action)&&Object.entries(work.operations).some(([key,op])=>key!==input.operation_id&&op.action===input.action))fail('UI capture operation already registered');
 const request_hash=digest(JSON.stringify([input.action,input.arguments]));
 const previous=work.operations[input.operation_id];
 if(previous){
  if(previous.request_hash!==request_hash)fail('UI action identity changed');
  return {execute:false,operation:previous};
 }
 const operation={request_hash,action:input.action,status:'pending',started_at:new Date().toISOString()};
 work.operations[input.operation_id]=operation;
 return {execute:true,operation,machine:a.machine,execution_id:r.dag_id};
}

export function finishUiAction(state,reservation,input,capability,result){
 const [, ,work]=uiWork(state,reservation,input,capability);
 const operation=work.operations[input.operation_id];
 if(!operation||operation.status!=='pending')fail('UI action is not pending');
 Object.assign(operation,{status:result.status,result:result.value,finished_at:new Date().toISOString()});
 return operation;
}

export function reconcileUiAction(state,reservation,input,capability,value){
 const [, ,work]=uiWork(state,reservation,input,capability);
 const operation=work.operations[input.operation_id];
 if(!operation||operation.action!==input.action||operation.status!=='uncertain')fail('UI action cannot be reconciled');
 Object.assign(operation,{status:'done',result:value,reconciled_at:new Date().toISOString()});
 return operation;
}
