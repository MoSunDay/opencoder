const pretty = value => JSON.stringify(value,null,2);

export function reviewFiles(definition, snapshot, events) {
  const files = Object.fromEntries(Object.entries(definition || {}).map(([path,text]) => [`definition/${path}`,text]));
  const sessions = {}; const current = new Map();
  if (snapshot?.workflow) {
    files['process/workflow.json'] = pretty(snapshot.workflow);
    files['process/parent/session.json'] = pretty({session_id:snapshot.workflow.parent_session_id});
    sessions['process/parent/session.json'] = snapshot.workflow.parent_session_id;
  }
  for (const node of snapshot?.nodes || []) files[`process/todos/${node.id}/status.json`] = pretty(node);
  for (const event of [...events].sort((a,b)=>a.seq-b.seq)) {
    const payload=event.payload || {};
    const seq=String(event.seq).padStart(12,'0');
    for (const assignment of payload.assignments || []) {
      const base=`process/todos/${assignment.todo_id}/attempts/${seq}`;
      current.set(assignment.todo_id,base);
      files[`${base}/dispatch.json`]=pretty({...assignment,dispatch_seq:event.seq,world_epoch:payload.world_epoch});
      if (assignment.context) files[`${base}/context.json`]=pretty(assignment.context);
      if (assignment.session_id) {
        files[`${base}/session.json`]=pretty({session_id:assignment.session_id,attempt:assignment.attempt,context_mode:assignment.context_mode});
        sessions[`${base}/session.json`]=assignment.session_id;
      }
    }
    if (payload.todo_id) {
      const base=current.get(payload.todo_id) || `process/todos/${payload.todo_id}/records`;
      files[`${base}/${seq}-${event.kind}.json`]=pretty(event);
      if (payload.candidate) files[`${base}/${seq}-result.json`]=pretty(payload.candidate);
    } else {
      files[`process/parent/events/${seq}-${event.kind}.json`]=pretty(event);
    }
  }
  return {files,sessions};
}

export function pathTodo(path) { return /(?:^|\/)todos\/([^/]+)\//.exec(path || '')?.[1] || ''; }
