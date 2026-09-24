import {lstat,readFile,realpath} from 'node:fs/promises';
import {resolve,sep} from 'node:path';
import {digest,fail} from '../state.mjs';

const FILES={owner:'owner.json',spec:'case-spec.json',result:'result.json',
 capture:'network/manifest.json',recovery:'recovery.json'};
const hash=value=>typeof value==='string'&&/^[a-f0-9]{64}$/.test(value);

async function protectedFile(folder,relative,expected){
 if(!hash(expected)||!relative||relative.split('/').some(part=>!part||part==='.'||part==='..'))fail('Invalid UI evidence path or hash',400);
 const file=resolve(folder,relative);
 if(!file.startsWith(folder+sep))fail('UI evidence escaped run root',400);
 const info=await lstat(file);
 if(!info.isFile()||info.isSymbolicLink()||(await realpath(file))!==file)fail('UI evidence is not a regular owned file');
 const bytes=await readFile(file);
 if(digest(bytes)!==expected)fail('UI evidence changed: '+relative);
 return bytes;
}

export function assessUiEvidence(values,identity,operations){
 const {owner,spec,result,capture,recovery}=values;
 if(!result||!Array.isArray(result.screenshots)||!Array.isArray(result.network_files))fail('UI result evidence manifest missing');
 if(Object.entries(identity).some(([key,value])=>owner?.[key]!==value))fail('UI owner receipt mismatch');
 if(spec?.case_key!==identity.case_id||result?.case_id!==identity.case_id||result?.ui_run_id!==identity.ui_run_id)fail('UI case evidence belongs to another work');
 const expected=spec?.explicit_semantic_review?.execution_assertions;
 const assertions=result?.assertions;
 if(!Array.isArray(expected)||!expected.length||!Array.isArray(assertions)||expected.length!==assertions.length)fail('Original UI assertions are incomplete');
 const ids=expected.map(row=>row.id),actual=assertions.map(row=>row.id??row.name);
 if(new Set(ids).size!==ids.length||new Set(actual).size!==actual.length||ids.some(id=>!actual.includes(id)))fail('Original UI assertion identity mismatch');
 if(assertions.some(row=>!['passed','failed','not_applicable'].includes(row.status)||typeof row.detail!=='string'))fail('Invalid UI assertion outcome');
 const evidence=new Set([...(result.screenshots??[]),...(result.network_files??[])].map(row=>row.file));
 if(assertions.some(row=>row.status!=='not_applicable'&&(!Array.isArray(row.evidence)||!row.evidence.length||row.evidence.some(file=>!evidence.has(file)))))fail('UI assertion evidence is incomplete');
 if(!['passed','failed','inconclusive'].includes(result.status)||result.status==='passed'&&(assertions.some(row=>row.status==='failed')||!assertions.some(row=>row.status==='passed'))||result.status==='failed'&&!assertions.some(row=>row.status==='failed'))fail('UI case verdict contradicts assertions');
 if(capture?.case_key!==identity.case_id||capture?.worker!==identity.machine||!['stopped','stop_failed','start_failed'].includes(capture.state)||capture.cleanup_verified!==true)fail('UI capture cleanup is unverified');
 const recorded=Object.values(operations??{});
 const start=recorded.filter(op=>op.action==='capture-start'&&op.status==='done');
 const stop=recorded.filter(op=>op.action==='capture-stop'&&op.status==='done');
 if(start.length!==1||stop.length!==1||!start[0].result?.capture_id||
    start[0].result.capture_id!==stop[0].result?.capture_id||
    capture.capture_id!==start[0].result.capture_id)fail('UI capture does not match registered operations');
 const shots=recorded.filter(op=>op.action==='screenshot'&&op.status==='done').map(op=>op.result);
 if(result.screenshots.some(row=>!shots.some(shot=>shot?.file===row.file&&shot.sha256===row.sha256)))fail('UI screenshots lack registered capture operations');
 if(capture.state==='stopped'&&(capture.integrity?.cdp?.complete_jsonl!==true||
    capture.cdp_stop?.cleanup?.collectorAlive!==false||
    capture.https_enabled===true&&capture.https_stop?.cleanup?.restored!==true||
    capture.native_enabled===true&&(capture.native_stop?.cleanup?.collectorAlive!==false||
      capture.native_stop?.cleanup?.drivers?.length!==0)))fail('UI capture integrity or collector cleanup is incomplete');
 if(recovery?.ui_run_id!==identity.ui_run_id||recovery?.machine!==identity.machine||recovery?.application_restored!==true||recovery?.task_removed!==true||recovery?.proxy_restored!==true||recovery?.firewall_removed!==true||recovery?.collector_alive!==false)fail('UI application or capture environment is not restored');
 const uncertain=Object.entries(operations??{}).filter(([,op])=>op.status!=='done').map(([id])=>id);
 if(uncertain.length)fail('Uncertain UI operations require manager reconciliation before release');
 const complete=capture.state==='stopped'&&capture.coverage?.full_app_capture===true&&result.screenshots?.length>0&&result.network_files?.length>0;
 return {verified:true,evidence_complete:complete,case_status:complete?result.status:'inconclusive'};
}

export async function verifyUiReceipts(config,reservation,assignment,work,hashes){
 if(!hashes||typeof hashes!=='object'||Array.isArray(hashes)||Object.keys(hashes).some(key=>!FILES[key])||Object.keys(FILES).some(key=>!hash(hashes[key])))fail('UI receipt hashes are incomplete',400);
 const base=await realpath(config.evidence_root),folder=resolve(base,'ui',work.ui_run_id);
 if((await realpath(folder))!==folder||!folder.startsWith(base+sep))fail('UI evidence root is not owned');
 const raw={};for(const [key,path] of Object.entries(FILES))raw[key]=await protectedFile(folder,path,hashes[key]);
 if(digest(raw.spec)!==work.spec_sha256)fail('Frozen UI specification changed');
 const values={};for(const [key,bytes] of Object.entries(raw))values[key]=JSON.parse(bytes.toString('utf8'));
 const identity={reservation_id:reservation.reservation_id,dag_id:reservation.dag_id,
  instance_id:assignment.instance_id,session_id:work.session_id,generation:work.generation,
  machine:assignment.machine,ui_run_id:work.ui_run_id,case_id:work.case_id,
  spec_sha256:work.spec_sha256,input_version:work.input_version};
 const proof=assessUiEvidence(values,identity,work.operations);
 const screenshots=values.result.screenshots??[],network=values.result.network_files??[];
 if(!Array.isArray(screenshots)||screenshots.length>50||screenshots.some(row=>!row||!/^screenshots\/[A-Za-z0-9._-]+\.(png|jpg|jpeg)$/.test(row.file)||!hash(row.sha256)))fail('Invalid UI screenshot manifest');
 if(!Array.isArray(network)||network.length>1000||network.some(row=>!row||!/^network\/private\/[A-Za-z0-9._/-]+$/.test(row.file)||!hash(row.sha256)))fail('Invalid UI network manifest');
 if(new Set([...screenshots,...network].map(row=>row.file)).size!==screenshots.length+network.length)fail('Duplicate UI evidence file');
 const captured=values.capture.files;
 if(values.capture.state==='stopped'&&(!Array.isArray(captured)||!captured.length))fail('Capture manifest omitted raw files');
 if(Array.isArray(captured)){
  const declared=captured.map(row=>({file:'network/'+row.path,sha256:row.sha256}));
  if(declared.some(row=>!row.file.startsWith('network/private/')||!hash(row.sha256))||
   JSON.stringify(declared.sort((a,b)=>a.file.localeCompare(b.file)))!==JSON.stringify([...network].sort((a,b)=>a.file.localeCompare(b.file))))fail('UI raw capture files differ from collector manifest');
 }
 for(const row of [...screenshots,...network])await protectedFile(folder,row.file,row.sha256);
 return proof;
}
