const assert=require('assert/strict');
const path=require('path');

async function verifyConversation(h,workbench){
  const {page,root}=h;
  const parent=workbench.getByLabel('父 Agent 的对话',{exact:true});
  await parent.getByText('All results reviewed',{exact:true}).waitFor();
  assert.equal(await workbench.locator('[data-file-path]').count(),0,'conversation is the default view');
  await parent.getByText('原始回复',{exact:true}).click();
  await parent.locator('pre.todo-review-json').first().waitFor();
  await parent.evaluate(element=>{element.scrollTop=300;element.dispatchEvent(new Event('scroll'));});
  const parentTop=await parent.evaluate(element=>element.scrollTop);assert(parentTop>0);
  await workbench.locator('[data-todo-id="b"]').click();
  const child=workbench.getByLabel('TODO b 的对话',{exact:true});
  await child.getByText('b completed',{exact:true}).waitFor();
  await child.getByText('原始回复',{exact:true}).click();await child.locator('pre.todo-review-json').waitFor();
  await child.evaluate(element=>{element.scrollTop=180;element.dispatchEvent(new Event('scroll'));});
  const childTop=await child.evaluate(element=>element.scrollTop);assert(childTop>0);
  for(let i=0;i<3;i++){
    await workbench.getByRole('button',{name:'父 Agent',exact:true}).click();
    assert.equal(await parent.evaluate(element=>element.scrollTop),parentTop);
    assert(await parent.locator('pre.todo-review-json').first().isVisible());
    await workbench.locator('[data-todo-id="b"]').click();
    assert.equal(await child.evaluate(element=>element.scrollTop),childTop);
    assert(await child.locator('pre.todo-review-json').isVisible());
  }
  await workbench.getByText('执行结果',{exact:true}).click();
  await workbench.getByText('b reviewed result',{exact:true}).first().waitFor();
  await page.screenshot({path:path.join(root,'02-conversation.png'),animations:'disabled'});
}

async function verifyHistory(h,workbench,id){
  const {page,api,until,root}=h;
  await workbench.getByRole('tab',{name:'执行对话',exact:true}).click();
  await workbench.locator('[data-todo-id="b"]').click();
  const detail=await api('GET',`/api/todo/workflows/${id}/review?section=node&todo_id=b`);
  const old=detail.state.session_history[0];
  await workbench.getByRole('combobox',{name:'执行会话'}).click();await page.getByText('会话 1',{exact:true}).click();
  const session=workbench.locator(`[data-session-id="${old}"]`);
  await session.getByText('b completed',{exact:true}).waitFor();
  await session.getByRole('tab',{name:'执行事件',exact:true}).click();
  const events=session.getByRole('tabpanel',{name:'执行事件',exact:true});
  await events.locator('.ant-collapse-header').first().click();await events.locator('pre.todo-review-json').waitFor();
  const evidence=await events.locator('pre.todo-review-json').innerText();assert(evidence.length>2);
  await workbench.getByRole('button',{name:'父 Agent',exact:true}).click();
  await workbench.getByLabel('父 Agent 的对话',{exact:true}).getByText('All results reviewed',{exact:true}).first().waitFor();
  await workbench.locator('[data-todo-id="b"]').click();assert.equal(await events.locator('pre.todo-review-json').innerText(),evidence);
  await workbench.getByRole('tab',{name:'原始记录',exact:true}).click();
  await workbench.getByRole('tab',{name:'执行对话',exact:true}).click();assert(await events.locator('pre.todo-review-json').isVisible());
  await page.setViewportSize({width:390,height:900});
  assert(await workbench.evaluate(node=>node.scrollWidth<=node.clientWidth+1),'mobile workbench must not overflow');
  await workbench.getByRole('button',{name:'运行操作',exact:true}).click();
  await page.getByRole('menuitem',{name:'从选中任务重跑',exact:true}).waitFor();await page.keyboard.press('Escape');
  await workbench.getByRole('button',{name:'父 Agent',exact:true}).click();
  await page.screenshot({path:path.join(root,'04-mobile.png'),animations:'disabled'});
  await page.setViewportSize({width:1600,height:1000});
  await workbench.locator('[data-todo-id="b"]').click();
  await until(async()=>await events.locator('pre.todo-review-json').isVisible(),'pinned session remains visible');
}
module.exports={verifyConversation,verifyHistory};
