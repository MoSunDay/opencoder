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
    if(path==='/api/agents')return {agents:[{name:'act',primary:true}]};
    if(path==='/api/todo/envs')return {envs:[]};
    return {files:fixture(),revision:'revision-1',diagnostics:[]};
  });
  apiPost.mockReset().mockImplementation(async path=>path.endsWith('validate-files')?{valid:true}:{version:'v2',revision:'revision-2'});
});
const mount=async props=>{const result=render(<TodoEditor templateName="demo" version="v1" onClose={()=>{}} {...props}/>);await screen.findByLabelText('文件内容 objective.md');return result;};
it('saves a complete directory as a new version and retains the current draft',async()=>{
  await mount();replace('# 新目标\n保留上下文');fireEvent.click(button('保存新版本'));
  await waitFor(()=>expect(apiPost).toHaveBeenCalledWith('/api/todo/templates/demo/new-version',expect.objectContaining({source_version:'v1',expected_revision:'revision-1',files:expect.objectContaining({'objective.md':'# 新目标\n保留上下文'})})));
  expect(await screen.findByText('demo / v2')).toBeTruthy();expect(editor().state.doc.toString()).toBe('# 新目标\n保留上下文');
});
it('preserves invalid JSON while switching files and blocks save with a locating modal',async()=>{
  await mount();fireEvent.click(screen.getByText('workflow.json',{exact:true}));
  await screen.findByLabelText('文件内容 workflow.json');replace('{\n "broken": }');
  fireEvent.click(document.querySelector('[data-file-path="objective.md"]'));await screen.findByLabelText('文件内容 objective.md');
  fireEvent.click(button('保存新版本'));expect(await screen.findByText('文件不符合 TODO 框架要求')).toBeTruthy();expect(apiPost).not.toHaveBeenCalled();
  fireEvent.click(screen.getByText(/workflow.json:2:/));await screen.findByLabelText('文件内容 workflow.json');expect(editor().state.doc.toString()).toBe('{\n "broken": }');
});
it('shows framework diagnostics immediately when an existing file is invalid',async()=>{
  const files=fixture();files['todos/t1/context.md']='';
  apiGet.mockImplementation(async path=>path==='/api/agents'?{agents:[{name:'act',primary:true}]}:path==='/api/todo/envs'?{envs:[]}:{files,revision:'r'});
  await mount();expect(await screen.findByText('文件不符合 TODO 框架要求')).toBeTruthy();expect(screen.getByText(/todos\/t1\/context.md:1:1/)).toBeTruthy();
});
it('shows server validation errors and keeps edited contents on save failure',async()=>{
  await mount();replace('server validation example');
  apiPost.mockRejectedValueOnce(Object.assign(new Error('invalid'),{status:400,body:{diagnostics:[{path:'env.json',message:'环境不存在',line:1,column:1}]}}));
  fireEvent.click(button('保存新版本'));expect(await screen.findByText('环境不存在')).toBeTruthy();expect(editor().state.doc.toString()).toBe('server validation example');expect(apiPost).toHaveBeenCalledTimes(1);
});
it('Markdown supports source and sanitized preview without changing saved text',async()=>{
  await mount();replace('# 标题\n<script>window.bad=true</script>\n**内容**');fireEvent.click(screen.getByText('预览',{exact:true}));
  expect(await screen.findByRole('heading',{name:'标题'})).toBeTruthy();expect(document.querySelector('.file-editor-preview script')).toBeNull();
  fireEvent.click(screen.getByText('源码',{exact:true}));expect(editor().state.doc.toString()).toContain('<script>');
});
it('missing files can be repaired from the diagnostic without discarding other files',async()=>{
  const files=fixture();delete files['todos/t1/context.md'];
  apiGet.mockImplementation(async path=>path==='/api/agents'?{agents:[{name:'act',primary:true}]}:path==='/api/todo/envs'?{envs:[]}:{files,revision:'r'});
  await mount();fireEvent.click(await screen.findByText(/todos\/t1\/context.md:1:1/));
  await screen.findByLabelText('文件内容 todos/t1/context.md');replace('补齐需求背景');
  fireEvent.click(button('保存新版本'));
  await waitFor(()=>expect(apiPost).toHaveBeenCalledWith('/api/todo/templates/demo/new-version',expect.objectContaining({files:expect.objectContaining({'todos/t1/context.md':'补齐需求背景','objective.md':files['objective.md']})})));
});
