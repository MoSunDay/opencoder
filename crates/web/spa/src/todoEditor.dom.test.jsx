// @vitest-environment jsdom
import './test/setup-dom.js';
import {beforeEach,expect,it,vi} from 'vitest';
import {act,fireEvent,render,screen,waitFor} from '@testing-library/react';
import {EditorView} from '@codemirror/view';
import {TodoEditor} from './todoEditor.jsx';
import {EXAMPLE_SPEC,specFiles} from './todo/directory/model.js';
import {apiGet,apiPost} from './api.js';
vi.mock('./api.js',()=>({apiGet:vi.fn(),apiPost:vi.fn()}));
const button=text=>[...document.querySelectorAll('button')].find(b=>b.textContent.replace(/\s/g,'')===text);
const editor=()=>EditorView.findFromDOM(document.querySelector('.cm-editor'));
const replace=text=>act(()=>{const view=editor();view.dispatch({changes:{from:0,to:view.state.doc.length,insert:text}});});
const fixture=()=>specFiles(EXAMPLE_SPEC);
beforeEach(()=>{
  Range.prototype.getClientRects=()=>[];Range.prototype.getBoundingClientRect=()=>({left:0,right:0,top:0,bottom:0});
  apiGet.mockReset().mockImplementation(async path=>{
    if(path==='/api/agents')return {agents:[{name:'reviewer',primary:true}]};
    if(path==='/api/todo/envs')return {envs:[]};
    return {files:fixture(),revision:'revision-1',diagnostics:[]};
  });
  apiPost.mockReset().mockImplementation(async path=>path.endsWith('validate-files')?{valid:true}:{version:'v2',revision:'revision-2'});
});
const mount=async props=>{const result=render(<TodoEditor templateName="demo" version="v1" onClose={()=>{}} {...props}/>);await screen.findByLabelText('文件内容 objective.md');return result;};
it('saves a complete directory as a new version and retains the current draft',async()=>{
  await mount();replace('# 新目标\n保留上下文');fireEvent.click(button('保存'));
  await waitFor(()=>expect(apiPost).toHaveBeenCalledWith('/api/todo/templates/demo/new-version',expect.objectContaining({source_version:'v1',expected_revision:'revision-1',files:expect.objectContaining({'objective.md':'# 新目标\n保留上下文'})})));
  expect(apiPost.mock.calls[0][0]).toBe('/api/todo/validate-files');
  expect(editor().state.doc.toString()).toBe('# 新目标\n保留上下文');
  await waitFor(()=>expect(button('保存').disabled).toBe(false));
  replace('# 再次保存');fireEvent.click(button('保存'));
  await waitFor(()=>expect(apiPost).toHaveBeenCalledWith('/api/todo/templates/demo/new-version',expect.objectContaining({source_version:'v2',expected_revision:'revision-2'})));
});
it('preserves invalid JSON while switching files and blocks save with a locating modal',async()=>{
  await mount();fireEvent.click(screen.getByText('workflow.json',{exact:true}));
  await screen.findByLabelText('文件内容 workflow.json');replace('{\n "broken": }');
  fireEvent.click(document.querySelector('[data-file-path="objective.md"]'));await screen.findByLabelText('文件内容 objective.md');
  fireEvent.click(button('保存'));expect(await screen.findByText('文件不符合 TODO 框架要求')).toBeTruthy();expect(apiPost).not.toHaveBeenCalled();
  fireEvent.click(screen.getByText(/workflow.json:2:/));await screen.findByLabelText('文件内容 workflow.json');expect(editor().state.doc.toString()).toBe('{\n "broken": }');
});
it('shows framework diagnostics immediately when an existing file is invalid',async()=>{
  const files=fixture();files['todos/t1/context.md']='';
  apiGet.mockImplementation(async path=>path==='/api/agents'?{agents:[{name:'reviewer',primary:true}]}:path==='/api/todo/envs'?{envs:[]}:{files,revision:'r'});
  await mount();expect(await screen.findByText('文件不符合 TODO 框架要求')).toBeTruthy();expect(screen.getByText(/todos\/t1\/context.md:1:1/)).toBeTruthy();
});
it('shows server validation errors and keeps edited contents on save failure',async()=>{
  await mount();replace('server validation example');
  apiPost.mockRejectedValueOnce(Object.assign(new Error('invalid'),{status:400,body:{diagnostics:[{path:'env.json',message:'环境不存在',line:1,column:1}]}}));
  fireEvent.click(button('保存'));expect(await screen.findByText('环境不存在')).toBeTruthy();expect(editor().state.doc.toString()).toBe('server validation example');expect(apiPost).toHaveBeenCalledTimes(1);
});
it('Markdown supports source and sanitized preview without changing saved text',async()=>{
  await mount();replace('# 标题\n<script>window.bad=true</script>\n**内容**');fireEvent.click(screen.getByText('预览',{exact:true}));
  expect(await screen.findByRole('heading',{name:'标题'})).toBeTruthy();expect(document.querySelector('.file-editor-preview script')).toBeNull();
  fireEvent.click(screen.getByText('源码',{exact:true}));expect(editor().state.doc.toString()).toContain('<script>');
});
it('missing files can be repaired from the diagnostic without discarding other files',async()=>{
  const files=fixture();delete files['todos/t1/context.md'];
  apiGet.mockImplementation(async path=>path==='/api/agents'?{agents:[{name:'reviewer',primary:true}]}:path==='/api/todo/envs'?{envs:[]}:{files,revision:'r'});
  await mount();fireEvent.click(await screen.findByText(/todos\/t1\/context.md:1:1/));
  await screen.findByLabelText('文件内容 todos/t1/context.md');replace('补齐需求背景');
  fireEvent.click(button('保存'));
  await waitFor(()=>expect(apiPost).toHaveBeenCalledWith('/api/todo/templates/demo/new-version',expect.objectContaining({files:expect.objectContaining({'todos/t1/context.md':'补齐需求背景','objective.md':files['objective.md']})})));
});

const node=path=>document.querySelector(`[data-file-path="${path}"]`);
const context=async(path,label)=>{
  if(path)await waitFor(()=>expect(node(path)).toBeTruthy());
  fireEvent.contextMenu(path ? node(path).closest('.ant-tree-node-content-wrapper') : document.querySelector('.file-workspace-tree'));
  fireEvent.click(await screen.findByText(label,{selector:'.ant-dropdown-menu-title-content'}));
};
// Create/rename now commit inline (VSCode style) inside the tree; delete keeps its dialog.
const nameEntry=async(kind,name)=>{
  const input=await screen.findByLabelText(new RegExp(`^(新增|重命名)${kind}名称$`));
  fireEvent.change(input,{target:{value:name}});fireEvent.keyDown(input,{key:'Enter',code:'Enter',keyCode:13});
};

it('keeps only back and save in the page toolbar',async()=>{
  await mount();
  expect([...document.querySelectorAll('.todo-directory-toolbar button')].map(b=>b.textContent.replace(/\s/g,''))).toEqual(['返回','保存']);
  expect(screen.queryByText(/父 Agent|执行 Agent|负责调度/)).toBeNull();
  expect(button('新增 TODO')).toBeUndefined();expect(button('校验文件')).toBeUndefined();
  expect(screen.queryByLabelText('模板环境')).toBeNull();
});

it('allows returning during loading and blocks duplicate saves and file operations while saving',async()=>{
  apiGet.mockImplementation(()=>new Promise(()=>{}));
  const close=vi.fn();const pending=render(<TodoEditor templateName="demo" version="v1" onClose={close}/>);
  expect(button('保存').disabled).toBe(true);fireEvent.click(button('返回'));expect(close).toHaveBeenCalledOnce();pending.unmount();
  apiGet.mockImplementation(async path=>path==='/api/agents'?{agents:[{name:'reviewer',primary:true}]}:{files:fixture(),revision:'r'});
  await mount();let validate;
  apiPost.mockImplementationOnce(()=>new Promise(resolve=>{validate=resolve;}));
  fireEvent.click(button('保存'));fireEvent.click(button('保存'));
  expect(apiPost).toHaveBeenCalledTimes(1);
  fireEvent.contextMenu(document.querySelector('.file-workspace-tree'));
  expect(document.querySelector('.ant-dropdown')).toBeNull();
  expect(editor().state.readOnly).toBe(true);
  await act(async()=>validate({valid:true}));
  await waitFor(()=>expect(apiPost).toHaveBeenCalledTimes(2));
});

it('creates, renames and deletes task directories from the right-clicked row',async()=>{
  await mount();
  await context('todos','新增目录');await nameEntry('目录','second');
  await screen.findByLabelText('文件内容 todos/second/task.json');
  await context('todos/t1','重命名目录');await nameEntry('目录','first');
  await waitFor(()=>expect(node('todos/first')).toBeTruthy());
  expect(node('todos/t1')).toBeNull();
  // The context target is first, while the editor still has second open.
  expect(screen.getByLabelText('文件内容 todos/second/task.json')).toBeTruthy();
  await context('todos/first','删除目录');fireEvent.click(button('删除'));
  await waitFor(()=>expect(node('todos/first')).toBeNull());
  expect(node('todos/second')).toBeTruthy();
  fireEvent.click(button('保存'));
  await waitFor(()=>expect(apiPost).toHaveBeenCalledWith('/api/todo/templates/demo/new-version',expect.objectContaining({files:expect.objectContaining({'todos/second/context.md':EXAMPLE_SPEC.todos[0].requirement_background})})));
  const saved=apiPost.mock.calls.find(([path])=>path.endsWith('new-version'))[1].files;
  expect(JSON.parse(saved['workflow.json']).todos).toEqual(['second']);
});

it('distinguishes file operations and keeps sibling files when a file is deleted',async()=>{
  await mount();
  await context('todos/t1/context.md','重命名文件');await nameEntry('文件','background.md');
  await screen.findByLabelText('文件内容 todos/t1/background.md');
  expect(editor().state.doc.toString()).toBe(EXAMPLE_SPEC.todos[0].requirement_background);
  await context('todos/t1/background.md','删除文件');fireEvent.click(button('删除'));
  await waitFor(()=>expect(node('todos/t1/background.md')).toBeNull());
  expect(node('todos/t1/task.json')).toBeTruthy();
  await context('todos/t1','新增文件');await nameEntry('文件','context.md');
  await screen.findByLabelText('文件内容 todos/t1/context.md');replace('右键补齐背景');
  fireEvent.click(button('保存'));
  await waitFor(()=>expect(apiPost).toHaveBeenCalledWith('/api/todo/templates/demo/new-version',expect.objectContaining({files:expect.objectContaining({'todos/t1/context.md':'右键补齐背景'})})));
});

it('supports empty directories at the root and validates them on save without losing the draft',async()=>{
  const onDirtyChange=vi.fn();await mount({onDirtyChange});
  await context('','新增目录');await nameEntry('目录','notes');
  await waitFor(()=>expect(node('notes')?.dataset.fileKind).toBe('directory'));
  expect(onDirtyChange).toHaveBeenLastCalledWith(true);
  fireEvent.click(button('保存'));
  expect(await screen.findByText('空目录无法保存，请添加 TODO 文件或删除此目录')).toBeTruthy();
  expect(apiPost).not.toHaveBeenCalled();fireEvent.click(button('继续修改'));
  await context('notes','重命名目录');await nameEntry('目录','archive');
  await waitFor(()=>expect(node('archive')).toBeTruthy());
  await context('archive','删除目录');fireEvent.click(button('删除'));
  await waitFor(()=>expect(node('archive')).toBeNull());
  expect(onDirtyChange).toHaveBeenLastCalledWith(false);
});

it('keeps invalid names inline for correction and never overwrites an existing file',async()=>{
  await mount();
  await context('','新增文件');await nameEntry('文件','workflow.json');
  expect(await screen.findByText('同名文件或目录已存在')).toBeTruthy();
  expect(screen.getByLabelText('新增文件名称').value).toBe('workflow.json');
  await nameEntry('文件','notes.md');await screen.findByLabelText('文件内容 notes.md');
  expect(node('workflow.json')).toBeTruthy();expect(editor().state.doc.toString()).toBe('');
});
