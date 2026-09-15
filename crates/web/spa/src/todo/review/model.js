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

export function relatedEvent(event,todoId) {
  if(!todoId)return true;
  const payload=event.payload||{};
  return payload.todo_id===todoId || payload.todos?.some(t=>t.todo_id===todoId)
    || payload.assignments?.some(t=>t.todo_id===todoId)
    || event.kind.startsWith('workflow_') || event.kind==='milestone_marked'&&payload.todo_id===todoId;
}

export const EVENT_LABELS={todos_dispatched:'父 Agent 派发',todo_candidate_ready:'提交候选结果',todo_acceptance_started:'开始验收',
  todo_accepted:'验收通过',todo_revision_requested:'要求返工',todo_execution_failed:'执行失败',todo_failed:'验收失败',
  workflow_rerun_requested:'请求节点重跑',workflow_rerun_applied:'重跑已生效',workflow_rewound:'回到里程碑',
  workflow_completed:'工作流完成',workflow_failed:'工作流失败',workflow_interrupted:'工作流中断',workflow_resumed:'工作流恢复',milestone_marked:'标记里程碑'};
