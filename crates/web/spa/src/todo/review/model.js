export function applyFrame(snapshot,frame) {
  if(!snapshot?.workflow||!Number.isFinite(frame?.seq)||frame.seq<=snapshot.head_seq)return snapshot;
  const data=frame.data;
  if(!data||!Number.isFinite(data.generation)||data.generation<snapshot.workflow.generation)return snapshot;
  if(!Array.isArray(data.items))return snapshot;
  const updates=new Map(data.items.map(item=>[item.todo_id,item]));
  return {...snapshot,head_seq:frame.seq,workflow:{...snapshot.workflow,generation:data.generation,
    world_epoch:data.world_epoch,status:data.workflow_status},
    nodes:snapshot.nodes.map(node=>updates.has(node.id)?{...node,...updates.get(node.id)}:node),
    latest_event:{seq:frame.seq,kind:frame.event,payload:data}};
}
