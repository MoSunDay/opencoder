import {expect,it} from 'vitest';
import {directoryPaths,fileTree} from '../../ui/files/format.js';
import {EXAMPLE_SPEC,changeTask,decodeFiles,setProperty,specFiles} from './model.js';
import {changeEntry,directoryProblems} from './operations.js';

const draft = () => ({files:specFiles(EXAMPLE_SPEC),directories:[],selected:'objective.md'});
const operation = (action,path,isDirectory) => ({action,path,isDirectory});

it('creates a task directory with complete files, then keeps dependencies in sync when renaming',()=>{
  const original=draft();
  let next=changeEntry(original,operation('create-directory','todos',true),'second');
  next.files['todos/second/task.json']=setProperty(next.files['todos/second/task.json'],'depends_on',['t1']);
  next=changeEntry(next,operation('rename','todos/t1',true),'first');
  expect(JSON.parse(next.files['workflow.json']).todos).toEqual(['first','second']);
  expect(JSON.parse(next.files['todos/second/task.json']).depends_on).toEqual(['first']);
  expect(decodeFiles(next.files,['act']).diagnostics).toEqual([]);
  expect(next.selected).toBe('todos/second/task.json');
  expect(original.files).toEqual(specFiles(EXAMPLE_SPEC));
  expect(()=>changeEntry(next,operation('delete','todos/first',true))).toThrow('仍依赖');
});

it('removes only the chosen file, while deleting a directory removes its descendants',()=>{
  const original=draft();original.files=changeTask(original.files,'add',null,'t10');
  let next=changeEntry(original,operation('delete','todos/t1/context.md',false));
  expect(next.files['todos/t1/task.json']).toBeDefined();
  expect(JSON.parse(next.files['workflow.json']).todos).toEqual(['t1','t10']);
  next=changeEntry(next,operation('delete','todos/t1',true));
  expect(Object.keys(next.files).some(path=>path.startsWith('todos/t1/'))).toBe(false);
  expect(next.files['todos/t10/context.md']).toBeDefined();
  expect(JSON.parse(next.files['workflow.json']).todos).toEqual(['t10']);
  expect(decodeFiles(next.files,['act']).diagnostics).toEqual([]);
});

it('preserves and recursively renames empty directories and the currently edited file',()=>{
  let next=changeEntry(draft(),operation('create-directory','',true),'notes');
  next=changeEntry(next,operation('create-directory','notes',true),'empty');
  next=changeEntry(next,operation('create-file','notes',true),'draft.md');
  next.files['notes/draft.md']='draft contents';
  next=changeEntry(next,operation('rename','notes',true),'archive');
  expect(next.selected).toBe('archive/draft.md');
  expect(next.files['archive/draft.md']).toBe('draft contents');
  expect(next.directories).toContain('archive/empty');
  expect(directoryProblems(next.files,next.directories)).toEqual([
    expect.objectContaining({path:'archive/empty',message:expect.stringContaining('空目录')}),
  ]);
  const tree=fileTree(Object.keys(next.files),[],[],next.directories);
  expect(tree.find(node=>node.key==='archive').children[0]).toMatchObject({key:'archive/empty',isLeaf:false,children:[]});
  next=changeEntry(next,operation('delete','archive',true));
  expect(next.selected).toBeNull();
  expect(next.files).toEqual(draft().files);
  expect(directoryPaths(next.files,next.directories)).toEqual(directoryPaths(draft().files));
});

it('creates siblings when invoked on a file and never overwrites existing files or directories',()=>{
  const original=draft();
  const next=changeEntry(original,operation('create-file','todos/t1/task.json',false),'extra.md');
  expect(next.selected).toBe('todos/t1/extra.md');
  expect(next.files['todos/t1/extra.md']).toBe('');
  for(const name of ['workflow.json','todos'])
    expect(()=>changeEntry(original,operation('create-directory','',true),name)).toThrow('已存在');
  expect(()=>changeEntry(original,operation('rename','todos/t1/task.json',false),'context.md')).toThrow('已存在');
  expect(()=>changeEntry(original,operation('rename','todos/t1/task.json',false),'nested/file')).toThrow('合法名称');
  for(const name of ['', '.', '..', '../escape', 'bad\\name', 'nested//file'])
    expect(()=>changeEntry(original,operation('create-file','',true),name)).toThrow('合法名称');
  expect(original.files).toEqual(specFiles(EXAMPLE_SPEC));
});

it('fills in intermediate directories for implicit nested create paths',()=>{
  const next=changeEntry(draft(),operation('create-file','',true),'docs/guide/readme.md');
  expect(next.files['docs/guide/readme.md']).toBe('');
  expect(next.selected).toBe('docs/guide/readme.md');
  expect(directoryPaths(next.files,next.directories)).toEqual(expect.arrayContaining(['docs','docs/guide']));
  expect(()=>changeEntry(draft(),operation('create-directory','',true),'workflow.json/sub'))
    .toThrow('同名文件或目录已存在');
  const task=changeEntry(draft(),operation('create-directory','todos',true),'a/b');
  expect(task.directories).toEqual(expect.arrayContaining(['todos/a','todos/a/b']));
  // Multi-segment paths under `todos` only draft folders; the task skeleton
  // stays a single-segment behaviour.
  expect(JSON.parse(task.files['workflow.json']).todos).toEqual(['t1']);
  expect(task.selected).toBe('objective.md');
  expect(directoryProblems(task.files,task.directories).map(problem=>problem.path))
    .toEqual(expect.arrayContaining(['todos/a','todos/a/b']));
});

it('allows repairing file names and rejects empty directories instead of losing them on save',()=>{
  let next=changeEntry(draft(),operation('rename','objective.md',false),'draft.md');
  expect(next.selected).toBe('draft.md');
  expect(decodeFiles(next.files).diagnostics.some(problem=>problem.path==='objective.md')).toBe(true);
  next=changeEntry(next,operation('rename','draft.md',false),'objective.md');
  expect(decodeFiles(next.files).diagnostics).toEqual([]);
  next=changeEntry(next,operation('create-directory','',true),'empty');
  expect(directoryProblems(next.files,next.directories).map(problem=>problem.path)).toEqual(['empty']);
});
