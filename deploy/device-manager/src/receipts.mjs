import {readFile,realpath,lstat} from 'node:fs/promises';
import {resolve,sep} from 'node:path';
import {digest,fail} from './state.mjs';
export const FILES={owner:'device-owner.json',run:'run.json',cleanup:'cleanup.json',release:'release.json',capture:'capture.json',manifest:'network/manifest.json',proxy:'delivery/evidence/capture-proxy-restored.json',firewall:'capture-firewall-restored.json',restored:'delivery/evidence/restored.json'};
export async function verifyReceipts(config,reservation,allocation,work,hashes){
 if(!hashes||typeof hashes!=='object'||Array.isArray(hashes))fail('Receipt hashes required',400);
 const base=await realpath(config.evidence_root),folder=resolve(base,work.native_run_id);
 if((await realpath(folder))!==folder||!folder.startsWith(base+sep))fail('Receipt directory escaped root');
 const values={};
 for(const name of Object.keys(hashes)){
  if(!FILES[name]||!/^[a-f0-9]{64}$/.test(hashes[name]))fail('Unknown receipt or invalid hash',400);
  const path=resolve(folder,FILES[name]);if((await lstat(path)).isSymbolicLink()||(await realpath(path))!==path)fail('Receipt symlink rejected');
  const bytes=await readFile(path);if(digest(bytes)!==hashes[name])fail('Receipt changed: '+name);
  values[name]=JSON.parse(bytes.toString('utf8').replace(/^\uFEFF/,''));
 }
 const expected={reservation_id:reservation.reservation_id,dag_id:reservation.dag_id,instance_id:allocation.instance_id,session_id:work.session_id,machine:allocation.machine,generation:work.generation,native_run_id:work.native_run_id};
 if(!values.owner||Object.entries(expected).some(([k,v])=>values.owner[k]!==v))fail('Receipt owner mismatch');
 const c=values.cleanup,l=values.release,run=values.run;
 if(!c||c.restoration_verified!==true||!run)fail('Application restoration missing');
 if(run.run_id!==work.native_run_id||run.machine?.name!==allocation.machine)fail('Native run identity mismatch');
 if(!l||l.vm!==allocation.machine||l.owner!=='native-harness-'+work.native_run_id||l.case_key!==work.native_run_id||l.native_restoration_verified!==true)fail('Case restoration receipt owner mismatch');
 // Before any remote submission, trusted run/cleanup receipts can prove the app was untouched.
 if(run.task_start_attempted===false&&c.original_untouched===true){
  try{await lstat(resolve(folder,'capture.json'));}catch(error){if(error.code!=='ENOENT')throw error;
   try{await lstat(resolve(folder,'network/manifest.json'));fail('Interrupted capture requires verified cleanup');}catch(e){if(e.code!=='ENOENT')throw e;}
   try{await lstat(resolve(folder,'capture-firewall.json'));if(values.firewall?.removed!==true)fail('Firewall restoration missing');}catch(e){if(e.code!=='ENOENT')throw e;}
   return {verified:true,capture_failed:false};}
 }
 const capture=values.capture,manifest=values.manifest;
 if(!capture?.capture_id||capture.capture_id!==manifest?.capture_id||manifest.case_key!==work.native_run_id||manifest.worker!==allocation.machine||manifest.cleanup_verified!==true||!['stopped','stop_failed'].includes(manifest.state))fail('Capture cleanup/ownership is unverified');
 if(run.task_start_attempted===false&&c.original_untouched===true){
  const proxy=manifest.integrity?.https?.cleanup;
  if(values.firewall?.removed!==true||proxy?.restored!==true||proxy?.collectorAlive!==false
    ||!(proxy.active_marker_removed===true||proxy.active_marker_absent===true||proxy.noop_verified===true))fail('Capture-only proxy/firewall recovery incomplete');
  return {verified:true,capture_failed:manifest.state!=='stopped'};
 }
 if(c.task_removed!==true||values.restored?.restored!==true||values.proxy?.restored!==true||values.proxy?.active_marker_removed!==true||values.firewall?.removed!==true)fail('Task/application/proxy/firewall recovery incomplete');
 return {verified:true,capture_failed:manifest.state!=='stopped'};
}
