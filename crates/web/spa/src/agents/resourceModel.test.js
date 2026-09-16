import {expect,it} from 'vitest';
import {fileChanges,moveFiles,putFile,snapshotFiles,textContent,updateText,validatePath} from './resourceModel.js';
it('renames entire skill packages preserving binary bytes and modes',()=>{
  const files={'s/SKILL.md':{path:'s/SKILL.md',content_b64:btoa('skill'),mode:0o640},'s/a.bin':{path:'s/a.bin',content_b64:'AP8=',mode:0o751}};
  const moved=moveFiles(files,'s','next');
  expect(moved['next/a.bin']).toEqual({...files['s/a.bin'],path:'next/a.bin'});
  expect(fileChanges(files,moved).removed).toEqual(['s/SKILL.md','s/a.bin']);
  expect(textContent(moved['next/a.bin'])).toBeNull();
  expect(()=>moveFiles(files,'s','s/nested')).toThrow();
});
it('rejects malformed responses, unsafe paths and colliding files',()=>{
  expect(()=>snapshotFiles({})).toThrow();
  for(const path of ['../x','a//b','/a','a/./b','a\\b'])expect(()=>validatePath(path)).toThrow();
  const files=updateText({},'x','content');expect(()=>putFile(files,{path:'x/y'})).toThrow();
  expect(()=>putFile(files,{path:'x'})).toThrow();
});
import {strToU8,zipSync} from 'fflate';
import {mergeArchive,unzipArchive} from './resourceModel.js';
it('merges archive entries by overwriting same-name files only, keeping other files and directories',()=>{
  const files={'probe/SKILL.md':{path:'probe/SKILL.md',content_b64:btoa('old'),mode:0o640},'probe/keep.md':{path:'probe/keep.md',content_b64:btoa('keep'),mode:0o600},'note.md':{path:'note.md',content_b64:btoa('note'),mode:0o600}};
  const next=mergeArchive('memory',files,[{path:'note.md',content_b64:btoa('new')},{path:'extra/new.md',content_b64:btoa('x')}]);
  expect(next['note.md'].content_b64).toBe(btoa('new'));
  expect(next['note.md'].mode).toBe(0o600); // 覆盖同名文件沿用权限位
  expect(next['probe/keep.md']).toEqual(files['probe/keep.md']);
  expect(next['extra/new.md'].mode).toBe(0o600);
});
it('enforces the same merged-set rules as the backend save',()=>{
  const files={'probe/SKILL.md':{path:'probe/SKILL.md',content_b64:btoa('old'),mode:0o600},'note.md':{path:'note.md',content_b64:btoa('note'),mode:0o600}};
  expect(()=>mergeArchive('skills',files,[{path:'extra/new.md',content_b64:btoa('x')}])).toThrow(/SKILL\.md/);
  expect(()=>mergeArchive('skills',files,[{path:'note.md/x',content_b64:btoa('x')}])).toThrow(/冲突/);
  const big=btoa('a'.repeat(1536*1024));
  expect(()=>mergeArchive('memory',{},[{path:'a.md',content_b64:big},{path:'b.md',content_b64:big}])).toThrow(/1\.5 MiB/);
  expect(mergeArchive('skills',files,[{path:'extra/SKILL.md',content_b64:btoa('s')}])['extra/SKILL.md']).toBeTruthy();
});

it('unzips archives, skipping directories and OS metadata entries',async()=>{
  const zip=zipSync({'probe/SKILL.md':strToU8('skill'),'probe/.DS_Store':strToU8('junk'),'__MACOSX/probe/._x':strToU8('meta'),'empty/':new Uint8Array()});
  const {files,skipped}=await unzipArchive(new Blob([zip]));
  expect(files.map(f=>f.path)).toEqual(['probe/SKILL.md']);
  expect(atob(files[0].content_b64)).toBe('skill');
  expect(skipped).toBe(2);
});
