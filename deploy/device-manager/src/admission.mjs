// Pure host-maintenance transition. Guest and file evidence is verified by the caller.
import {fail} from './state.mjs';
export function admitVerified(state,proofs,now=Date.now()){
 const names=new Set();
 for(const p of proofs){
  if(!/^win-\d{2}$/.test(p.machine)||names.has(p.machine))fail('Invalid or duplicate admission machine');
  names.add(p.machine);
  if(!Number.isFinite(Date.parse(p.at))||now-Date.parse(p.at)>900000||Date.parse(p.at)>now+30000)fail('Admission proof is stale');
  if(p.logged_in!==true||p.home_verified!==true||p.idle_verified!==true||p.product_verified!==true||p.legacy_recovered!==true)fail('Incomplete admission proof');
  if(state.devices[p.machine]?.reservation_id)fail('Admission target is currently reserved');
 }
 const next=structuredClone(state);
 for(const p of proofs){
  const device=next.devices[p.machine]??={machine:p.machine,generation:0,reservation_id:null};
  device.quarantine=null;device.handoff=structuredClone(p);
 }
 return next;
}
