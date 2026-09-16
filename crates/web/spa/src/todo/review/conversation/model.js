import {turnsFromMessages} from '../../../reduce.js';

const OPERATIONS = {dispatch:'派发任务',accept:'验收通过',revise:'要求修改',fail:'执行失败',complete:'工作流完成',suspend:'暂停执行',rewind:'回到里程碑',mark_milestone:'标记里程碑'};

// Structured TODO replies are still Say. Keep their original text for inspection.
export function presentSay(text) {
  let value;
  try {value=JSON.parse(text);} catch {return text;}
  if (!value || typeof value!=='object' || Array.isArray(value)) return text;
  if (OPERATIONS[value.operation] && typeof value.reason==='string') {
    const tasks=[...(Array.isArray(value.todos) ? value.todos.map(todo=>todo?.todo_id) : []),value.todo_id,value.milestone_todo_id].filter(id=>typeof id==='string');
    return `**${OPERATIONS[value.operation]}**\n\n${value.reason}${tasks.length ? `\n\nTODO：${tasks.join('、')}` : ''}`;
  }
  if (['candidate','blocked','interrupted'].includes(value.status) && typeof value.summary==='string') {
    return [value.status==='blocked'?'**已阻塞**':value.status==='interrupted'?'**已中断**':'',value.summary,typeof value.result==='string'&&value.result,
      typeof value.verification==='string'&&`**验证**\n\n${value.verification}`].filter(Boolean).join('\n\n');
  }
  return text;
}

export function sessionPresentation(messages) {
  const original=turnsFromMessages(messages || []);
  const raw=[];
  const turns=original.filter(turn=>turn.role!=='user').map((turn,index)=>{
    if (turn.kind!=='text' || turn.role!=='assistant' || turn.image) return turn;
    const text=presentSay(turn.text);
    if(text!==turn.text)raw.push({key:index,text:turn.text});
    return text===turn.text ? turn : {...turn,text};
  });
  return {turns,raw,inputs:original.filter(turn=>turn.role==='user')};
}

export function taskSessions(node, detail) {
  const sessions=[...new Set([...(detail?.state?.session_history || []),detail?.state?.active_session_id,node?.active_session_id].filter(Boolean))];
  return {sessions,current:node?.active_session_id || detail?.state?.active_session_id || sessions.at(-1) || ''};
}
