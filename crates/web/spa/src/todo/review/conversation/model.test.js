import {expect,it} from 'vitest';
import {presentSay,sessionPresentation,taskSessions} from './model.js';

it('renders structured decisions and results while preserving the exact original reply',()=>{
  const decision=JSON.stringify({operation:'dispatch',reason:'先完成验证',todos:[{todo_id:'t1'},null]});
  const messages=[{role:'user',blocks:[{kind:'text',text:'large dispatch context'}]},
    {role:'assistant',blocks:[{kind:'text',text:decision}]}];
  const value=sessionPresentation(messages);
  expect(value.turns[0].text).toBe('**派发任务**\n\n先完成验证\n\nTODO：t1');
  expect(value.raw[0].text).toBe(decision);
  expect(value.inputs[0].text).toBe('large dispatch context');
  expect(presentSay(JSON.stringify({status:'candidate',summary:'已实现',result:'结果',verification:'通过'}))).toBe('已实现\n\n结果\n\n**验证**\n\n通过');
  for(const text of ['# 普通回复','[1,2]','null','{"operation":"unknown"}'])expect(presentSay(text)).toBe(text);
});

it('keeps historical sessions in order and uses the last session after execution ends',()=>{
  expect(taskSessions({}, {state:{session_history:['a','b','a']}})).toEqual({sessions:['a','b'],current:'b'});
  expect(taskSessions({active_session_id:'c'}, {state:{session_history:['a','b'],active_session_id:'b'}})).toEqual({sessions:['a','b','c'],current:'c'});
});
