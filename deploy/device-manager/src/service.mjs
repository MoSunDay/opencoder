import {randomBytes,timingSafeEqual} from 'node:crypto';
import {lstat} from 'node:fs/promises';
import {dirname,join} from 'node:path';
import {allocate,assignment,claim,scoped,startWork,completeWork,reclaim,publicValue,terminal,fail,digest} from './state.mjs';
import {verifyReceipts} from './receipts.mjs';
import {verifyUiReceipts} from './ui/receipts.mjs';
import {startUiWork,uiWork,completeUiWork,beginUiAction,finishUiAction,reconcileUiAction} from './ui/work.mjs';
import {uiAction,uiScreenshot,uiCapture,recoverUiEvidence} from './ui/transport.mjs';
import {dagStatus,recordOutcome,recoveryOnly} from './lifecycle.mjs';
const equal=(a,b)=>typeof a==='string'&&typeof b==='string'&&a.length===b.length&&timingSafeEqual(Buffer.from(a),Buffer.from(b));
async function reconcileHarnessQuarantines(config,store){
 const root=join(dirname(config.evidence_root),'quarantine'),blocked=[];
 for(const machine of Object.keys(store.read().devices)){
  try{await lstat(join(root,`${machine}.json`));blocked.push(machine);}
  catch(error){if(error.code!=='ENOENT')throw error;}
 }
 const devices=store.read().devices;
 if(blocked.some(machine=>!devices[machine].quarantine))await store.transaction(state=>{
  for(const machine of blocked)state.devices[machine].quarantine??='native_harness_quarantine';
 });
}
export function service(config,store,upstream,adminToken){
 const admin=token=>{if(!equal(token,adminToken))fail('Unauthorized',401);};
 const authorized=token=>equal(token,adminToken)||Object.values(store.read().allocators??{}).some(a=>equal(a.capability,token))||Object.values(store.read().reservations??{}).some(r=>r.assignments.some(a=>equal(a.capability,token)));
 async function observeDag(id){
  const observed=await upstream.dag(id);
  const status=dagStatus(store.read(),id,observed);
  if(terminal(status))await store.transaction(state=>{Object.assign(state,recordOutcome(state,id,status));});
  return status;
 }
 const activeAllocator=(state,scope)=>{
  const session=state.sessions?.[scope.session_id];
  if(!session||terminal(session.outcome))fail('Allocator session ended',403);
 };
 async function handle(method,path,input,token){
  const finish=path.match(/^\/v1\/step-sessions\/([^/]+)\/finish$/);
  if(method==='POST'&&finish){
   admin(token);const session=decodeURIComponent(finish[1]);
   if(!terminal(input.outcome))fail('Session outcome must be terminal',400);
   return store.transaction(state=>{
    const known=state.sessions?.[session];
    if(!known||['dag_id','step_id','instance_id'].some(k=>known[k]!==input[k]))fail('Host session identity mismatch');
    if(known.outcome&&known.outcome!==input.outcome)fail('Host completion changed');
    known.outcome=input.outcome;known.finished_at??=new Date().toISOString();return known;
   });
  }
  if(method==='POST'&&path==='/v1/step-sessions'){
   admin(token);
   const {dag_id,step_id,session_id,target_step,count,role}=input,work_type=input.work_type??'native';
   if(role!=='allocate'||!step_id||!session_id||!target_step||!Number.isInteger(count)||count<1||count>Object.keys(store.read().devices).length)fail('Invalid allocator session',400);
   if(!['native','ui'].includes(work_type)||work_type==='ui'&&(!Array.isArray(input.case_ids)||input.case_ids.length<count||new Set(input.case_ids).size!==input.case_ids.length||input.case_ids.some(id=>typeof id!=='string'||!/^[A-Za-z0-9._-]{1,100}$/.test(id))))fail('Invalid UI allocator scope',400);
   if(work_type==='ui'&&(!Array.isArray(input.case_specs)||input.case_specs.length!==input.case_ids.length||input.case_specs.some((row,i)=>row?.case_id!==input.case_ids[i]||!/^[a-f0-9]{64}$/.test(row.spec_sha256??'')||!/^[A-Za-z0-9._-]{1,100}$/.test(row.input_version??''))))fail('Invalid frozen UI specifications',400);
   const status=await observeDag(dag_id);
   return store.transaction(state=>{
    if(terminal(dagStatus(state,dag_id,status)))fail('DAG already terminal');
    state.sessions??={};const known=state.sessions[session_id];
    if(known?.outcome)fail('Allocator session already ended');
    if(known&&(known.dag_id!==dag_id||known.step_id!==step_id||known.instance_id!==null))fail('Host session already registered');
    state.sessions[session_id]??={dag_id,step_id,instance_id:null,session_id};
    state.allocators??={};const key=digest(JSON.stringify([dag_id,step_id,session_id]));
    const old=state.allocators[key];if(old){if(old.target_step!==target_step||old.count!==count||(old.work_type??'native')!==work_type||JSON.stringify(old.case_ids??[])!==JSON.stringify(input.case_ids??[])||JSON.stringify(old.case_specs??[])!==JSON.stringify(input.case_specs??[]))fail('Allocator scope changed');return old;}
    return state.allocators[key]={dag_id,step_id,session_id,target_step,count,role,work_type,case_ids:work_type==='ui'?input.case_ids:undefined,case_specs:work_type==='ui'?input.case_specs:undefined,capability:randomBytes(32).toString('hex')};
   });
  }
  if(method==='POST'&&path==='/v1/reservations'){
   const scope=Object.values(store.read().allocators??{}).find(a=>equal(a.capability,token));
   if(!scope||scope.dag_id!==input.dag_id||scope.target_step!==input.target_step||scope.count!==input.count||(scope.work_type??'native')!==(input.work_type??'native')||JSON.stringify(scope.case_ids??[])!==JSON.stringify(input.case_ids??[])||JSON.stringify(scope.case_specs??[])!==JSON.stringify(input.case_specs??[]))fail('Allocator capability mismatch',403);
   activeAllocator(store.read(),scope);
   await reconcileHarnessQuarantines(config,store);
   const status=await observeDag(input.dag_id);
   return publicValue(await store.transaction(state=>{activeAllocator(state,scope);return allocate(state,input,status);}));
  }
  if(method==='GET'&&path==='/v1/devices'){admin(token);return Object.values(store.read().devices);}
  const match=path.match(/^\/v1\/reservations\/([a-f0-9]{32})(?:\/(claims|work|ui-work|ui-actions|ui-screenshots|ui-capture|ui-reconcile|ui-completions|completions|status))?$/);
  if(!match)fail('Not found',404);const [,id,action]=match;
  if(method==='GET'&&(!action||action==='status')){
   const r=store.read().reservations[id];if(!r)fail('Not found',404);
   if(!equal(token,adminToken)&&!r.assignments.some(a=>equal(a.capability,token))&&!Object.values(store.read().allocators??{}).some(a=>equal(a.capability,token)&&a.dag_id===r.dag_id&&a.target_step===r.target_step))fail('Unauthorized',403);
   return publicValue(r);
  }
  if(method!=='POST')fail('Method not allowed',405);
  if(action==='claims'){
   admin(token);const [r,a]=assignment(store.read(),id,input);
   const dag=await observeDag(r.dag_id);
   let oldTerminal=false;
   if(a.session_id&&a.session_id!==input.session_id){
    const recorded=store.read().sessions?.[a.session_id];
    oldTerminal=terminal(recorded?.outcome);
    if(!oldTerminal)oldTerminal=terminal(await upstream.session(r.dag_id,r.target_step,a.instance_id,a.session_id));
   }
   return store.transaction(state=>{
    const [,current]=assignment(state,id,input),status=dagStatus(state,r.dag_id,dag);
    if(terminal(status)&&!current.works.some(w=>!w.recovery_verified))fail('Terminal DAG has no work requiring recovery');
    state.sessions??={};const known=state.sessions[input.session_id];
    if(known&&(known.dag_id!==r.dag_id||known.step_id!==r.target_step||known.instance_id!==input.instance_id||known.outcome))fail('Host session identity changed or already ended');
    const result=claim(state,id,input,oldTerminal);
    result.recovery_only=recoveryOnly(status,result);
    state.sessions[input.session_id]??={dag_id:r.dag_id,step_id:r.target_step,instance_id:input.instance_id,session_id:input.session_id};
    return result;
   });
  }
  if(action==='work'){
   const [r]=scoped(store.read(),id,input,token),observed=await observeDag(r.dag_id);
   return store.transaction(async state=>{
    const [current,a]=scoped(state,id,input,token),status=dagStatus(state,r.dag_id,observed);
    if(terminal(status)&&!a.works.some(w=>w.native_run_id===input.native_run_id))fail('DAG is terminal; new work cannot start');
    if(!a.works.some(w=>w.native_run_id===input.native_run_id)){
     for(const w of a.works)if(w.recovery_verified)await verifyReceipts(config,current,a,w,w.receipts);
    }
    return {...startWork(state,id,input,token),recovery_only:recoveryOnly(status,a)};
   });
  }
  if(action==='ui-work'){
   const [r]=scoped(store.read(),id,input,token),observed=await observeDag(r.dag_id);
   return store.transaction(async state=>{
    const [current,a]=scoped(state,id,input,token),status=dagStatus(state,r.dag_id,observed);
    if(terminal(status)&&!a.works.some(w=>w.ui_run_id===input.ui_run_id))fail('DAG is terminal; new UI work cannot start');
    if(!a.works.some(w=>w.ui_run_id===input.ui_run_id)){
     for(const w of a.works)if(w.recovery_verified)await verifyUiReceipts(config,current,a,w,w.receipts);
    }
    return {...startUiWork(state,id,input,token),recovery_only:recoveryOnly(status,a)};
   });
  }
  if(action==='ui-reconcile'){
   const [,a,w]=uiWork(store.read(),id,input,token);
   const value=await recoverUiEvidence(config,a.machine,input,w);
   return store.transaction(state=>reconcileUiAction(state,id,input,token,value));
  }
  if(['ui-actions','ui-screenshots','ui-capture'].includes(action)){
   const routeAction=action==='ui-screenshots'?'screenshot':action==='ui-capture'?'capture-':'standard';
   if(routeAction==='screenshot'&&input.action!=='screenshot'||routeAction==='capture-'&&!['capture-start','capture-stop'].includes(input.action)||routeAction==='standard'&&['screenshot','capture-start','capture-stop'].includes(input.action))fail('UI operation route mismatch',400);
   const [r]=uiWork(store.read(),id,input,token),observed=await observeDag(r.dag_id);
   if(terminal(observed)&&input.action!=='capture-stop')fail('Terminal DAG cannot perform new UI actions');
   const begun=await store.transaction(state=>beginUiAction(state,id,input,token));
   if(!begun.execute){
    if(begun.operation.status==='done')return begun.operation;
    fail('UI action outcome pending or uncertain; inspect before recovery');
   }
   let value;
   try{value=action==='ui-screenshots'?await uiScreenshot(config,begun.machine,input):
    action==='ui-capture'?await uiCapture(config,begun.machine,input):await uiAction(config,begun.machine,input);}
   catch(error){
    await store.transaction(state=>finishUiAction(state,id,input,token,{status:'uncertain',value:{error:error.message}}));
    throw error;
   }
   return store.transaction(state=>finishUiAction(state,id,input,token,{status:'done',value}));
  }
  if(action==='ui-completions')return store.transaction(async state=>{
   const [r,a,w]=uiWork(state,id,input,token);
   const proof=await verifyUiReceipts(config,r,a,w,input.receipts);
   return completeUiWork(state,id,input,token,proof);
  });
  if(action==='completions')return store.transaction(async state=>{
   const [r,a]=scoped(state,id,input,token),w=a.works.find(w=>w.native_run_id===input.native_run_id);if(!w)fail('Work was not registered');
   const proof=await verifyReceipts(config,r,a,w,input.receipts);return completeWork(state,id,input,token,proof);
  });
  fail('Not found',404);
 }
 async function poll(){
  await reconcileHarnessQuarantines(config,store);
  const results=[];
  for(const r of Object.values(store.read().reservations)){
   if(r.assignments.every(a=>a.released))continue;
   try{
    const status=await observeDag(r.dag_id);if(!terminal(status))continue;
    const sessions={};for(const a of r.assignments)if(!a.released&&a.session_id&&!a.works.length){
     const recorded=store.read().sessions?.[a.session_id];
     if(terminal(recorded?.outcome))sessions[a.session_id]=recorded.outcome;
     else try{sessions[a.session_id]=await upstream.session(r.dag_id,r.target_step,a.instance_id,a.session_id);}catch{sessions[a.session_id]='unknown';}
    }
    // Revalidate recorded receipts before returning capacity; changed/deleted evidence fails closed.
    await store.transaction(async state=>{
     const current=state.reservations[r.reservation_id];
     for(const a of current.assignments)for(const w of a.works)if(!a.released&&w.recovery_verified){
      try{if(w.kind==='ui')await verifyUiReceipts(config,current,a,w,w.receipts);
       else await verifyReceipts(config,current,a,w,w.receipts);}
      catch{w.recovery_verified=false;w.recovery_error='Recorded evidence no longer verifies';}
     }
     results.push({reservation_id:r.reservation_id,released:reclaim(state,r.reservation_id,status,sessions)});
    });
   }catch(error){results.push({reservation_id:r.reservation_id,released:[],error:error.message});}
  }
  return results;
 }
 return {handle,poll,authorized};
}
