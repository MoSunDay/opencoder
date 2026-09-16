import {unzipSync} from 'fflate';
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

// ── 压缩包（.zip）覆盖上传 ─────────────────────────────────────────────
// 资源路径契约禁止隐藏段（`safe_rel_path` 同规则），而 zip 树几乎总带系统
// 元数据噪音（`__MACOSX/`、`.DS_Store`、`._` resource fork）：这类条目按
// 计数跳过而不是让整个导入失败；其余不安全路径一律丢弃并计数。
function archiveEntryPath(raw) {
  const path = String(raw).replace(/\\/g, '/').replace(/^\.\//, '');
  const segments = path.split('/');
  if (!path || segments.length > 64) return null;
  if (segments.some(segment => !segment || segment === '.' || segment === '..' || segment.startsWith('.'))) return null;
  if (segments[0] === '__MACOSX' || segments.some(segment => segment.startsWith('._'))) return null;
  return path;
}

export function bytesToB64(bytes) {
  let bin = '';
  for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(bin);
}

export async function unzipArchive(archive) {
  const entries = unzipSync(new Uint8Array(await archive.arrayBuffer()));
  const files = []; let skipped = 0;
  for (const [raw, bytes] of Object.entries(entries)) {
    if (String(raw).endsWith('/')) continue; // 目录标记条目：静默丢弃，不计入 skipped
    const path = archiveEntryPath(raw);
    if (path) files.push({path, content_b64: bytesToB64(bytes)});
    else skipped++;
  }
  return {files, skipped};
}

// 覆盖上传语义：同名文件直接覆盖（只覆盖文件，不动目录），目录中原有的其他
// 文件全部保留；覆盖已存在文件时沿用其权限位，新文件 0o600。合并集与后端
// `merge_files` 同规：skills 包必须带 SKILL.md、总量 ≤1.5 MiB / ≤4096 文件。
export function mergeArchive(cat, files, incoming) {
  let next = files;
  for (const file of incoming) next = putFile(next, {...file, mode: files[file.path]?.mode ?? 0o600}, true);
  if (cat === 'skills') {
    const paths = new Set(Object.keys(next));
    for (const path of Object.keys(next)) {
      const skill = path.split('/')[0];
      if (path.includes('/') && !paths.has(`${skill}/SKILL.md`)) throw new Error(`技能包 ${skill} 缺少 SKILL.md`);
    }
  }
  const total = Object.values(next).reduce((sum, file) => sum + atob(file.content_b64).length, 0);
  if (Object.keys(next).length > 4096 || total > 1536 * 1024) throw new Error('资源超过 1.5 MiB 或 4096 个文件上限');
  return next;
}
