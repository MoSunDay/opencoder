import {directoryPaths} from '../../ui/files/format.js';
import {changeTask, setProperty} from './model.js';

const within = (path, directory) => path === directory || path.startsWith(`${directory}/`);
export const parentPath = path => path.split('/').slice(0,-1).join('/');

// Validate an entry name and split it into path segments. `multi` accepts
// implicit nested paths (`a/b/c.md`) so inline creation can fill in
// intermediate directories; rename stays single-segment.
function entrySegments(name, {multi = false} = {}) {
  const value = name.trim();
  if (!value) throw new Error('请输入合法名称，不能为空');
  if (!multi && /[\\/]/.test(value)) throw new Error('请输入合法名称，不能包含路径分隔符，也不能使用 . 或 ..');
  return value.split('/').map(part => {
    if (!part.trim() || part.trim() === '.' || part.trim() === '..' || /[\\\0]/.test(part.trim()) || new TextEncoder().encode(part.trim()).length > 255)
      throw new Error('请输入合法名称，不能包含路径分隔符，也不能使用 . 或 ..');
    return part.trim();
  });
}

// Apply a draft mutation atomically. Task folders also maintain the workflow graph.
export function changeEntry({files, directories = [], selected}, operation, name = '') {
  const {action,path,isDirectory} = operation;
  const folders = directoryPaths(files,directories);
  const creating = action === 'create-file' || action === 'create-directory';
  const parent = creating && isDirectory ? path : parentPath(path);
  if (path && !(isDirectory ? folders.includes(path) : Object.hasOwn(files,path))) throw new Error('文件或目录已不存在');
  if (!creating && !path) throw new Error('不能修改工作区根目录');
  const segments = action === 'delete' ? [] : entrySegments(name,{multi:creating});
  const nextPath = action === 'delete' ? '' : [parent,...segments].filter(Boolean).join('/');
  if (action === 'rename' && path === nextPath) return {files,directories,selected};
  // Every segment of an implicit nested path must be free, not just the leaf.
  const targets = segments.map((_,index) => [parent,...segments.slice(0,index + 1)].filter(Boolean).join('/'));
  if (targets.some(target => Object.hasOwn(files,target) || folders.includes(target))) throw new Error('同名文件或目录已存在');
  let nextFiles = files; let nextFolders = folders; let nextSelected = selected;
  if (creating) {
    if (action === 'create-file') {
      nextFiles = {...files,[nextPath]:''};nextSelected = nextPath;
    } else {
      // Implicit nested paths (`a/b`) need their intermediate folders drafted too.
      const middle = targets.slice(0,-1);
      nextFolders = [...folders,...middle,nextPath];
      // A single new folder directly under `todos` starts a task skeleton.
      if (parent === 'todos' && segments.length === 1) {
        nextFiles = changeTask(files,'add',null,name.trim());
        nextSelected = `${nextPath}/task.json`;
      }
    }
  } else if (action === 'rename' || action === 'delete') {
    const affected = entry => isDirectory ? within(entry,path) : entry === path;
    const renamed = entry => nextPath + entry.slice(path.length);
    const taskId = isDirectory && /^todos\/([^/]+)$/.exec(path)?.[1];
    if (taskId && JSON.parse(files['workflow.json']).todos?.includes(taskId)) {
      nextFiles = changeTask(files,action,taskId,name.trim());
    } else {
      nextFiles = Object.fromEntries(Object.entries(files).flatMap(([entry,text]) =>
        !affected(entry) ? [[entry,text]] : action === 'rename' ? [[renamed(entry),text]] : []));
      if (isDirectory && path === 'todos' && action === 'delete')
        nextFiles['workflow.json'] = setProperty(files['workflow.json'],'todos',[]);
    }
    nextFolders = folders.flatMap(entry => !affected(entry) ? [entry] : action === 'rename' ? [renamed(entry)] : []);
    if (selected && affected(selected)) nextSelected = action === 'rename' ? renamed(selected) : null;
  } else throw new Error('不支持的文件操作');
  return {files:nextFiles,directories:nextFolders,selected:nextSelected};
}

// Empty draft folders must not silently disappear when saving the file-only API.
export function directoryProblems(files, directories) {
  return directoryPaths(files,directories)
    .filter(path => !Object.keys(files).some(file => file.startsWith(`${path}/`)))
    .map(path => ({path,message:'空目录无法保存，请添加 TODO 文件或删除此目录',line:1,column:1}));
}
