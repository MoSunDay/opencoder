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
  for(const file of ['app.js','app.css'])assert((await h.request('GET',`/static/${file}`)).value===fs.readFileSync(path.join(__dirname,'../../../crates/web/spa/dist/static',file),'utf8'),'Server must serve the current SPA build');
  await page.getByText('Agent',{exact:true}).first().click();
  await page.getByRole('menuitem',{name:'TODO 管理'}).click();
  await page.getByRole('button',{name:'新建模板',exact:true}).click();
  const editor=page.locator('.todo-directory-editor');
  await editor.locator('.file-workspace').waitFor();
  await editor.getByLabel('模板名',{exact:true}).fill('review-directory');
  const clickFile=async(scope,file)=>{
    await scope.getByLabel('搜索文件',{exact:true}).fill(file);
    await scope.locator(`[data-file-path="${file}"]`).click();
    return scope.getByLabel(`文件内容 ${file}`,{exact:true});
  };
  const editFile=async(file,text)=>{await (await clickFile(editor,file)).fill(text);};
  await clickFile(editor,'todos/t1/task.json');
  await editor.getByRole('button',{name:'重命名 TODO',exact:true}).click();
  let operation=page.locator('.ant-modal:visible');
  await operation.getByLabel('TODO ID',{exact:true}).fill('a');
  await operation.getByRole('button',{name:/^确\s*定$/}).click();
  for(const id of ['b','other']){
    await editor.getByRole('button',{name:'新增 TODO',exact:true}).click();
    operation=page.locator('.ant-modal:visible');
    await operation.getByLabel('TODO ID',{exact:true}).fill(id);
    await operation.getByRole('button',{name:/^确\s*定$/}).click();
  }
  const definition=spec();
  const {objective,todos,...manifest}=definition;
  await editFile('workflow.json',JSON.stringify({...manifest,todos:todos.map(t=>t.id)},null,2));
  await editFile('objective.md',objective);
  for(const todo of todos){
    const {id,requirement_background,instructions,acceptance,...task}=todo;
    await editFile(`todos/${id}/task.json`,JSON.stringify({...task,required_tool_calls:[]},null,2));
    await editFile(`todos/${id}/context.md`,requirement_background);
    await editFile(`todos/${id}/instructions.md`,instructions);
    await editFile(`todos/${id}/acceptance.md`,acceptance.criteria);
  }
  const broken='todos/b/task.json';
  const valid=(await clickFile(editor,broken));const source=await valid.innerText();
  await valid.fill('{\ninvalid');
  await editor.getByRole('button',{name:/创建模板$/}).click();
  const problem=page.locator('.ant-modal:visible');
  await problem.getByText('文件不符合 TODO 框架要求',{exact:true}).waitFor();
  assert.equal((await h.request('GET','/api/todo/templates/review-directory')).response.status,404);
  await page.screenshot({path:path.join(root,'00-invalid-file.png'),animations:'disabled'});
  await problem.getByRole('button',{name:/todos\/b\/task.json:/}).click();
  await editor.getByLabel(`文件内容 ${broken}`,{exact:true}).fill(source);
  await editFile('objective.md','# 执行目标\n\n'+objective);
  await editor.getByText('分屏',{exact:true}).click();
  await editor.locator('.file-editor-preview h1').waitFor();
  await editor.getByLabel('搜索文件',{exact:true}).fill('');
  await page.screenshot({path:path.join(root,'01-editor.png'),animations:'disabled'});
  await editor.getByRole('button',{name:/创建模板$/}).click();
  await editor.waitFor({state:'hidden'});
  console.log('template created');
  const saved=await api('GET','/api/todo/templates/review-directory/v1/context.json');
  assert.equal(saved.todos[1].metadata.keep,'b');
  await page.locator('tr[data-row-key="review-directory"] .ant-table-row-expand-icon').click();
  await page.getByRole('button',{name:'编辑',exact:true}).click();
  await editor.locator('.file-workspace').waitFor();
  const revisedObjective=objective+'\n必须保留每次执行上下文';
  await editFile('objective.md',revisedObjective);
  await editor.getByLabel('文件内容 objective.md',{exact:true}).press('Control+s');
  await editor.getByText('review-directory / v2',{exact:true}).waitFor();
  await editor.getByRole('button',{name:/^返\s*回$/}).click();
  await editor.waitFor({state:'hidden'});
  assert.equal((await api('GET','/api/todo/templates/review-directory/v1/context.json')).objective,saved.objective);
  assert.equal((await api('GET','/api/todo/templates/review-directory/v2/context.json')).objective,revisedObjective);
  const id='todos-review-browser';
  await api('POST','/api/todo/templates/review-directory/v2/run',{id});
  await until(async()=>(await api('GET',`/api/executions/${id}`)).execution.status==='done','initial workflow completion');
  await page.getByRole('tab',{name:'运行',exact:true}).click();
  await page.locator(`tr[data-row-key="${id}"]`).click();
  console.log('workflow completed');
  assert.equal(fs.readFileSync(path.join(root,'node-data/todos',id,'definition/objective.md'),'utf8'),revisedObjective);
  const workbench=page.locator('.todo-workbench').first();
  const context='process/todos/b/attempts/';
  await until(async()=>await workbench.locator(`[data-file-path^="${context}"][data-file-path$="/context.json"]`).count()>0,'dispatch context file');
  await workbench.locator(`[data-file-path^="${context}"][data-file-path$="/context.json"]`).first().click();
  await until(async()=>(await workbench.locator('.cm-content').innerText()).includes('a reviewed result'),'complete accepted dependency context');
  assert.equal(await workbench.locator('.cm-content').getAttribute('contenteditable'),'false');
  assert(await workbench.locator('.file-workspace-tree').evaluate(tree=>tree.scrollWidth<=tree.clientWidth+1),'directory names must fit without hiding folder indentation');
  await page.screenshot({path:path.join(root,'02-review.png'),animations:'disabled'});
  await workbench.getByRole('button',{name:'从选中任务重跑'}).click();
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
  await workbench.getByRole('button',{name:'刷新过程记录'}).click();
  await until(async()=>await workbench.locator(`[data-file-path^="${context}"][data-file-path$="/context.json"]`).count()===2,'both dispatch context files retained');
  await workbench.getByRole('button',{name:'父 Agent 会话'}).click();
  const session=page.getByRole('dialog').filter({hasText:'会话 Review'});
  await session.getByText('All results reviewed',{exact:false}).first().waitFor();
  await page.screenshot({path:path.join(root,'04-parent-session.png'),animations:'disabled',timeout:60000});
  await session.locator('.ant-drawer-close').click();
  assert(await workbench.getByRole('button',{name:'从选中任务重跑'}).isEnabled(),'opening the parent session preserves the selected task');
  await page.setViewportSize({width:390,height:900});
  await h.pause(300);
  assert(await workbench.evaluate(node=>node.scrollWidth<=node.clientWidth+1),'mobile workbench must not overflow horizontally');
  await page.screenshot({path:path.join(root,'06-mobile.png'),animations:'disabled',timeout:60000});
  await page.setViewportSize({width:1600,height:1000});
  // Offline data must be visible and control actions must stop until synchronization succeeds.
  await page.route('**/api/todo/workflows/*/review?*',route=>route.fulfill({status:503,contentType:'application/json',body:JSON.stringify({error:'fixture node offline'})}));
  await workbench.getByRole('button',{name:'刷新状态'}).click();
  await workbench.getByText('fixture node offline',{exact:true}).waitFor();
  assert(await workbench.getByRole('button',{name:'从选中任务重跑'}).isDisabled());
  await page.screenshot({path:path.join(root,'05-offline.png'),animations:'disabled',timeout:60000});
  await page.unroute('**/api/todo/workflows/*/review?*');
  await workbench.getByRole('button',{name:'刷新状态'}).click();
  await until(async()=>await workbench.getByText('fixture node offline',{exact:true}).count()===0,'connection recovered');
  await h.restart();
  await until(async()=> (await api('GET',`/api/todo/workflows/${id}/review?section=node&todo_id=b`)).state.session_history.length===2,'history after restart');
  assert.deepEqual(h.errors,[]);
  fs.writeFileSync(path.join(root,'result.json'),JSON.stringify({result:'PASS',cases:['create-directory','json-markdown-edit','immutable-version-save','runtime-directory-load','invalid-file-modal','context-files','run-review','arbitrary-rerun','parent-session','offline-recovery','restart-history','mobile-layout'],records},null,2));
  console.log(JSON.stringify({result:'PASS',root}));
}
const deadline=setTimeout(()=>{console.error('acceptance deadline');process.exit(1);},300000);
main().catch(async error=>{console.error(error);if(h){console.error(`artifacts: ${h.root}`);await h.page.screenshot({path:path.join(h.root,'failure.png')});fs.writeFileSync(path.join(h.root,'failure.html'),await h.page.content());}process.exitCode=1;}).finally(async()=>{await harness.close();clearTimeout(deadline);});
