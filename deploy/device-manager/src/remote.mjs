import {readFile} from 'node:fs/promises';
const transientStatus=new Set([502,503,504]);
const transientCodes=new Set(['ECONNRESET','ECONNREFUSED','ETIMEDOUT','EPIPE','EAI_AGAIN','UND_ERR_SOCKET','UND_ERR_CONNECT_TIMEOUT']);
function queryError(code, details={}) {
 return Object.assign(Error('Server query failed'),{code,...details});
}
export async function getJson(url,token,budget,deps={}) {
 const now=deps.now??Date.now, fetcher=deps.fetch??fetch;
 const sleep=deps.sleep??(ms=>new Promise(resolve=>setTimeout(resolve,ms)));
 const deadline=now()+budget;
 let last=queryError('upstream_timeout'),attempts=0;
 while(now()<deadline && attempts<8){
  attempts++;
  try{
   const response=await fetcher(url,{headers:{Authorization:'Bearer '+token},
    signal:AbortSignal.timeout(Math.max(1,Math.floor(deadline-now()))),redirect:'error'});
   if(!response.ok){
    await response.body?.cancel();
    const error=queryError('upstream_http',{upstream_status:response.status});
    if(!transientStatus.has(response.status))throw Object.assign(error,{permanent:true});
    throw error;
   }
   let value;
   try{value=await response.json();}catch{throw Object.assign(queryError('upstream_content'),{permanent:true});}
   if(now()>=deadline)throw queryError('upstream_timeout');
   return value;
  }catch(error){
   const cause=error.cause?.code??error.code;
   if(error.permanent)throw Object.assign(error,{attempts});
   if(error.code==='upstream_http'||error.code==='upstream_timeout')last=error;
   else if(transientCodes.has(cause)||['TimeoutError','AbortError'].includes(error.name))
    last=queryError('upstream_transport',{cause_code:transientCodes.has(cause)?cause:'timeout'});
   else throw Object.assign(queryError('upstream_query'),{attempts});
  }
  if(now()<deadline && attempts<8)await sleep(Math.min(250*2**(attempts-1),2000,deadline-now()));
 }
 throw Object.assign(last,{attempts});
}
export function remote(config){
 async function get(route){
  const token=(await readFile(config.server.token_file,'utf8')).trim();
  return getJson(config.server.endpoint+route,token,config.server.timeout_ms??10000);
 }
 return {async dag(id){const value=await get('/api/executions/'+encodeURIComponent(id));if(value.execution?.id!==id)throw queryError('upstream_identity');return value.execution.status;},
 async session(dag,step,index,id){
  const value=await get('/api/dag/runs/'+encodeURIComponent(dag)+'/steps/'+encodeURIComponent(step)+'/instances/'+encodeURIComponent(index));
  if(value.run_id!==dag||value.name!==step||String(value.index)!==String(index)||value.session_id!==id)throw queryError('upstream_identity');
  return value.status;
 }};
}
