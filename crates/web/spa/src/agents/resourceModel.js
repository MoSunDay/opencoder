import { b64EncodeText } from '../agentsItems.js';

export const CATEGORIES = [{cat:'prompts',label:'Prompt'}, {cat:'skills',label:'Skills'}, {cat:'tools',label:'Tools'}, {cat:'memory',label:'Memory'}];
export const PARTS = [{path:'soul.md',label:'Soul（人格底色）'}, {path:'how.md',label:'How（工作方法）'}, {path:'output.md',label:'Output（产出契约）'}];
export const resourceUrl = (name, cat) => `/api/agents/${encodeURIComponent(name)}/resources/${cat}`;
export function textContent(file) {
  if (!file) return '';
  const bytes = Uint8Array.from(atob(file.content_b64), c => c.charCodeAt(0));
  if (bytes.includes(0)) return null;
  try { return new TextDecoder('utf-8', {fatal:true}).decode(bytes); } catch { return null; }
}
export function snapshotFiles(view) {
  if (!view?.baseline || !Number.isInteger(view.baseline.version) || typeof view.baseline.revision !== 'string' || (view.baseline.resource !== null && typeof view.baseline.resource !== 'string') || !Array.isArray(view.files) || !Array.isArray(view.versions) || typeof view.read_only !== 'boolean') throw new Error('资源响应不完整');
  const files = {};
  for (const file of view.files) {
    if (typeof file.content_b64 !== 'string' || Object.hasOwn(files, file.path)) throw new Error('文件响应无效');
    validatePath(file.path); textContent(file); files[file.path] = file;
  }
  return files;
}
export function updateText(files, path, text) {
  return {...files, [path]:{...files[path], path, content_b64:b64EncodeText(text), mode:files[path]?.mode ?? 0o600}};
}
export function fileChanges(original, draft) {
  return {
    files:Object.values(draft).filter(file => !original[file.path] || file.content_b64 !== original[file.path].content_b64 || file.mode !== original[file.path].mode),
    removed:Object.keys(original).filter(path => !Object.hasOwn(draft, path)),
  };
}
export function isDirty(entry) {
  if (!entry?.original || !entry?.draft) return false;
  const changes = fileChanges(entry.original, entry.draft);
  return !!(changes.files.length || changes.removed.length);
}
export function validatePath(path) {
  if (!path || /[\\\0]/.test(path) || path.split('/').length > 64 || path.split('/').some(p => !p || p === '.' || p === '..')) throw new Error('请输入有效的相对文件路径');
}
export function putFile(files, file, replace = false) {
  validatePath(file.path);
  if (!replace && Object.hasOwn(files,file.path)) throw new Error('文件已存在');
  if (Object.keys(files).some(path => path.startsWith(file.path + '/') || file.path.startsWith(path + '/'))) throw new Error('文件与目录路径冲突');
  return {...files, [file.path]:file};
}
export function moveFiles(files, from, to) {
  validatePath(to);
  if (from === to) return files;
  if (to.startsWith(from + '/')) throw new Error('不能移动到自身目录内');
  const moving = Object.keys(files).filter(path => path === from || path.startsWith(from + '/'));
  let next = Object.fromEntries(Object.entries(files).filter(([path]) => !moving.includes(path)));
  for (const path of moving) next = putFile(next, {...files[path], path:to + path.slice(from.length)});
  return next;
}
export function removeFiles(files, path) {
  return Object.fromEntries(Object.entries(files).filter(([p]) => p !== path && !p.startsWith(path + '/')));
}
export function firstReadable(files) {
  const paths = Object.keys(files).sort();
  return paths.find(path => textContent(files[path]) !== null) || paths[0] || '';
}
export function readUpload(file) {
  if (file.size > 1536 * 1024) return Promise.reject(new Error('文件超过 1.5 MiB'));
  return new Promise((resolve,reject) => {
    const reader = new FileReader(); reader.onload = () => resolve(String(reader.result).split(',')[1]);
    reader.onerror = () => reject(new Error('读取上传文件失败')); reader.readAsDataURL(file);
  });
}
