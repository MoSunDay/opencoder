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
