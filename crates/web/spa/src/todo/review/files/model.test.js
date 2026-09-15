import {expect,it} from 'vitest';
import {reviewFiles,pathTodo} from './model.js';

it('keeps reruns with reset attempt numbers in distinct dispatch directories',()=>{
  const snapshot={workflow:{parent_session_id:'parent'},nodes:[{id:'t1',status:'passed'}]};
  const events=[
    {seq:10,kind:'todos_dispatched',payload:{world_epoch:0,assignments:[{todo_id:'t1',attempt:1,session_id:'first',context:{text:'old'}}]}},
    {seq:11,kind:'todo_candidate_ready',payload:{todo_id:'t1',candidate:{result:'old result'}}},
    {seq:20,kind:'todos_dispatched',payload:{world_epoch:1,assignments:[{todo_id:'t1',attempt:1,session_id:'second',context:{text:'new'}}]}},
    {seq:21,kind:'todo_candidate_ready',payload:{todo_id:'t1',candidate:{result:'new result'}}},
  ];
  const {files,sessions}=reviewFiles({'objective.md':'frozen'},snapshot,events);
  expect(files['definition/objective.md']).toBe('frozen');
  expect(JSON.parse(files['process/todos/t1/attempts/000000000010/context.json'])).toEqual({text:'old'});
  expect(JSON.parse(files['process/todos/t1/attempts/000000000020/context.json'])).toEqual({text:'new'});
  expect(sessions['process/todos/t1/attempts/000000000010/session.json']).toBe('first');
  expect(pathTodo('definition/todos/t1/context.md')).toBe('t1');
});

it('does not invent an attempt for old events whose dispatch has not been read',()=>{
  const {files}=reviewFiles({},null,[{seq:50,kind:'todo_accepted',payload:{todo_id:'a',reason:'ok'}}]);
  expect(Object.keys(files)).toEqual(['process/todos/a/records/000000000050-todo_accepted.json']);
});
