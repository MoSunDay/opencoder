// @vitest-environment jsdom
import '../test/setup-dom.js';
import {beforeEach,expect,it,vi} from 'vitest';
import {fireEvent,render,screen,waitFor} from '@testing-library/react';
import {strToU8,zipSync} from 'fflate';
import {ArchiveUpload} from './archiveUpload.jsx';
import {b64DecodeText} from '../agentsItems.js';

beforeEach(()=>{Range.prototype.getClientRects=()=>[];Range.prototype.getBoundingClientRect=()=>({left:0,right:0,top:0,bottom:0});});

const entry=(path,text,mode=0o600)=>({path,content_b64:btoa(text),mode});
const zipFile=tree=>new File([zipSync(tree)],'bundle.zip');
const mount=(files,cat='memory')=>{
  const refs={onMerge:vi.fn(),onError:vi.fn()};
  render(<ArchiveUpload cat={cat} files={files} disabled={false} onMerge={refs.onMerge} onError={refs.onError}/>);
  return refs;
};
it('imports a zip into the draft, overwriting same-name files and keeping the rest',async()=>{
  const {onMerge,onError}=mount({'note.md':entry('note.md','old'),'keep/deep.md':entry('keep/deep.md','deep')});
  fireEvent.change(document.querySelector('input[type=file]'),{target:{files:[zipFile({'note.md':strToU8('new'),'add/more.md':strToU8('extra')})]}});
  expect(await screen.findByText(/共 2 个文件：新增 1 个，覆盖同名 1 个/)).toBeTruthy();
  expect(screen.getByText(/只覆盖同名文件，不会删除或清空目录/)).toBeTruthy();
  fireEvent.click(screen.getByRole('button',{name:'覆盖上传'}));
  await waitFor(()=>expect(onMerge).toHaveBeenCalled());
  const next=onMerge.mock.calls[0][0];
  expect(b64DecodeText(next['note.md'].content_b64)).toBe('new');
  expect(next['keep/deep.md']).toEqual(entry('keep/deep.md','deep'));
  expect(next['add/more.md'].mode).toBe(0o600);
});

it('warns in the dialog before merging and never merges on cancel',async()=>{
  const {onMerge,onError}=mount({'note.md':entry('note.md','old')});
  fireEvent.change(document.querySelector('input[type=file]'),{target:{files:[zipFile({'note.md':strToU8('new')})]}});
  await screen.findByText(/覆盖同名 1 个/);
  fireEvent.click(screen.getByRole('button',{name:/取\s*消/}));
  expect(onMerge).not.toHaveBeenCalled();
});

it('rejects non-zip picks and skill packages without SKILL.md',async()=>{
  const {onMerge,onError}=mount({},'skills');
  fireEvent.change(document.querySelector('input[type=file]'),{target:{files:[new File([new Blob(['x'])],'note.txt')]}});
  await waitFor(()=>expect(onError).toHaveBeenCalledWith('请选择 .zip 压缩包'));
  fireEvent.change(document.querySelector('input[type=file]'),{target:{files:[new File([zipSync({'probe/x.md':strToU8('x')})],'bundle.zip')]}});
  await waitFor(()=>expect(onError).toHaveBeenCalledWith('技能包 probe 缺少 SKILL.md'));
  fireEvent.change(document.querySelector('input[type=file]'),{target:{files:[new File([zipSync({'probe/SKILL.md':strToU8('s'),'probe/x.md':strToU8('x')})],'bundle.zip')]}});
  fireEvent.click(await screen.findByRole('button',{name:'覆盖上传'}));
  await waitFor(()=>expect(onMerge).toHaveBeenCalled());
  expect(Object.keys(onMerge.mock.calls[0][0]).sort()).toEqual(['probe/SKILL.md','probe/x.md']);
});

it('round-trips binary bytes through the archive',async()=>{
  const {onMerge,onError}=mount({'run.sh':entry('run.sh','old',0o755)});
  const bytes=new Uint8Array([0,255,2]);
  fireEvent.change(document.querySelector('input[type=file]'),{target:{files:[zipFile({'run.sh':bytes})]}});
  await screen.findByText(/覆盖同名 1 个/);
  fireEvent.click(screen.getByRole('button',{name:'覆盖上传'}));
  await waitFor(()=>expect(onMerge).toHaveBeenCalled());
  const next=onMerge.mock.calls[0][0];
  expect(next['run.sh'].mode).toBe(0o755); // 覆盖同名文件沿用权限位
  expect([...atob(next['run.sh'].content_b64)].map(c=>c.charCodeAt(0))).toEqual([0,255,2]);
});

it('blocks the antd default upload so no background request is ever sent',async()=>{
  const open=vi.spyOn(XMLHttpRequest.prototype,'open');
  const send=vi.spyOn(XMLHttpRequest.prototype,'send');
  const fetchSpy=vi.spyOn(globalThis,'fetch').mockImplementation(()=>Promise.reject(new Error('fetch must not be used')));
  try {
    const {onMerge,onError}=mount({'a.md':entry('a.md','old')});
    fireEvent.change(document.querySelector('input[type=file]'),{target:{files:[zipFile({'a.md':strToU8('new')})]}});
    await screen.findByText(/覆盖同名 1 个/);
    expect(open).not.toHaveBeenCalled();
    expect(send).not.toHaveBeenCalled();
    expect(fetchSpy).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button',{name:'覆盖上传'}));
    await waitFor(()=>expect(onMerge).toHaveBeenCalled());
    expect(onError.mock.calls.every(call => call[0] === '')).toBe(true); // 只允许清空 Alert 的空串，无真实错误
  } finally { fetchSpy.mockRestore(); }
});
