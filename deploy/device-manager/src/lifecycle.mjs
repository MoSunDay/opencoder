// Pure lifecycle decisions; service.mjs owns IO and commits transitions.
export const terminal=status=>['done','error','cancelled'].includes(status);
export const active=status=>['running','pending','queued','assigned'].includes(status);

export function dagStatus(state,id,observed){
 if(!active(observed)&&!terminal(observed))
  throw Object.assign(Error('Unknown DAG status; ownership retained'),{status:503});
 return Object.hasOwn(state.dag_outcomes??{},id)?state.dag_outcomes[id]:observed;
}

export function recordOutcome(state,id,observed){
 const outcome=dagStatus(state,id,observed);
 if(!terminal(outcome))return state;
 return {...state,dag_outcomes:{...state.dag_outcomes,[id]:outcome}};
}

export const recoveryOnly=(status,allocation)=>terminal(status)||allocation.works.some(work=>
 !work.recovery_verified&&(work.session_id!==allocation.session_id||work.generation!==allocation.generation));
