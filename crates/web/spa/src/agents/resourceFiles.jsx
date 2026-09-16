import {useEffect, useState} from 'react';
import {Alert, Button, Checkbox, Input, Modal, Space, Typography} from 'antd';
import {FileWorkspace} from '../ui/files/workspace.jsx';
import {FileEditor} from '../ui/files/editor.jsx';
import {b64EncodeText} from '../agentsItems.js';
import {firstReadable, moveFiles, putFile, readUpload, removeFiles, textContent, updateText} from './resourceModel.js';

export function ResourceFiles({cat,files,onChange,readOnly,onSave,onWorking}) {
  const [selected,setSelected] = useState(() => firstReadable(files));
  const [operation,setOperation] = useState(null); const [path,setPath] = useState('');
  const [directories,setDirectories] = useState([]);
  const [error,setError] = useState(''); const [uploading,setUploading] = useState(false);
  useEffect(() => { if (!files[selected]) setSelected(firstReadable(files)); },[files,selected]);
  const file = files[selected]; const text = file ? textContent(file) : '';
  const renameDraft = (from,to) => {
    setDirectories(previous => [...new Set(previous.map(dir => dir === from || dir.startsWith(`${from}/`) ? to + dir.slice(from.length) : dir))].sort());
    if (selected === from || selected.startsWith(`${from}/`)) setSelected(to + selected.slice(from.length));
  };
  const dropDraft = dropped => {
    if (dropped) setDirectories(previous => previous.filter(dir => dir !== dropped && !dir.startsWith(`${dropped}/`)));
  };
  // Inline drafts (create/rename from the tree) arrive with a name; the
  // toolbar buttons and delete keep using the dialog. Saves are files-only
  // PUTs: a new skill folder gets its mandatory SKILL.md skeleton, other
  // categories keep the empty folder as a local tree draft.
  const apply = operation => {
    const creating = operation.action.startsWith('create-');
    const parent = operation.action === 'rename'
      ? operation.path.split('/').slice(0,-1).join('/')
      : creating && operation.isDirectory ? operation.path : operation.path.split('/').slice(0,-1).join('/');
    const target = parent ? `${parent}/${operation.name}` : operation.name;
    try {
      if (operation.action === 'rename') {
        onChange(moveFiles(files,operation.path,target)); renameDraft(operation.path,target);
      } else if (operation.action === 'create-directory') {
        const segments = target.split('/').filter(Boolean);
        setDirectories(previous => [...new Set([...previous,...segments.map((_,index) => segments.slice(0,index + 1).join('/'))])].sort());
        if (cat === 'skills') {
          const next = putFile(files,{path:`${target}/SKILL.md`,content_b64:b64EncodeText(''),mode:0o600});
          onChange(next); setSelected(`${target}/SKILL.md`);
        }
      } else {
        const next = putFile(files,{path:target,content_b64:b64EncodeText(''),mode:0o600});
        onChange(next); setSelected(target);
      }
    } catch (e) { setError(e.message); }
  };
  const start = operation => {
    setError('');
    if (!operation.name) {
      setOperation(operation);
      const parent = operation.isDirectory ? operation.path : operation.path.split('/').slice(0,-1).join('/');
      setPath(operation.action === 'rename' ? operation.path : `${parent ? parent + '/' : ''}${cat === 'skills' && !parent ? 'new-skill/SKILL.md' : 'new-file.md'}`);
      return;
    }
    apply(operation);
  };
  const commit = () => {
    try {
      let next;
      if (operation.action === 'delete') {
        next = removeFiles(files,operation.path); dropDraft(operation.path);
      }
      else if (operation.action === 'rename') {
        next = moveFiles(files,operation.path,path); renameDraft(operation.path,path);
      }
      else next = putFile(files,{path,content_b64:b64EncodeText(''),mode:0o600});
      onChange(next); setSelected(operation.action === 'delete' ? firstReadable(next) : path); setOperation(null);
    } catch (e) { setError(e.message); }
  };
  const upload = async (event, replace) => {
    const uploads = Array.from(event.target.files || []); event.target.value = '';
    if (!uploads.length) return;
    setUploading(true); onWorking?.(true); setError('');
    try {
      let next = files;
      const parent = selected.split('/').slice(0,-1).join('/');
      for (const item of uploads) {
        const target = replace ? selected : (item.webkitRelativePath || `${parent ? parent + '/' : ''}${item.name}`);
        next = putFile(next,{path:target,content_b64:await readUpload(item),mode:replace ? file.mode : 0o600},replace);
      }
      onChange(next); setSelected(replace ? selected : firstReadable(next));
    } catch (e) { setError(e.message); }
    finally { setUploading(false); onWorking?.(false); }
  };
  // Memory is directory-shaped like the other pools: the same
  // FileWorkspace tree (multi-file, inline create/rename, upload). The
  // read side aggregates every `*.md` of the saved version dir.
  const display = Object.fromEntries(Object.entries(files).map(([path,file]) => [path,textContent(file) ?? '二进制文件，请下载查看或上传替换。']));
  return <div>
    {error && <Alert type="error" showIcon title={error}/>}
    <Space wrap style={{marginBottom:12}}>
      {!readOnly && <>
        <Button onClick={() => start({action:'create-file',path:'',isDirectory:true})}>新增文件</Button>
        <label>上传文件 <input aria-label="上传文件" type="file" multiple disabled={uploading} onChange={event => upload(event,false)}/></label>
        {cat === 'skills' && <label>上传技能目录 <input aria-label="上传技能目录" type="file" multiple webkitdirectory="" disabled={uploading} onChange={event => upload(event,false)}/></label>}
        {file && <>
          <Button onClick={() => start({action:'rename',path:selected})}>重命名</Button>
          <Button danger onClick={() => start({action:'delete',path:selected})}>移除</Button>
          <label>上传替换 <input aria-label="上传替换" type="file" disabled={uploading} onChange={event => upload(event,true)}/></label>
        </>}
      </>}
      {file && <a download={selected.split('/').at(-1)} href={`data:application/octet-stream;base64,${file.content_b64}`}>下载</a>}
      {file && <Typography.Text type="secondary">{selected} · {atob(file.content_b64).length} 字节 · {text === null ? '二进制' : '文本'} · 权限 {(file.mode ?? 0o600).toString(8)}</Typography.Text>}
      {cat === 'tools' && file && <Checkbox disabled={readOnly} checked={!!(file.mode & 0o111)}
        onChange={event => onChange({...files,[selected]:{...file,mode:event.target.checked ? file.mode | 0o100 : file.mode & ~0o111}})}>可执行</Checkbox>}
    </Space>
    <FileWorkspace files={display} directories={directories} selected={selected} onSelect={setSelected}
      readOnly={readOnly || text === null} onChange={(path,value) => onChange(updateText(files,path,value))}
      onSave={onSave} operationsReadOnly={readOnly} onOperation={readOnly ? undefined : start}/>
    {!Object.keys(files).length && <Typography.Text type="secondary">{readOnly ? '未配置。' : `未配置，可新增文件或上传内容后保存。${cat === 'skills' ? '技能目录必须包含 SKILL.md。' : ''}`}</Typography.Text>}
    <Modal open={!!operation} title={operation?.action === 'delete' ? '移除文件或目录' : operation?.action === 'rename' ? '重命名' : '新增文件'}
      onCancel={() => setOperation(null)} onOk={commit} okText="确认">
      {operation?.action === 'delete' ? <p>移除 {operation.path} 及其内容？保存后生效。</p>
        : <Input aria-label="文件路径" value={path} onChange={event => setPath(event.target.value)} placeholder="skill-name/SKILL.md"/>}
      {error && <Alert type="error" title={error}/>}
    </Modal>
  </div>;
}
