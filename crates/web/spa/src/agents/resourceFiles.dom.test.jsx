// @vitest-environment jsdom
import '../test/setup-dom.js';
import {beforeEach,expect,it,vi} from 'vitest';
import {fireEvent,render,screen,waitFor} from '@testing-library/react';
import {ResourceFiles} from './resourceFiles.jsx';
import {b64EncodeText} from '../agentsItems.js';

beforeEach(()=>{Range.prototype.getClientRects=()=>[];Range.prototype.getBoundingClientRect=()=>({left:0,right:0,top:0,bottom:0});});

const entry=(path,text='x')=>({path,content_b64:b64EncodeText(text),mode:0o600});
const node=path=>document.querySelector(`[data-file-path="${path}"]`);
const contextMenu=async(path,label)=>{
  await waitFor(()=>expect(node(path)).toBeTruthy());
  fireEvent.contextMenu(node(path).closest('.ant-tree-node-content-wrapper'));
  fireEvent.click(await screen.findByText(label,{selector:'.ant-dropdown-menu-title-content'}));
};
const type=(input,text)=>fireEvent.change(input,{target:{value:text}});
const commit=input=>fireEvent.keyDown(input,{key:'Enter',code:'Enter',keyCode:13});
const mount=(files,cat='tools')=>{
  const onChange=vi.fn();
  render(<ResourceFiles cat={cat} files={files} onChange={onChange} onSave={()=>{}}/>);
  return onChange;
};

it('creates an agent resource file inline with an implicit nested path',async()=>{
  const onChange=mount({'a.md':entry('a.md')});
  await contextMenu('a.md','新增文件');
  const input=await screen.findByLabelText('新增文件名称');
  type(input,'guides/intro.md');commit(input);
  await waitFor(()=>expect(onChange).toHaveBeenCalled());
  const next=onChange.mock.calls[0][0];
  expect(next['guides/intro.md']).toEqual({path:'guides/intro.md',content_b64:b64EncodeText(''),mode:0o600});
});

it('drafts a skill folder inline together with its mandatory SKILL.md skeleton',async()=>{
  const onChange=mount({'pkg/SKILL.md':entry('pkg/SKILL.md')},'skills');
  await contextMenu('pkg','新增目录');
  const input=await screen.findByLabelText('新增目录名称');
  type(input,'other');commit(input);
  await waitFor(()=>expect(onChange).toHaveBeenCalled());
  const next=onChange.mock.calls[0][0];
  expect(next['pkg/other/SKILL.md']).toEqual({path:'pkg/other/SKILL.md',content_b64:b64EncodeText(''),mode:0o600});
  expect(await screen.findByText('other')).toBeTruthy();
});

it('keeps an empty tools folder as a draft directory without touching files',async()=>{
  const onChange=mount({'a.md':entry('a.md')});
  await contextMenu('a.md','新增目录');
  const input=await screen.findByLabelText('新增目录名称');
  type(input,'buckets');commit(input);
  expect(await screen.findByText('buckets')).toBeTruthy();
  expect(onChange).not.toHaveBeenCalled();
});

it('renames an agent resource file inline and moves its content',async()=>{
  const onChange=mount({'a.md':entry('a.md'),'b.md':entry('b.md')});
  await contextMenu('a.md','重命名文件');
  const input=await screen.findByLabelText('重命名文件名称');
  type(input,'renamed.md');commit(input);
  await waitFor(()=>expect(onChange).toHaveBeenCalled());
  const next=onChange.mock.calls[0][0];
  expect(next['renamed.md']).toEqual({path:'renamed.md',content_b64:b64EncodeText('x'),mode:0o600});
  expect(next['a.md']).toBeUndefined();
});

it('refuses inline rename onto an existing resource name without losing the draft',async()=>{
  const onChange=mount({'a.md':entry('a.md'),'b.md':entry('b.md')});
  await contextMenu('a.md','重命名文件');
  const input=await screen.findByLabelText('重命名文件名称');
  type(input,'b.md');
  expect(input.className).toContain('ant-input-status-error');
  commit(input);
  expect(onChange).not.toHaveBeenCalled();
  expect(screen.getByLabelText('重命名文件名称')).toBeTruthy();
});

it('keeps delete behind the confirmation dialog for agent resources',async()=>{
  const onChange=mount({'a.md':entry('a.md')});
  await contextMenu('a.md','删除文件');
  fireEvent.click([...document.querySelectorAll('button')].find(b=>b.textContent.replace(/\s/g,'')==='确认'));
  await waitFor(()=>expect(onChange).toHaveBeenCalled());
  expect(onChange.mock.calls[0][0]).toEqual({});
});

it('shows a multi-file memory pool as a tree and creates a file inline',async()=>{
  const onChange=mount({'memory.md':entry('memory.md'),'topics/rust.md':entry('topics/rust.md')},'memory');
  await waitFor(()=>expect(node('memory.md')).toBeTruthy());
  expect(await screen.findByText('rust.md')).toBeTruthy();
  await contextMenu('topics/rust.md','新增文件');
  const input=await screen.findByLabelText('新增文件名称');
  type(input,'vim.md');commit(input);
  await waitFor(()=>expect(onChange).toHaveBeenCalled());
  const next=onChange.mock.calls[0][0];
  expect(next['topics/vim.md']).toEqual({path:'topics/vim.md',content_b64:b64EncodeText(''),mode:0o600});
  expect(next['memory.md']).toEqual(entry('memory.md'));
  expect(next['topics/rust.md']).toEqual(entry('topics/rust.md'));
});

it('leaves structure operations to the tree and offers no single-file upload',async()=>{
  mount({'memory.md':entry('memory.md')},'memory');
  expect(document.querySelector('input[type=file]')).toBeNull(); // 上传收敛到保存行「上传压缩包」
  expect(screen.queryByRole('button',{name:'新增文件'})).toBeNull();
  expect(screen.queryByRole('button',{name:'重命名'})).toBeNull();
  expect(screen.queryByRole('button',{name:'移除'})).toBeNull();
  expect(screen.getByText('memory.md · 1 字节 · 文本 · 权限 600')).toBeTruthy();
  expect(screen.getByText('下载')).toBeTruthy();
});
