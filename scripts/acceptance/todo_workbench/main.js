const assert=require('assert/strict');
const fs=require('fs');
const path=require('path');
const harness=require('./harness');
let h;
const records=[];
function field(prompt,key){const line=prompt.split('\n').find(line=>line.startsWith(`${key}=`));return line?JSON.parse(line.slice(key.length+1)):null;}
async function answer(prompt){
  if(prompt.includes('Decide the next workflow operation')){
    const state=field(prompt,'STATE'),ready=field(prompt,'RUNNABLE');
    if(!ready.length)return {operation:'complete',reason:'All results reviewed'};
    const id=ready[0];return {operation:'dispatch',todos:[{todo_id:id,context_mode:state.todos[id].next_context_mode||'new'}],reason:`Ready ${id}`};
  }
  if(prompt.includes('Accept or reject one TODO candidate'))return {operation:'accept',reason:'Verified candidate evidence',mark_milestone:false};
  if(prompt.includes('Complete exactly one focused TODO')){
    const todo=field(prompt,'TODO');records.push({todo:todo.id,context:field(prompt,'ACCEPTED_DEPENDENCIES'),rerun:field(prompt,'RERUN')});
    return {status:'candidate',summary:`${todo.id} completed`,result:`${todo.id} reviewed result`,verification:'fixture verification',evidence_refs:['result.txt'],recovery_context:{summary:'complete',refs:[]}};
  }
  throw new Error(`Unexpected model prompt: ${prompt.slice(0,120)}`);
}
function spec(){return {schema_version:1,id:'review-ui',name:'TODO Review 交付',objective:'检查父 Agent 调度与独立任务上下文',constraints:['保留执行历史'],metadata:{owner:'review'},todos:[
  {id:'a',title:'收集信息',depends_on:[]},{id:'b',title:'交付结果',depends_on:['a']},{id:'other',title:'独立检查',depends_on:[]},
].map(t=>({...t,agent:'act',requirement_background:'验收工作台',instructions:'返回可以复核的结果',max_attempts:2,acceptance:{criteria:'结果完整'},metadata:{keep:t.id}}))};}
async function main(){
  h=await harness.open(answer);console.log('fleet ready');const {page,api,until,root}=h;
  await page.getByText('Agent',{exact:true}).first().click();
  await page.getByRole('menuitem',{name:'TODO 管理'}).click();
  await page.getByRole('button',{name:'新建模板',exact:true}).click();
  const editor=page.locator('.ant-drawer:visible');
  await editor.locator('.todo-edit-canvas').waitFor();
  await editor.getByLabel('模板名',{exact:true}).fill('review-canvas');
  await editor.getByText('JSON 源码',{exact:true}).click();
  await editor.getByLabel('spec-json').fill(JSON.stringify(spec()));
  await editor.getByText('画布',{exact:true}).click();
  await until(async()=>await editor.locator('.dag-edit-node').count()===3,'three edit nodes');
  await editor.locator('.dag-edit-node').filter({hasText:'交付结果'}).click();
  await editor.getByRole('button',{name:'预览派发上下文'}).click();
  if(!await editor.getByText('workflow_objective',{exact:false}).last().isVisible()) await editor.getByText('派发上下文',{exact:true}).click();
  await editor.getByText('workflow_objective',{exact:false}).last().waitFor();
  await page.screenshot({path:path.join(root,'01-editor.png'),animations:'disabled',timeout:60000});
  await editor.getByRole('button',{name:/^创\s*建$/}).click();
  await editor.waitFor({state:'hidden'});
  console.log('template created');
  const saved=await api('GET','/api/todo/templates/review-canvas/v1/context.json');
  assert.equal(saved.todos[1].metadata.keep,'b');
  const id='todos-review-browser';
  await api('POST','/api/todo/templates/review-canvas/v1/run',{id});
  await until(async()=>(await api('GET',`/api/executions/${id}`)).execution.status==='done','initial workflow completion');
  await page.getByRole('tab',{name:'运行',exact:true}).click();
  await page.locator(`tr[data-row-key="${id}"]`).click();
  console.log('workflow completed');
  const workbench=page.locator('.todo-workbench').first();
  await until(async()=>await workbench.locator('.oc-todo-run-node').count()===3,'three running nodes');
  await workbench.locator('.todo-nav-node').filter({hasText:'交付结果'}).click();
  await workbench.locator('.todo-review-inspector').getByText('b reviewed result',{exact:true}).waitFor();
  await workbench.getByRole('tab',{name:'派发上下文'}).click();
  await workbench.locator('.todo-review-inspector pre').filter({hasText:'a reviewed result'}).waitFor();
  await page.screenshot({path:path.join(root,'02-review.png'),animations:'disabled',timeout:60000});
  await workbench.getByRole('button',{name:'从选中节点重跑'}).click();
  const modal=page.getByRole('dialog').filter({hasText:'从 b 重新执行'});
  await modal.getByLabel('重跑原因').fill('检查修订后的交付结果');
  await modal.getByText('保留当前文件、外部操作结果和历史记录。',{exact:false}).waitFor();
  await page.screenshot({path:path.join(root,'03-rerun.png'),animations:'disabled',timeout:60000});
  await modal.getByRole('button',{name:'确认暂停并重跑'}).click();
  await modal.waitFor({state:'hidden'});
  await until(async()=>{
    const snapshot=await api('GET',`/api/todo/workflows/${id}/review?section=overview`);
    return snapshot.workflow.world_epoch===1&&snapshot.execution_status==='done';
  },'rerun done');
  const b=await api('GET',`/api/todo/workflows/${id}/review?section=node&todo_id=b`);
  assert.equal(b.state.session_history.length,2);
  assert.deepEqual(records.map(r=>r.todo).sort(),['a','b','b','other']);
  assert.equal(records.filter(r=>r.todo==='b').at(-1).rerun.reason,'检查修订后的交付结果');
  await workbench.getByRole('tab',{name:'历史尝试'}).click();
  await workbench.locator('.todo-history').first().getByText('父 Agent 派发',{exact:false}).first().waitFor();
  await workbench.getByRole('button',{name:'父 Agent 会话'}).click();
  const session=page.getByRole('dialog').filter({hasText:'会话 Review'});
  await session.getByText('All results reviewed',{exact:false}).first().waitFor();
  await page.screenshot({path:path.join(root,'04-parent-session.png'),animations:'disabled',timeout:60000});
  await session.locator('.ant-drawer-close').click();
  await page.setViewportSize({width:390,height:900});
  await h.pause(300);
  assert(await workbench.evaluate(node=>node.scrollWidth<=node.clientWidth+1),'mobile workbench must not overflow horizontally');
  await page.screenshot({path:path.join(root,'06-mobile.png'),animations:'disabled',timeout:60000});
  await page.setViewportSize({width:1600,height:1000});
  // Offline data must be visible and control actions must stop until synchronization succeeds.
  await page.route('**/api/todo/workflows/*/review?*',route=>route.fulfill({status:503,contentType:'application/json',body:JSON.stringify({error:'fixture node offline'})}));
  await workbench.getByRole('button',{name:'刷新状态'}).click();
  await workbench.getByText('fixture node offline',{exact:true}).waitFor();
  assert(await workbench.getByRole('button',{name:'从选中节点重跑'}).isDisabled());
  await page.screenshot({path:path.join(root,'05-offline.png'),animations:'disabled',timeout:60000});
  await page.unroute('**/api/todo/workflows/*/review?*');
  await workbench.getByRole('button',{name:'刷新状态'}).click();
  await until(async()=>await workbench.getByText('fixture node offline',{exact:true}).count()===0,'connection recovered');
  await h.restart();
  await until(async()=> (await api('GET',`/api/todo/workflows/${id}/review?section=node&todo_id=b`)).state.session_history.length===2,'history after restart');
  assert.deepEqual(h.errors,[]);
  fs.writeFileSync(path.join(root,'result.json'),JSON.stringify({result:'PASS',cases:['create-canvas','context-preview','run-review','arbitrary-rerun','parent-session','offline-recovery','restart-history','mobile-layout'],records},null,2));
  console.log(JSON.stringify({result:'PASS',root}));
}
const deadline=setTimeout(()=>{console.error('acceptance deadline');process.exit(1);},300000);
main().catch(async error=>{console.error(error);if(h){console.error(`artifacts: ${h.root}`);await h.page.screenshot({path:path.join(h.root,'failure.png')});}process.exitCode=1;}).finally(async()=>{await harness.close();clearTimeout(deadline);});
