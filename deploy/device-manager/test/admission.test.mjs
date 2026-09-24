import test from 'node:test';
import assert from 'node:assert/strict';
import {initial} from '../src/state.mjs';
import {admitVerified} from '../src/admission.mjs';
const now=Date.now();
const proof=machine=>({machine,at:new Date(now).toISOString(),logged_in:true,home_verified:true,idle_verified:true,product_verified:true,legacy_recovered:true});
test('admission adds a logged-in device while preserving every existing reservation and ownership',()=>{
 const s=initial();s.devices['win-04'].reservation_id='owned';s.reservations.owned={assignments:[{machine:'win-04',generation:8,session_id:'active'}]};
 const before=structuredClone(s),next=admitVerified(s,[proof('win-20'),proof('win-02')],now);
 assert.deepEqual(s,before);assert.deepEqual(next.reservations,s.reservations);assert.deepEqual(next.devices['win-04'],s.devices['win-04']);
 assert.equal(next.devices['win-20'].quarantine,null);assert.equal(next.devices['win-02'].quarantine,null);
});
test('admission rejects missing login, stale observations, duplicate targets, and claimed devices atomically',()=>{
 const s=initial();s.devices['win-04'].reservation_id='owned';
 for(const input of [[{...proof('win-20'),logged_in:false}],[{...proof('win-20'),at:new Date(now-900001).toISOString()}],[proof('win-20'),proof('win-20')],[proof('win-20'),proof('win-04')]]){
  assert.throws(()=>admitVerified(s,input,now));assert.equal(s.devices['win-20'],undefined);
 }
});
