const assert=require('assert/strict');
const path=require('path');

async function verifyEditor(h,definition){
  const {page,api,until,root}=h;
  const writes=[];
  page.on('request',request=>{
    const url=new URL(request.url());
    if(request.method()==='POST'&&url.pathname.startsWith('/api/todo/'))writes.push(url.pathname);
  });
  await page.getByText('Agent',{exact:true}).first().click();
  await page.getByRole('menuitem',{name:'TODO 管理'}).click();
  await page.getByRole('button',{name:'新建模板',exact:true}).click();
  const editor=page.getByLabel('TODO 目录编辑器');
  await editor.locator('.file-workspace').waitFor();
  assert.deepEqual((await editor.locator('.todo-directory-toolbar button').allTextContents()).map(text=>text.replace(/\s/g,'')),['返回','保存']);
  assert.equal(await editor.getByText(/父 Agent|执行 Agent/).count(),0);
  await editor.getByLabel('模板名',{exact:true}).fill('review-directory');
  const node=file=>editor.locator(`[data-file-path="${file}"]`);
  async function menu(file,label){
    await editor.getByLabel('搜索文件',{exact:true}).fill('');
    await node(file).click({button:'right'});
    await page.getByRole('menuitem',{name:label,exact:true}).click();
  }
  async function name(kind,value){
    const input=page.getByLabel(`${kind}名称`,{exact:true});
    await input.fill(value);await input.press('Enter');await input.waitFor({state:'hidden'});
  }
  async function edit(file,text){
    await editor.getByLabel('搜索文件',{exact:true}).fill(file);
    await node(file).click();
    await editor.getByLabel(`文件内容 ${file}`,{exact:true}).fill(text);
  }
  await menu('todos/t1','重命名目录');await name('目录','a');
  for(const id of ['b','other','discard']){await menu('todos','新增目录');await name('目录',id);}
  await menu('todos/discard','删除目录');await page.getByRole('button',{name:/^删\s*除$/}).click();
  await node('todos/discard').waitFor({state:'hidden'});
  await menu('todos/a/context.md','新增文件');await name('文件','draft.md');
  await edit('todos/a/draft.md','temporary file');
  await menu('todos/a/draft.md','重命名文件');await name('文件','notes.md');
  await menu('todos/a/notes.md','删除文件');await page.getByRole('button',{name:/^删\s*除$/}).click();
  await node('todos/a/notes.md').waitFor({state:'hidden'});
  await node('todos/a/context.md').waitFor();
  const {objective,todos,...manifest}=definition;
  await edit('workflow.json',JSON.stringify({...manifest,todos:todos.map(todo=>todo.id)},null,2));
  await edit('objective.md',objective);
  for(const todo of todos){
    const {id,requirement_background,instructions,acceptance,...task}=todo;
    await edit(`todos/${id}/task.json`,JSON.stringify({...task,required_tool_calls:[]},null,2));
    await edit(`todos/${id}/context.md`,requirement_background);
    await edit(`todos/${id}/instructions.md`,instructions);
    await edit(`todos/${id}/acceptance.md`,acceptance.criteria);
  }
  const broken='todos/b/task.json';
  await editor.getByLabel('搜索文件',{exact:true}).fill(broken);await node(broken).click();
  const content=editor.getByLabel(`文件内容 ${broken}`,{exact:true});const original=await content.innerText();
  await content.fill('{\ninvalid');await editor.getByRole('button',{name:/保\s*存$/}).click();
  const problem=page.getByRole('dialog').filter({hasText:'文件不符合 TODO 框架要求'});
  await problem.waitFor();assert.deepEqual(writes,[]);
  assert.equal((await h.request('GET','/api/todo/templates/review-directory')).response.status,404);
  await page.screenshot({path:path.join(root,'00-invalid-file.png'),animations:'disabled'});
  await problem.getByRole('button',{name:/todos\/b\/task.json:/}).click();
  await content.fill(original);
  await edit('objective.md','# 执行目标\n\n'+objective);
  await editor.getByText('分屏',{exact:true}).click();await editor.locator('.file-editor-preview h1').waitFor();
  await editor.getByLabel('搜索文件',{exact:true}).fill('');
  await page.screenshot({path:path.join(root,'01-editor.png'),animations:'disabled'});
  await editor.getByRole('button',{name:/保\s*存$/}).click();await editor.waitFor({state:'hidden'});
  assert.deepEqual(writes,['/api/todo/validate-files','/api/todo/templates']);
  const saved=await api('GET','/api/todo/templates/review-directory/v1/context.json');
  assert.deepEqual(saved.todos.map(todo=>todo.id),['a','b','other']);assert.equal(saved.todos[1].metadata.keep,'b');
  await page.locator('tr[data-row-key="review-directory"] .ant-table-row-expand-icon').click();
  await page.getByRole('button',{name:'编辑',exact:true}).click();await editor.locator('.file-workspace').waitFor();
  const revisedObjective=objective+'\n必须保留每次执行上下文';
  await edit('objective.md',revisedObjective);await editor.getByLabel('文件内容 objective.md',{exact:true}).press('Control+s');
  await until(async()=>(await api('GET','/api/todo/templates/review-directory')).template.current==='v2','saved version v2');
  await editor.getByRole('button',{name:/^返\s*回$/}).click();await editor.waitFor({state:'hidden'});
  assert.deepEqual(writes.slice(-2),['/api/todo/validate-files','/api/todo/templates/review-directory/new-version']);
  assert.equal((await api('GET','/api/todo/templates/review-directory/v1/context.json')).objective,saved.objective);
  assert.equal((await api('GET','/api/todo/templates/review-directory/v2/context.json')).objective,revisedObjective);
  await page.reload();await page.getByText('Agent',{exact:true}).first().click();
  await page.getByRole('menuitem',{name:'TODO 管理'}).click();
  await page.locator('tr[data-row-key="review-directory"] .ant-table-row-expand-icon').click();
  await page.getByRole('button',{name:'编辑',exact:true}).last().click();await editor.locator('.file-workspace').waitFor();
  await editor.getByLabel('搜索文件',{exact:true}).fill('objective.md');await node('objective.md').click();
  assert.equal(await editor.getByLabel('文件内容 objective.md',{exact:true}).innerText(),revisedObjective);
  await editor.getByRole('button',{name:/^返\s*回$/}).click();await editor.waitFor({state:'hidden'});
  return {revisedObjective,writes};
}
module.exports={verifyEditor};
