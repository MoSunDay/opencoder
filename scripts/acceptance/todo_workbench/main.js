const assert=require('assert/strict');
const fs=require('fs');
const path=require('path');
const crypto=require('crypto');
const harness=require('./harness');
const {verifyEditor}=require('./editor');
const {verifyConversation,verifyHistory}=require('./conversation');
let h;
const records=[];
function spaSources(directory){
  const hash=crypto.createHash('sha256');
  function visit(folder){for(const entry of fs.readdirSync(folder,{withFileTypes:true}).sort((a,b)=>a.name.localeCompare(b.name))){
    const file=path.join(folder,entry.name);
    if(entry.isDirectory())visit(file);else hash.update(path.relative(directory,file)).update(fs.readFileSync(file));
  }}
  visit(directory);return hash.digest('hex');
}
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
    return {status:'candidate',summary:`${todo.id} completed`,result:`${todo.id} reviewed result\n\n${Array.from({length:24},(_,i)=>`Evidence ${i+1}: independently inspectable output.`).join('\n\n')}`,verification:'fixture verification',evidence_refs:['result.txt'],recovery_context:{summary:'complete',refs:[]}};
  }
  throw new Error(`Unexpected model prompt: ${prompt.slice(0,120)}`);
}
function spec(){return {schema_version:1,id:'review-ui',name:'TODO Review 交付',objective:'检查父 Agent 调度与独立任务上下文',constraints:['保留执行历史'],metadata:{owner:'review'},todos:[
  {id:'a',title:'收集信息',depends_on:[]},{id:'b',title:'交付结果',depends_on:['a']},{id:'other',title:'独立检查',depends_on:[]},
].map(t=>({...t,agent:'act',requirement_background:'验收工作台',instructions:'返回可以复核的结果',max_attempts:2,acceptance:{criteria:'结果完整'},metadata:{keep:t.id}}))};}
async function main(){
  const spa=path.join(__dirname,'../../../crates/web/spa');
  const sourceSha256=spaSources(path.join(spa,'src'));
  h=await harness.open(answer);console.log('fleet ready');const {page,api,until,root}=h;
  for(const file of ['app.js','app.css'])assert((await h.request('GET',`/static/${file}`)).value===fs.readFileSync(path.join(__dirname,'../../../crates/web/spa/dist/static',file),'utf8'),'Server must serve the current SPA build');
  const {revisedObjective,writes}=await verifyEditor(h,spec());
  console.log('editor real API round-trip passed');
  const id='todos-review-browser';
  await api('POST','/api/todo/templates/review-directory/v2/run',{id});
  await until(async()=>(await api('GET',`/api/executions/${id}`)).execution.status==='done','initial workflow completion');
  await page.getByRole('tab',{name:'运行',exact:true}).click();
  await page.locator(`tr[data-row-key="${id}"]`).click();
  console.log('workflow completed');
  assert.equal(fs.readFileSync(path.join(root,'node-data/todos',id,'definition/objective.md'),'utf8'),revisedObjective);
  const workbench=page.locator('.todo-workbench').first();
  await verifyConversation(h,workbench);
  console.log('parent/task conversation switching passed');
  await workbench.getByRole('tab',{name:'原始记录',exact:true}).click();
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
  await workbench.getByRole('button',{name:'刷新状态'}).click();
  await verifyHistory(h,workbench,id);
  console.log('rerun and historical conversation passed');
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
  assert.equal(spaSources(path.join(spa,'src')),sourceSha256,'SPA sources must stay unchanged throughout acceptance');
  const assets={};
  for(const file of ['app.js','app.css']){
    const bytes=fs.readFileSync(path.join(spa,'dist/static',file));
    assert.equal((await h.request('GET',`/static/${file}`)).value,bytes.toString());
    assets[file]=crypto.createHash('sha256').update(bytes).digest('hex');
  }
  fs.writeFileSync(path.join(root,'result.json'),JSON.stringify({result:'PASS',sourceSha256,assets,errors:h.errors,api:'real Server and Agent; deterministic model fixture only',writes,cases:['context-menu-file-and-directory-operations','parent-Say-default','three-round-trips-retain-scroll-and-details','historical-session-pinning','create-directory','json-markdown-edit','immutable-version-save','runtime-directory-load','invalid-file-modal','context-files','run-review','arbitrary-rerun','parent-session','offline-recovery','restart-history','mobile-layout'],records},null,2));
  console.log(JSON.stringify({result:'PASS',root}));
}
const deadline=setTimeout(()=>{console.error('acceptance deadline');process.exit(1);},300000);
main().catch(async error=>{console.error(error);if(h){console.error(`artifacts: ${h.root}`);await h.page.screenshot({path:path.join(h.root,'failure.png')});fs.writeFileSync(path.join(h.root,'failure.html'),await h.page.content());}process.exitCode=1;}).finally(async()=>{await harness.close();clearTimeout(deadline);});
