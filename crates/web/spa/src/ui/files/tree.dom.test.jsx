// @vitest-environment jsdom
import '../../test/setup-dom.js';
import {beforeEach,expect,it,vi} from 'vitest';
import {fireEvent,render,screen,waitFor} from '@testing-library/react';
import {FileWorkspace} from './workspace.jsx';

beforeEach(()=>{Range.prototype.getClientRects=()=>[];Range.prototype.getBoundingClientRect=()=>({left:0,right:0,top:0,bottom:0});});

const node=path=>document.querySelector(`[data-file-path="${path}"]`);
const contextMenu=async(path,label)=>{
  if(path)await waitFor(()=>expect(node(path)).toBeTruthy());
  fireEvent.contextMenu(path ? node(path).closest('.ant-tree-node-content-wrapper') : document.querySelector('.file-workspace-tree'));
  fireEvent.click(await screen.findByText(label,{selector:'.ant-dropdown-menu-title-content'}));
};
const type=(input,text)=>fireEvent.change(input,{target:{value:text}});
const commit=input=>fireEvent.keyDown(input,{key:'Enter',code:'Enter',keyCode:13});
const cancel=input=>fireEvent.keyDown(input,{key:'Escape',code:'Escape',keyCode:27});
const mount=(props={})=>{
  const operation=vi.fn();
  const view=render(<FileWorkspace files={{'a.md':'alpha','todos/t1/task.json':'{}'}} selected="a.md"
    onSelect={()=>{}} onOperation={operation} {...props}/>);
  return {operation,view};
};

it('creates a nested file inline from the context menu without a dialog',async()=>{
  const {operation}=mount();
  await contextMenu('a.md','新增文件');
  const input=await screen.findByLabelText('新增文件名称');
  expect(input.placeholder).toContain('docs/guide.md');
  expect(input.closest('.file-workspace-tree')).toBeTruthy();
  expect(document.querySelector('.ant-modal-root')).toBeNull();
  type(input,'docs/guide.md');commit(input);
  await waitFor(()=>expect(operation).toHaveBeenCalledWith({action:'create-file',path:'a.md',isDirectory:false,name:'docs/guide.md'}));
  expect(screen.queryByLabelText('新增文件名称')).toBeNull();
});

it('creates a directory inline from a right-clicked directory with a directory placeholder',async()=>{
  const {operation}=mount();
  await contextMenu('todos','新增目录');
  const input=await screen.findByLabelText('新增目录名称');
  expect(input.placeholder).toContain('docs/guide');
  type(input,'notes/archive');commit(input);
  expect(operation).toHaveBeenCalledWith({action:'create-directory',path:'todos',isDirectory:true,name:'notes/archive'});
});

it('rejects a duplicate sibling name inline with an error style and keeps the draft',async()=>{
  const {operation}=mount();
  await contextMenu('a.md','新增文件');
  const input=await screen.findByLabelText('新增文件名称');
  type(input,'a.md');
  expect(input.className).toContain('ant-input-status-error');
  expect(screen.getByText('同名文件或目录已存在')).toBeTruthy();
  commit(input);
  expect(operation).not.toHaveBeenCalled();
  expect(screen.getByLabelText('新增文件名称')).toBeTruthy();
});

it('rejects illegal inline names such as .. and backslashes',async()=>{
  const {operation}=mount();
  await contextMenu('','新增文件');
  const input=await screen.findByLabelText('新增文件名称');
  for (const bad of ['a\\b.md','..','.']) {
    type(input,bad);
    expect(input.className).toContain('ant-input-status-error');
    commit(input);
    expect(operation).not.toHaveBeenCalled();
  }
  expect(screen.getByLabelText('新增文件名称')).toBeTruthy();
});

it('renames inline with the original name preselected and commits the new name',async()=>{
  const {operation}=mount();
  await contextMenu('a.md','重命名文件');
  const input=await screen.findByLabelText('重命名文件名称');
  expect(input.value).toBe('a.md');
  expect(input.selectionStart).toBe(0);expect(input.selectionEnd).toBe('a.md'.length);
  type(input,'b.md');commit(input);
  expect(operation).toHaveBeenCalledWith({action:'rename',path:'a.md',isDirectory:false,name:'b.md'});
  expect(screen.queryByLabelText('重命名文件名称')).toBeNull();
});

it('reports the directory kind when renaming a folder inline',async()=>{
  const {operation}=mount();
  await contextMenu('todos','重命名目录');
  const input=await screen.findByLabelText('重命名目录名称');
  expect(input.value).toBe('todos');
  type(input,'docs');commit(input);
  expect(operation).toHaveBeenCalledWith({action:'rename',path:'todos',isDirectory:true,name:'docs'});
});

it('refuses renaming onto an existing sibling name but allows correcting the draft',async()=>{
  const {operation}=mount({files:{'a.md':'alpha','b.md':'beta','todos/t1/task.json':'{}'}});
  await contextMenu('a.md','重命名文件');
  const input=await screen.findByLabelText('重命名文件名称');
  type(input,'b.md');
  expect(input.className).toContain('ant-input-status-error');
  commit(input);
  expect(operation).not.toHaveBeenCalled();
  expect(screen.getByLabelText('重命名文件名称')).toBeTruthy();
  type(input,'notes.md');commit(input);
  expect(operation).toHaveBeenCalledWith({action:'rename',path:'a.md',isDirectory:false,name:'notes.md'});
});

it('cancels inline create on Escape or blur without calling back',async()=>{
  const {operation}=mount();
  await contextMenu('','新增文件');
  const input=await screen.findByLabelText('新增文件名称');
  type(input,'draft.md');
  cancel(input);
  expect(screen.queryByLabelText('新增文件名称')).toBeNull();
  expect(operation).not.toHaveBeenCalled();
  await contextMenu('','新增文件');
  const second=await screen.findByLabelText('新增文件名称');
  type(second,'draft2.md');
  fireEvent.blur(second);
  expect(screen.queryByLabelText('新增文件名称')).toBeNull();
  expect(operation).not.toHaveBeenCalled();
});

it('cancels inline rename on Escape keeping the original entry untouched',async()=>{
  const {operation}=mount();
  await contextMenu('todos','重命名目录');
  const input=await screen.findByLabelText('重命名目录名称');
  type(input,'renamed');
  cancel(input);
  expect(screen.queryByLabelText('重命名目录名称')).toBeNull();
  expect(operation).not.toHaveBeenCalled();
  await waitFor(()=>expect(node('todos')).toBeTruthy());
});

it('keeps the create-directory entry only when the workspace allows directories',async()=>{
  mount({allowCreateDirectory:false});
  fireEvent.contextMenu(node('a.md').closest('.ant-tree-node-content-wrapper'));
  expect(await screen.findByText('新增文件',{selector:'.ant-dropdown-menu-title-content'})).toBeTruthy();
  expect(screen.queryByText('新增目录',{selector:'.ant-dropdown-menu-title-content'})).toBeNull();
});
