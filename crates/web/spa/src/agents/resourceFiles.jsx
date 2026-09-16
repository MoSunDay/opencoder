import {useEffect, useState} from 'react';
import {Alert, Checkbox, Modal, Space, Typography} from 'antd';
import {FileWorkspace} from '../ui/files/workspace.jsx';
import {b64EncodeText} from '../agentsItems.js';
import {firstReadable, moveFiles, putFile, removeFiles, textContent, updateText} from './resourceModel.js';

// Skills / Memory / Tools 的文件区：结构操作（新增文件/目录、重命名、删除）
// 全部收敛到目录树右键菜单；单文件上传/替换已移除，整包导入走保存按钮右侧
// 的「上传压缩包」（覆盖上传）。这里只保留文件级下载 / 信息 / 可执行位。
export function ResourceFiles({cat,files,onChange,readOnly,onSave}) {
  const [selected,setSelected] = useState(() => firstReadable(files));
  const [confirming,setConfirming] = useState(null); const [directories,setDirectories] = useState([]);
  const [error,setError] = useState('');
  useEffect(() => { if (!files[selected]) setSelected(firstReadable(files)); },[files,selected]);
  const file = files[selected]; const text = file ? textContent(file) : '';
  const renameDraft = (from,to) => {
    setDirectories(previous => [...new Set(previous.map(dir => dir === from || dir.startsWith(`${from}/`) ? to + dir.slice(from.length) : dir))].sort());
    if (selected === from || selected.startsWith(`${from}/`)) setSelected(to + selected.slice(from.length));
  };
  const dropDraft = dropped => {
    if (dropped) setDirectories(previous => previous.filter(dir => dir !== dropped && !dir.startsWith(`${dropped}/`)));
  };
  // 目录树右键的内联草稿（create/rename）一定带 name，直接落草稿；不带
  // name 的调用只有 delete，走确认弹窗。保存是 files-only PUT：新技能目录
  // 由这里补 SKILL.md 骨架，其余类别保留空目录草稿。
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
    if (operation.name) return apply(operation);
    setError('');
    setConfirming(operation.path);
  };
  const commit = () => {
    const next = removeFiles(files,confirming); dropDraft(confirming);
    onChange(next); setSelected(firstReadable(next)); setConfirming(null);
  };
  // Memory is directory-shaped like the other pools: the same
  // FileWorkspace tree (multi-file, inline create/rename). The
  // read side aggregates every `*.md` of the saved version dir.
  const display = Object.fromEntries(Object.entries(files).map(([path,file]) => [path,textContent(file) ?? '二进制文件，请下载查看；如需替换，请用同名文件打包成压缩包后上传。']));
  return <div>
    {error && <Alert type="error" showIcon title={error}/>}
    {file && <Space wrap style={{marginBottom:12}}>
      <a download={selected.split('/').at(-1)} href={`data:application/octet-stream;base64,${file.content_b64}`}>下载</a>
      <Typography.Text type="secondary">{selected} · {atob(file.content_b64).length} 字节 · {text === null ? '二进制' : '文本'} · 权限 {(file.mode ?? 0o600).toString(8)}</Typography.Text>
      {cat === 'tools' && <Checkbox disabled={readOnly} checked={!!(file.mode & 0o111)}
        onChange={event => onChange({...files,[selected]:{...file,mode:event.target.checked ? file.mode | 0o100 : file.mode & ~0o111}})}>可执行</Checkbox>}
    </Space>}
    <FileWorkspace files={display} directories={directories} selected={selected} onSelect={setSelected}
      readOnly={readOnly || text === null} onChange={(path,value) => onChange(updateText(files,path,value))}
      onSave={onSave} operationsReadOnly={readOnly} onOperation={readOnly ? undefined : start}/>
    {!Object.keys(files).length && <Typography.Text type="secondary">{readOnly ? '未配置。' : `未配置，可在目录树右键新增文件，或用上方「上传压缩包」导入后保存。${cat === 'skills' ? '技能目录必须包含 SKILL.md。' : ''}`}</Typography.Text>}
    <Modal open={!!confirming} title="移除文件或目录" onCancel={() => setConfirming(null)} onOk={commit} okText="确认">
      <p>移除 {confirming} 及其内容？保存后生效。</p>
      {error && <Alert type="error" title={error}/>}
    </Modal>
  </div>;
}
